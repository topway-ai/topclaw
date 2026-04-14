//! Generic config patcher: applies a value to a narrow, centrally-reviewed
//! allowlist of `config.toml` paths after explicit user approval.
//!
//! The tool is a thin wrapper around [`crate::config::patch`]. The security
//! boundary lives in the patch registry's closed match-list: any path not
//! enumerated there is rejected in [`approval_precheck`] before the approval
//! prompt is ever shown, so a malicious or confused model cannot use this tool
//! to rewrite arbitrary config sections.
//!
//! Each successful call writes `config.toml` atomically (temp-file + rename)
//! and preserves comments and formatting via `toml_edit`.

use super::traits::{Tool, ToolResult};
use crate::config::patch::{apply_patch, load_edit_save, PatchError, PATCHABLE_PATHS};
use crate::config::Config;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use toml_edit::DocumentMut;
use tracing::info;

pub struct ConfigPatchTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl ConfigPatchTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

fn extract_path_and_value(args: &Value) -> Result<(&str, &Value), String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing 'path' parameter (string)".to_string())?;
    let value = args
        .get("value")
        .ok_or_else(|| "missing 'value' parameter".to_string())?;
    Ok((path, value))
}

#[async_trait]
impl Tool for ConfigPatchTool {
    fn name(&self) -> &str {
        "config_patch"
    }

    fn description(&self) -> &str {
        "Apply a value to a centrally-reviewed config.toml path. Requires user approval. \
         Only a narrow allowlist of paths is accepted; unknown paths are rejected before \
         approval is requested. On success, config.toml is written atomically with comments \
         preserved. Use this instead of asking the user to hand-edit config.toml."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Dotted config path. Must be one of the patchable paths.",
                    "enum": PATCHABLE_PATHS,
                },
                "value": {
                    "description": "New value. Type depends on the path (string, array, etc.)."
                },
                "reason": {
                    "type": "string",
                    "description": "Short human-readable reason, shown in the approval prompt."
                }
            },
            "required": ["path", "value"]
        })
    }

    fn approval_precheck(&self, args: &Value) -> Result<(), String> {
        let (path, value) = extract_path_and_value(args)?;
        if !PATCHABLE_PATHS.contains(&path) {
            return Err(format!(
                "path '{path}' is not in the patchable-paths allowlist: {PATCHABLE_PATHS:?}"
            ));
        }
        // Dry-run the patch against an empty document so invalid values fail
        // before the user is asked to approve them. The real patch runs against
        // the live document in execute().
        let mut probe = DocumentMut::new();
        match apply_patch(&mut probe, path, value) {
            Ok(_) => Ok(()),
            Err(PatchError::UnknownPath { path }) => Err(format!("unknown patchable path: {path}")),
            Err(PatchError::InvalidValue { path, reason }) => {
                Err(format!("invalid value for {path}: {reason}"))
            }
            Err(PatchError::ParseError(e)) => Err(e),
        }
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }
        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        let (path, value) = match extract_path_and_value(&args) {
            Ok(pv) => pv,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e),
                });
            }
        };

        let reason = args.get("reason").and_then(Value::as_str);
        let channel = args.get("__approval_channel").and_then(Value::as_str);
        let user = args.get("__approval_user").and_then(Value::as_str);

        match load_edit_save(&self.config.config_path, path, value).await {
            Ok(outcome) => {
                info!(
                    target: "topclaw::audit",
                    event = "config_patch",
                    path = %outcome.path,
                    changed = outcome.changed,
                    channel = ?channel,
                    user = ?user,
                    reason = ?reason,
                    "config patched"
                );
                Ok(ToolResult {
                    success: true,
                    output: outcome.summary,
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tempfile::TempDir;

    fn tool_with_tempdir() -> (ConfigPatchTool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "").unwrap();
        let mut config = Config::default();
        config.config_path = config_path;
        let security = Arc::new(SecurityPolicy {
            autonomy: crate::security::policy::AutonomyLevel::Supervised,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        (ConfigPatchTool::new(Arc::new(config), security), tmp)
    }

    #[test]
    fn precheck_rejects_unknown_path() {
        let (tool, _tmp) = tool_with_tempdir();
        let err = tool
            .approval_precheck(&json!({
                "path": "security.allow_everything",
                "value": true
            }))
            .unwrap_err();
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn precheck_rejects_missing_fields() {
        let (tool, _tmp) = tool_with_tempdir();
        assert!(tool.approval_precheck(&json!({})).is_err());
        assert!(tool
            .approval_precheck(&json!({"path": "browser.backend"}))
            .is_err());
    }

    #[test]
    fn precheck_rejects_invalid_backend_before_approval() {
        let (tool, _tmp) = tool_with_tempdir();
        let err = tool
            .approval_precheck(&json!({
                "path": "browser.backend",
                "value": "not-a-backend"
            }))
            .unwrap_err();
        assert!(err.to_lowercase().contains("must be one of"));
    }

    #[test]
    fn precheck_accepts_valid_backend() {
        let (tool, _tmp) = tool_with_tempdir();
        assert!(tool
            .approval_precheck(&json!({
                "path": "browser.backend",
                "value": "computer_use"
            }))
            .is_ok());
    }

    #[tokio::test]
    async fn execute_writes_backend_and_persists() {
        let (tool, tmp) = tool_with_tempdir();
        let result = tool
            .execute(json!({
                "path": "browser.backend",
                "value": "computer_use",
                "reason": "user asked to enable desktop computer use"
            }))
            .await
            .unwrap();
        assert!(result.success, "error = {:?}", result.error);

        let contents = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(contents.contains("backend = \"computer_use\""));
    }

    #[tokio::test]
    async fn execute_returns_structured_error_on_invalid_value() {
        let (tool, _tmp) = tool_with_tempdir();
        let result = tool
            .execute(json!({
                "path": "browser.backend",
                "value": "totally-bogus"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .unwrap()
            .to_lowercase()
            .contains("must be one of"));
    }
}
