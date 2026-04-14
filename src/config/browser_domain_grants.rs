//! Runtime-grantable browser allowlist overlay.
//!
//! This module maintains a persistent, append-only overlay of browser domain
//! grants stored in `~/.topclaw/browser-allowed-domains-grants.json`. Grants
//! are unioned with the static `[browser].allowed_domains` entries from
//! `config.toml` to form the effective allowlist consulted by `browser_open`.
//!
//! The overlay file never mutates `config.toml`; the declarative config
//! remains the baseline while grants are a user-auditable record of approvals
//! made through the tool approval flow.
//!
//! Validation intentionally refuses wildcards, IPs, local-only hosts, and
//! single-label or punycode-unaware inputs so that an agent cannot widen
//! policy beyond a concrete, named public domain.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tools::url_validation::normalize_domain;

pub const GRANTS_FILENAME: &str = "browser-allowed-domains-grants.json";
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// A single domain grant, recorded at approval time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDomainGrant {
    pub domain: String,
    pub granted_at: DateTime<Utc>,
    #[serde(default)]
    pub granted_by_channel: Option<String>,
    #[serde(default)]
    pub granted_by_user: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedGrants {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    grants: Vec<BrowserDomainGrant>,
}

/// Validate a candidate grant domain.
///
/// Returns the normalized domain on success. The normalized form is what gets
/// persisted and matched by `host_matches_allowlist`.
///
/// Refuses: wildcards, empty input, IP literals, `localhost`/`.local`,
/// single-label hosts, labels containing anything other than ASCII LDH,
/// and labels starting or ending with `-`.
pub fn validate_grantable_domain(raw: &str) -> Result<String, String> {
    if raw.contains('*') || raw.contains('?') {
        return Err("wildcards are not allowed in grants".into());
    }
    let normalized =
        normalize_domain(raw).ok_or_else(|| "domain could not be normalized".to_string())?;

    if normalized.parse::<std::net::IpAddr>().is_ok() {
        return Err("IP literals are not allowed".into());
    }

    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld == "local")
    {
        return Err("local-only hosts cannot be granted".into());
    }

    let labels: Vec<&str> = normalized.split('.').collect();
    if labels.len() < 2 {
        return Err("domain must have at least two labels (e.g. 'example.com')".into());
    }

    let tld = *labels.last().expect("labels non-empty");
    if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err("top-level label must be at least two ASCII letters".into());
    }

    for label in &labels {
        if label.is_empty() {
            return Err("empty labels are not allowed".into());
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("labels may only contain ASCII letters, digits, and '-'".into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err("labels may not start or end with '-'".into());
        }
    }

    Ok(normalized)
}

/// Shared, mutable allowlist used by browser tools.
///
/// Holds the static `config.toml` entries unioned with runtime grants loaded
/// from (and persisted to) `~/.topclaw/browser-allowed-domains-grants.json`.
pub struct BrowserAllowlist {
    store_path: Option<PathBuf>,
    static_domains: Vec<String>,
    grants: RwLock<Vec<BrowserDomainGrant>>,
    effective: RwLock<Vec<String>>,
}

impl BrowserAllowlist {
    /// Load grants from `topclaw_dir` and build the effective allowlist.
    pub fn load(topclaw_dir: &Path, static_domains: Vec<String>) -> Result<Arc<Self>> {
        let store_path = topclaw_dir.join(GRANTS_FILENAME);
        let grants = read_grants(&store_path)?;
        let me = Arc::new(Self {
            store_path: Some(store_path),
            static_domains,
            grants: RwLock::new(grants),
            effective: RwLock::new(Vec::new()),
        });
        me.recompute_effective();
        Ok(me)
    }

    /// Build an in-memory-only allowlist with no grants file.
    ///
    /// Used when the grants file is unavailable or corrupt so that the
    /// browser tool remains functional with static config-only entries.
    pub fn in_memory(static_domains: Vec<String>) -> Arc<Self> {
        let me = Arc::new(Self {
            store_path: None,
            static_domains,
            grants: RwLock::new(Vec::new()),
            effective: RwLock::new(Vec::new()),
        });
        me.recompute_effective();
        me
    }

    /// Current effective allowlist (config ∪ grants, normalized, deduped).
    pub fn snapshot(&self) -> Vec<String> {
        self.effective.read().clone()
    }

    /// All runtime grants, in insertion order.
    pub fn grants_snapshot(&self) -> Vec<BrowserDomainGrant> {
        self.grants.read().clone()
    }

    /// Append a grant (validated, idempotent) and persist.
    pub async fn grant(
        &self,
        domain: &str,
        granted_by_channel: Option<String>,
        granted_by_user: Option<String>,
        reason: Option<String>,
    ) -> Result<BrowserDomainGrant> {
        let normalized = validate_grantable_domain(domain)
            .map_err(|e| anyhow::anyhow!("invalid domain: {e}"))?;

        {
            let guard = self.grants.read();
            if let Some(existing) = guard.iter().find(|g| g.domain == normalized) {
                return Ok(existing.clone());
            }
        }

        let entry = BrowserDomainGrant {
            domain: normalized.clone(),
            granted_at: Utc::now(),
            granted_by_channel,
            granted_by_user,
            reason,
        };

        {
            let mut guard = self.grants.write();
            if !guard.iter().any(|g| g.domain == normalized) {
                guard.push(entry.clone());
            }
        }

        if self.store_path.is_some() {
            self.persist().await?;
        }
        self.recompute_effective();
        Ok(entry)
    }

    fn recompute_effective(&self) {
        let grants = self.grants.read();
        let combined: Vec<String> = self
            .static_domains
            .iter()
            .cloned()
            .chain(grants.iter().map(|g| g.domain.clone()))
            .collect();
        let mut normalized = crate::tools::url_validation::normalize_allowed_domains(combined);
        normalized.sort_unstable();
        normalized.dedup();
        *self.effective.write() = normalized;
    }

    async fn persist(&self) -> Result<()> {
        let Some(store_path) = self.store_path.as_ref() else {
            return Ok(());
        };

        if let Some(parent) = store_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!(
                    "Failed to create browser grants directory at {}",
                    parent.display()
                )
            })?;
        }

        let snapshot = self.grants.read().clone();
        let persisted = PersistedGrants {
            schema_version: CURRENT_SCHEMA_VERSION,
            grants: snapshot,
        };
        let json =
            serde_json::to_vec_pretty(&persisted).context("Failed to serialize browser grants")?;
        let tmp = store_path.with_file_name(format!(
            "{}.tmp.{}.{}",
            GRANTS_FILENAME,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        tokio::fs::write(&tmp, &json).await.with_context(|| {
            format!(
                "Failed to write temporary browser grants file at {}",
                tmp.display()
            )
        })?;
        tokio::fs::rename(&tmp, store_path).await.with_context(|| {
            format!(
                "Failed to replace browser grants store at {}",
                store_path.display()
            )
        })?;
        Ok(())
    }
}

fn read_grants(path: &Path) -> Result<Vec<BrowserDomainGrant>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read browser grants store at {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let persisted: PersistedGrants = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse browser grants store at {}", path.display()))?;
    Ok(persisted.grants)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_wildcards() {
        assert!(validate_grantable_domain("*").is_err());
        assert!(validate_grantable_domain("*.example.com").is_err());
        assert!(validate_grantable_domain("exa?mple.com").is_err());
    }

    #[test]
    fn rejects_ip_literals() {
        assert!(validate_grantable_domain("127.0.0.1").is_err());
        assert!(validate_grantable_domain("8.8.8.8").is_err());
    }

    #[test]
    fn rejects_local_hosts() {
        assert!(validate_grantable_domain("localhost").is_err());
        assert!(validate_grantable_domain("foo.localhost").is_err());
        assert!(validate_grantable_domain("router.local").is_err());
    }

    #[test]
    fn rejects_single_label() {
        assert!(validate_grantable_domain("com").is_err());
        assert!(validate_grantable_domain("intranet").is_err());
    }

    #[test]
    fn rejects_empty_or_bad_labels() {
        assert!(validate_grantable_domain("").is_err());
        assert!(validate_grantable_domain("a..b").is_err());
        assert!(validate_grantable_domain("-bad.com").is_err());
        assert!(validate_grantable_domain("bad-.com").is_err());
        assert!(validate_grantable_domain("foo_bar.com").is_err());
    }

    #[test]
    fn rejects_numeric_tld() {
        assert!(validate_grantable_domain("example.123").is_err());
    }

    #[test]
    fn accepts_public_domains() {
        assert_eq!(validate_grantable_domain("baidu.com").unwrap(), "baidu.com");
        assert_eq!(
            validate_grantable_domain("API.Example.Com").unwrap(),
            "api.example.com"
        );
        assert_eq!(
            validate_grantable_domain("https://docs.rs/").unwrap(),
            "docs.rs"
        );
        assert_eq!(
            validate_grantable_domain("sub.example.co.uk").unwrap(),
            "sub.example.co.uk"
        );
    }

    #[test]
    fn accepts_punycode_labels() {
        // Pre-encoded punycode is allowed (LDH-valid); the matcher sees it as ASCII.
        assert!(validate_grantable_domain("xn--bcher-kva.example").is_ok());
    }

    #[tokio::test]
    async fn grant_persists_and_roundtrips() -> Result<()> {
        let tmp = TempDir::new()?;
        let allowlist = BrowserAllowlist::load(tmp.path(), vec!["google.com".into()])?;

        let entry = allowlist
            .grant(
                "baidu.com",
                Some("telegram".into()),
                Some("user123".into()),
                Some("opening search".into()),
            )
            .await?;
        assert_eq!(entry.domain, "baidu.com");

        let snap = allowlist.snapshot();
        assert!(snap.contains(&"baidu.com".to_string()));
        assert!(snap.contains(&"google.com".to_string()));

        let reloaded = BrowserAllowlist::load(tmp.path(), vec!["google.com".into()])?;
        let reloaded_snap = reloaded.snapshot();
        assert!(reloaded_snap.contains(&"baidu.com".to_string()));
        assert_eq!(reloaded.grants_snapshot().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn grant_is_idempotent() -> Result<()> {
        let tmp = TempDir::new()?;
        let allowlist = BrowserAllowlist::load(tmp.path(), vec![])?;

        let first = allowlist.grant("baidu.com", None, None, None).await?;
        let second = allowlist.grant("Baidu.COM", None, None, None).await?;

        assert_eq!(first.domain, second.domain);
        assert_eq!(first.granted_at, second.granted_at);
        assert_eq!(allowlist.grants_snapshot().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn grant_rejects_invalid_inputs() -> Result<()> {
        let tmp = TempDir::new()?;
        let allowlist = BrowserAllowlist::load(tmp.path(), vec![])?;

        assert!(allowlist.grant("*", None, None, None).await.is_err());
        assert!(allowlist
            .grant("localhost", None, None, None)
            .await
            .is_err());
        assert!(allowlist
            .grant("127.0.0.1", None, None, None)
            .await
            .is_err());
        assert!(allowlist.grant("com", None, None, None).await.is_err());
        assert!(allowlist.grants_snapshot().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn corrupt_store_returns_error_so_caller_can_fallback() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(GRANTS_FILENAME), b"{not valid json").unwrap();
        let err = match BrowserAllowlist::load(tmp.path(), vec![]) {
            Ok(_) => panic!("corrupt store must err"),
            Err(e) => e,
        };
        assert!(err.to_string().to_lowercase().contains("parse"));
    }

    #[test]
    fn in_memory_allowlist_has_no_persistence() {
        let allowlist = BrowserAllowlist::in_memory(vec!["google.com".into()]);
        assert_eq!(allowlist.snapshot(), vec!["google.com".to_string()]);
        assert!(allowlist.grants_snapshot().is_empty());
    }
}
