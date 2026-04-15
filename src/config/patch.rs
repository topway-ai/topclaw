//! Centralized registry of user-approved config.toml mutations.
//!
//! This module is the **single** place where the agent may write back to the
//! user's `config.toml`. Every writable key is listed in [`apply_patch`]; a
//! reviewer auditing TopClaw's config-mutation surface reads one file.
//!
//! Adding a patchable path is a security decision. New match arms belong here
//! and must pass code review — do not push validation logic down into a trait
//! or registry elsewhere.
//!
//! # Contract
//!
//! [`apply_patch`] is pure in-memory (takes a `toml_edit::DocumentMut`, edits
//! it, returns a [`PatchOutcome`]). [`load_edit_save`] wraps it with atomic
//! read-modify-write on disk using temp-file + rename, matching the pattern
//! in `src/auth/profiles.rs`.
//!
//! # Audit & reversibility
//!
//! Every successful patch returns a human-readable summary that callers log
//! to the audit trail. `config.toml` comments and ordering are preserved by
//! `toml_edit`, so users can still read their file after an agent-initiated
//! edit and revert manually if desired.

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};
use toml_edit::{value as toml_value, Array, DocumentMut, Item};

/// Canonical list of patchable paths, in declaration order.
///
/// Used for human-facing error messages that enumerate options when the
/// caller supplies an unknown path. Keep in sync with [`apply_patch`].
pub const PATCHABLE_PATHS: &[&str] = &[
    "agent.max_tool_iterations",
    "browser.enabled",
    "browser.backend",
    "browser.computer_use.window_allowlist",
];

/// Outcome of a successful patch, suitable for surfacing to the user.
#[derive(Debug, Clone)]
pub struct PatchOutcome {
    /// Canonical path that was written, e.g. `"browser.backend"`.
    pub path: String,
    /// Short human-readable summary of what changed. Rendered into the
    /// approval prompt and the audit log.
    pub summary: String,
    /// Whether the patch changed the document. Idempotent no-op calls
    /// return `changed = false` and the caller can skip persistence.
    pub changed: bool,
}

/// Structured failure describing why a patch was refused.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error(
        "path '{path}' is not patchable. Patchable paths: {}",
        PATCHABLE_PATHS.join(", ")
    )]
    UnknownPath { path: String },
    #[error("invalid value for '{path}': {reason}")]
    InvalidValue { path: String, reason: String },
    #[error("failed to parse config.toml: {0}")]
    ParseError(String),
}

/// Apply a patch to an in-memory TOML document.
///
/// **This is the security boundary.** Every writable path lives in the match
/// below. Unknown paths return [`PatchError::UnknownPath`]; the agent cannot
/// invent new paths.
pub fn apply_patch(
    doc: &mut DocumentMut,
    path: &str,
    input: &JsonValue,
) -> Result<PatchOutcome, PatchError> {
    match path {
        "agent.max_tool_iterations" => patch_agent_max_tool_iterations(doc, input),
        "browser.enabled" => patch_browser_enabled(doc, input),
        "browser.backend" => patch_browser_backend(doc, input),
        "browser.computer_use.window_allowlist" => patch_window_allowlist(doc, input),
        _ => Err(PatchError::UnknownPath {
            path: path.to_string(),
        }),
    }
}

/// Read `config.toml`, apply a patch, and atomically persist on change.
///
/// On success returns [`PatchOutcome`]. Returns [`PatchError::ParseError`] if
/// the file is not valid TOML — the agent should surface this rather than
/// attempt a recovery write that could mangle the user's config.
pub async fn load_edit_save(
    config_path: &Path,
    patch_path: &str,
    value: &JsonValue,
) -> Result<PatchOutcome> {
    let raw = tokio::fs::read_to_string(config_path)
        .await
        .with_context(|| format!("Failed to read config at {}", config_path.display()))?;

    let mut doc: DocumentMut = raw
        .parse()
        .map_err(|e: toml_edit::TomlError| PatchError::ParseError(e.to_string()))?;

    let outcome = apply_patch(&mut doc, patch_path, value)?;

    if outcome.changed {
        persist_atomic(config_path, &doc.to_string()).await?;
    }

    Ok(outcome)
}

async fn persist_atomic(config_path: &Path, serialized: &str) -> Result<()> {
    let parent = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config path has no parent directory"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("Failed to create config directory at {}", parent.display()))?;
    let file_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.toml");
    let tmp = tmp_path(parent, file_name);
    tokio::fs::write(&tmp, serialized)
        .await
        .with_context(|| format!("Failed to write temp config at {}", tmp.display()))?;
    tokio::fs::rename(&tmp, config_path)
        .await
        .with_context(|| format!("Failed to replace config at {}", config_path.display()))?;
    Ok(())
}

fn tmp_path(parent: &Path, file_name: &str) -> PathBuf {
    parent.join(format!(
        "{}.tmp.{}.{}",
        file_name,
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

// ── patchable paths ────────────────────────────────────────────────────────

/// Upper bound on `agent.max_tool_iterations`. Chosen high enough for deep
/// research / skill-building sessions but low enough to bound a single turn's
/// worst-case tool spend. Bumping beyond 1000 should be a deliberate code
/// change, not an agent-initiated patch.
const AGENT_MAX_TOOL_ITERATIONS_CEILING: i64 = 1000;

fn patch_agent_max_tool_iterations(
    doc: &mut DocumentMut,
    input: &JsonValue,
) -> Result<PatchOutcome, PatchError> {
    let n = input.as_i64().ok_or_else(|| PatchError::InvalidValue {
        path: "agent.max_tool_iterations".into(),
        reason: "must be an integer".into(),
    })?;
    if n < 1 {
        return Err(PatchError::InvalidValue {
            path: "agent.max_tool_iterations".into(),
            reason: format!("must be >= 1, got {n}"),
        });
    }
    if n > AGENT_MAX_TOOL_ITERATIONS_CEILING {
        return Err(PatchError::InvalidValue {
            path: "agent.max_tool_iterations".into(),
            reason: format!("must be <= {AGENT_MAX_TOOL_ITERATIONS_CEILING}, got {n}"),
        });
    }

    let current = doc
        .get("agent")
        .and_then(Item::as_table)
        .and_then(|t| t.get("max_tool_iterations"))
        .and_then(Item::as_integer);
    if current == Some(n) {
        return Ok(PatchOutcome {
            path: "agent.max_tool_iterations".into(),
            summary: format!("agent.max_tool_iterations already = {n} (no change)"),
            changed: false,
        });
    }

    let agent = doc
        .entry("agent")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| PatchError::InvalidValue {
            path: "agent.max_tool_iterations".into(),
            reason: "[agent] is not a table in config.toml".into(),
        })?;
    agent["max_tool_iterations"] = toml_value(n);

    let summary = match current {
        Some(prev) => format!("Set agent.max_tool_iterations = {n} (was {prev})"),
        None => format!("Set agent.max_tool_iterations = {n} (was default)"),
    };
    Ok(PatchOutcome {
        path: "agent.max_tool_iterations".into(),
        summary,
        changed: true,
    })
}

fn patch_browser_enabled(
    doc: &mut DocumentMut,
    input: &JsonValue,
) -> Result<PatchOutcome, PatchError> {
    let new_value = input.as_bool().ok_or_else(|| PatchError::InvalidValue {
        path: "browser.enabled".into(),
        reason: "must be a boolean".into(),
    })?;

    let current = doc
        .get("browser")
        .and_then(Item::as_table)
        .and_then(|t| t.get("enabled"))
        .and_then(Item::as_bool)
        .unwrap_or(false);
    if current == new_value {
        return Ok(PatchOutcome {
            path: "browser.enabled".into(),
            summary: format!("browser.enabled already = {new_value} (no change)"),
            changed: false,
        });
    }

    let browser = doc
        .entry("browser")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| PatchError::InvalidValue {
            path: "browser.enabled".into(),
            reason: "[browser] is not a table in config.toml".into(),
        })?;
    browser["enabled"] = toml_value(new_value);

    Ok(PatchOutcome {
        path: "browser.enabled".into(),
        summary: format!("Set browser.enabled = {new_value} (was {current})"),
        changed: true,
    })
}

const BROWSER_BACKEND_ALLOWED: &[&str] = &["auto", "computer_use", "rust_native", "agent_browser"];

fn patch_browser_backend(
    doc: &mut DocumentMut,
    input: &JsonValue,
) -> Result<PatchOutcome, PatchError> {
    let new_value = input.as_str().ok_or_else(|| PatchError::InvalidValue {
        path: "browser.backend".into(),
        reason: "must be a string".into(),
    })?;
    if !BROWSER_BACKEND_ALLOWED.contains(&new_value) {
        return Err(PatchError::InvalidValue {
            path: "browser.backend".into(),
            reason: format!(
                "must be one of {:?}, got {:?}",
                BROWSER_BACKEND_ALLOWED, new_value
            ),
        });
    }

    let current = doc
        .get("browser")
        .and_then(Item::as_table)
        .and_then(|t| t.get("backend"))
        .and_then(Item::as_str)
        .unwrap_or("")
        .to_string();
    if current == new_value {
        return Ok(PatchOutcome {
            path: "browser.backend".into(),
            summary: format!("browser.backend already = \"{new_value}\" (no change)"),
            changed: false,
        });
    }

    let browser = doc
        .entry("browser")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| PatchError::InvalidValue {
            path: "browser.backend".into(),
            reason: "[browser] is not a table in config.toml".into(),
        })?;
    browser["backend"] = toml_value(new_value);

    Ok(PatchOutcome {
        path: "browser.backend".into(),
        summary: format!("Set browser.backend = \"{new_value}\" (was \"{current}\")"),
        changed: true,
    })
}

const WINDOW_ALLOWLIST_MAX_LEN: usize = 32;
const WINDOW_TITLE_MAX_CHARS: usize = 128;

fn patch_window_allowlist(
    doc: &mut DocumentMut,
    input: &JsonValue,
) -> Result<PatchOutcome, PatchError> {
    let arr = input.as_array().ok_or_else(|| PatchError::InvalidValue {
        path: "browser.computer_use.window_allowlist".into(),
        reason: "must be an array of strings".into(),
    })?;
    if arr.len() > WINDOW_ALLOWLIST_MAX_LEN {
        return Err(PatchError::InvalidValue {
            path: "browser.computer_use.window_allowlist".into(),
            reason: format!(
                "at most {WINDOW_ALLOWLIST_MAX_LEN} entries allowed, got {}",
                arr.len()
            ),
        });
    }
    let mut normalized: Vec<String> = Vec::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        let s = entry.as_str().ok_or_else(|| PatchError::InvalidValue {
            path: "browser.computer_use.window_allowlist".into(),
            reason: format!("entry {i} is not a string"),
        })?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(PatchError::InvalidValue {
                path: "browser.computer_use.window_allowlist".into(),
                reason: format!("entry {i} is empty or whitespace"),
            });
        }
        if trimmed.chars().count() > WINDOW_TITLE_MAX_CHARS {
            return Err(PatchError::InvalidValue {
                path: "browser.computer_use.window_allowlist".into(),
                reason: format!("entry {i} exceeds {WINDOW_TITLE_MAX_CHARS} chars"),
            });
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(PatchError::InvalidValue {
                path: "browser.computer_use.window_allowlist".into(),
                reason: format!("entry {i} contains control characters"),
            });
        }
        normalized.push(trimmed.to_string());
    }
    normalized.sort();
    normalized.dedup();

    let current: Vec<String> = doc
        .get("browser")
        .and_then(Item::as_table)
        .and_then(|t| t.get("computer_use"))
        .and_then(Item::as_table)
        .and_then(|t| t.get("window_allowlist"))
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let mut current_sorted = current.clone();
    current_sorted.sort();
    current_sorted.dedup();

    if current_sorted == normalized {
        return Ok(PatchOutcome {
            path: "browser.computer_use.window_allowlist".into(),
            summary: "browser.computer_use.window_allowlist unchanged (no change)".into(),
            changed: false,
        });
    }

    let browser = doc
        .entry("browser")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| PatchError::InvalidValue {
            path: "browser.computer_use.window_allowlist".into(),
            reason: "[browser] is not a table in config.toml".into(),
        })?;
    let computer_use = browser
        .entry("computer_use")
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| PatchError::InvalidValue {
            path: "browser.computer_use.window_allowlist".into(),
            reason: "[browser.computer_use] is not a table in config.toml".into(),
        })?;

    let mut array = Array::new();
    for entry in &normalized {
        array.push(entry.as_str());
    }
    computer_use["window_allowlist"] = Item::Value(toml_edit::Value::Array(array));

    let summary = if current.is_empty() {
        format!(
            "Set browser.computer_use.window_allowlist = {:?} (was empty)",
            normalized
        )
    } else {
        format!(
            "Set browser.computer_use.window_allowlist = {:?} (was {:?})",
            normalized, current
        )
    };
    Ok(PatchOutcome {
        path: "browser.computer_use.window_allowlist".into(),
        summary,
        changed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn parse(raw: &str) -> DocumentMut {
        raw.parse().unwrap()
    }

    #[test]
    fn unknown_path_rejected_and_enumerates() {
        let mut doc = parse("");
        let err = apply_patch(&mut doc, "autonomy.allowed_commands", &json!(["rm"]))
            .expect_err("must reject");
        let msg = err.to_string();
        assert!(msg.contains("autonomy.allowed_commands"));
        assert!(msg.contains("agent.max_tool_iterations"));
        assert!(msg.contains("browser.enabled"));
        assert!(msg.contains("browser.backend"));
        assert!(msg.contains("browser.computer_use.window_allowlist"));
    }

    #[test]
    fn agent_max_tool_iterations_rejects_non_integer() {
        let mut doc = parse("");
        assert!(apply_patch(&mut doc, "agent.max_tool_iterations", &json!("100")).is_err());
        assert!(apply_patch(&mut doc, "agent.max_tool_iterations", &json!(1.5)).is_err());
    }

    #[test]
    fn agent_max_tool_iterations_rejects_out_of_range() {
        let mut doc = parse("");
        assert!(apply_patch(&mut doc, "agent.max_tool_iterations", &json!(0)).is_err());
        assert!(apply_patch(&mut doc, "agent.max_tool_iterations", &json!(-5)).is_err());
        assert!(apply_patch(&mut doc, "agent.max_tool_iterations", &json!(10_000)).is_err());
    }

    #[test]
    fn agent_max_tool_iterations_applies_and_creates_section() {
        let mut doc = parse("");
        let outcome = apply_patch(&mut doc, "agent.max_tool_iterations", &json!(100)).unwrap();
        assert!(outcome.changed);
        assert!(doc.to_string().contains("max_tool_iterations = 100"));
    }

    #[test]
    fn agent_max_tool_iterations_preserves_comments() {
        let raw = r#"# top comment
[agent]
# iteration cap
max_tool_iterations = 20
max_history_messages = 50
"#;
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "agent.max_tool_iterations", &json!(100)).unwrap();
        assert!(outcome.changed);
        let rendered = doc.to_string();
        assert!(rendered.contains("# top comment"));
        assert!(rendered.contains("# iteration cap"));
        assert!(rendered.contains("max_tool_iterations = 100"));
    }

    #[test]
    fn agent_max_tool_iterations_idempotent_when_same() {
        let raw = "[agent]\nmax_tool_iterations = 50\n";
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "agent.max_tool_iterations", &json!(50)).unwrap();
        assert!(!outcome.changed);
    }

    #[test]
    fn browser_enabled_rejects_non_bool() {
        let mut doc = parse("");
        assert!(apply_patch(&mut doc, "browser.enabled", &json!("true")).is_err());
        assert!(apply_patch(&mut doc, "browser.enabled", &json!(1)).is_err());
    }

    #[test]
    fn browser_enabled_applies_and_creates_section() {
        let mut doc = parse("");
        let outcome = apply_patch(&mut doc, "browser.enabled", &json!(true)).unwrap();
        assert!(outcome.changed);
        assert!(doc.to_string().contains("enabled = true"));
    }

    #[test]
    fn browser_enabled_preserves_comments() {
        let raw = r#"# top comment
[browser]
# gate
enabled = false
backend = "auto"
"#;
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "browser.enabled", &json!(true)).unwrap();
        assert!(outcome.changed);
        let rendered = doc.to_string();
        assert!(rendered.contains("# top comment"));
        assert!(rendered.contains("# gate"));
        assert!(rendered.contains("enabled = true"));
    }

    #[test]
    fn browser_enabled_idempotent_when_same() {
        let raw = "[browser]\nenabled = true\n";
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "browser.enabled", &json!(true)).unwrap();
        assert!(!outcome.changed);
    }

    #[test]
    fn browser_backend_rejects_non_string() {
        let mut doc = parse("");
        assert!(apply_patch(&mut doc, "browser.backend", &json!(42)).is_err());
    }

    #[test]
    fn browser_backend_rejects_unknown_enum() {
        let mut doc = parse("");
        let err = apply_patch(&mut doc, "browser.backend", &json!("hacker")).unwrap_err();
        assert!(err.to_string().contains("must be one of"));
    }

    #[test]
    fn browser_backend_applies_and_preserves_comments() {
        let raw = r#"# top comment
[browser]
# which backend to use
backend = "auto"
allowed_domains = []
"#;
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "browser.backend", &json!("computer_use")).unwrap();
        assert!(outcome.changed);
        let rendered = doc.to_string();
        assert!(rendered.contains("# top comment"));
        assert!(rendered.contains("# which backend to use"));
        assert!(rendered.contains("backend = \"computer_use\""));
    }

    #[test]
    fn browser_backend_idempotent_when_same() {
        let raw = "[browser]\nbackend = \"computer_use\"\n";
        let mut doc = parse(raw);
        let outcome = apply_patch(&mut doc, "browser.backend", &json!("computer_use")).unwrap();
        assert!(!outcome.changed);
    }

    #[test]
    fn browser_backend_creates_missing_section() {
        let mut doc = parse("");
        let outcome = apply_patch(&mut doc, "browser.backend", &json!("rust_native")).unwrap();
        assert!(outcome.changed);
        assert!(doc.to_string().contains("backend = \"rust_native\""));
    }

    #[test]
    fn window_allowlist_rejects_non_array() {
        let mut doc = parse("");
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!("chrome")
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_rejects_non_string_entry() {
        let mut doc = parse("");
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(["ok", 5])
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_rejects_empty_or_whitespace() {
        let mut doc = parse("");
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!([""])
        )
        .is_err());
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(["   "])
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_rejects_control_chars() {
        let mut doc = parse("");
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(["bad\u{0007}title"])
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_rejects_too_long_entry() {
        let mut doc = parse("");
        let long: String = "a".repeat(129);
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!([long])
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_rejects_too_many_entries() {
        let mut doc = parse("");
        let many: Vec<String> = (0..40).map(|i| format!("app-{i}")).collect();
        assert!(apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(many)
        )
        .is_err());
    }

    #[test]
    fn window_allowlist_sorts_and_dedupes() {
        let mut doc = parse("");
        let outcome = apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(["Firefox", "Chrome", "Firefox"]),
        )
        .unwrap();
        assert!(outcome.changed);
        let rendered = doc.to_string();
        // Sorted: Chrome first
        let chrome_pos = rendered.find("Chrome").unwrap();
        let firefox_pos = rendered.find("Firefox").unwrap();
        assert!(chrome_pos < firefox_pos);
        // Only one Firefox
        assert_eq!(rendered.matches("Firefox").count(), 1);
    }

    #[test]
    fn window_allowlist_idempotent_semantic() {
        let raw = r#"[browser.computer_use]
window_allowlist = ["Firefox", "Chrome"]
"#;
        let mut doc = parse(raw);
        let outcome = apply_patch(
            &mut doc,
            "browser.computer_use.window_allowlist",
            &json!(["Chrome", "Firefox"]),
        )
        .unwrap();
        assert!(!outcome.changed);
    }

    #[tokio::test]
    async fn load_edit_save_roundtrips_and_persists() -> Result<()> {
        let tmp = TempDir::new()?;
        let cfg = tmp.path().join("config.toml");
        let original = r#"# my comment
[browser]
backend = "auto"
"#;
        tokio::fs::write(&cfg, original).await?;

        let outcome = load_edit_save(&cfg, "browser.backend", &json!("computer_use")).await?;
        assert!(outcome.changed);

        let after = tokio::fs::read_to_string(&cfg).await?;
        assert!(after.contains("backend = \"computer_use\""));
        assert!(after.contains("# my comment"));
        Ok(())
    }

    #[tokio::test]
    async fn load_edit_save_on_invalid_path_does_not_touch_file() -> Result<()> {
        let tmp = TempDir::new()?;
        let cfg = tmp.path().join("config.toml");
        let original = "[browser]\nbackend = \"auto\"\n";
        tokio::fs::write(&cfg, original).await?;

        let err = load_edit_save(&cfg, "not.a.path", &json!("x")).await;
        assert!(err.is_err());

        let after = tokio::fs::read_to_string(&cfg).await?;
        assert_eq!(after, original);
        Ok(())
    }

    #[tokio::test]
    async fn load_edit_save_reports_parse_errors() -> Result<()> {
        let tmp = TempDir::new()?;
        let cfg = tmp.path().join("config.toml");
        tokio::fs::write(&cfg, "this is not [valid toml").await?;
        let err = load_edit_save(&cfg, "browser.backend", &json!("auto"))
            .await
            .expect_err("must error on parse");
        assert!(err.to_string().to_lowercase().contains("parse"));
        Ok(())
    }

    #[tokio::test]
    async fn load_edit_save_idempotent_skips_write() -> Result<()> {
        let tmp = TempDir::new()?;
        let cfg = tmp.path().join("config.toml");
        let original = "[browser]\nbackend = \"computer_use\"\n";
        tokio::fs::write(&cfg, original).await?;
        let before = tokio::fs::metadata(&cfg).await?.modified()?;

        // Sleep just enough to ensure a write would change mtime.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let outcome = load_edit_save(&cfg, "browser.backend", &json!("computer_use")).await?;
        assert!(!outcome.changed);

        let after = tokio::fs::metadata(&cfg).await?.modified()?;
        assert_eq!(before, after, "no-op patch must not rewrite the file");
        Ok(())
    }
}
