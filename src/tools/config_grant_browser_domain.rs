//! Tool that appends a single domain to the browser allowlist after explicit
//! user approval through the non-CLI approval flow.
//!
//! This tool cannot silently widen the allowlist: every invocation is a tool
//! call that the approval machinery intercepts and shows to the user with
//! its full argument set (domain, optional reason). On approval, the domain
//! is validated (refusing wildcards, IPs, local hosts, single-label inputs),
//! appended to the shared [`BrowserAllowlist`], and persisted to
//! `~/.topclaw/browser-allowed-domains-grants.json`.
//!
//! The grant takes effect immediately for subsequent `browser_open` calls in
//! the same process (and all future processes once persisted).

use super::traits::{Tool, ToolResult};
use crate::config::browser_domain_grants::validate_grantable_domain;
use crate::config::BrowserAllowlist;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tracing::info;

pub struct ConfigGrantBrowserDomainTool {
    allowlist: Arc<BrowserAllowlist>,
}

impl ConfigGrantBrowserDomainTool {
    pub fn new(allowlist: Arc<BrowserAllowlist>) -> Self {
        Self { allowlist }
    }
}

#[async_trait]
impl Tool for ConfigGrantBrowserDomainTool {
    fn name(&self) -> &str {
        "config_grant_browser_domain"
    }

    fn description(&self) -> &str {
        "Request user approval to add a single public domain to the browser allowlist. \
         Each call requires explicit approval; wildcards, IP literals, and local hosts are refused. \
         On approval, the domain is appended to ~/.topclaw/browser-allowed-domains-grants.json and \
         takes effect immediately for subsequent browser_open calls. Use this only when the user \
         has asked you to open a URL whose host is not yet in the allowlist."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Public domain to grant, e.g. 'baidu.com'. Subdomains are implicitly allowed by the existing matcher (api.baidu.com ⊂ baidu.com)."
                },
                "reason": {
                    "type": "string",
                    "description": "Short human-readable reason the grant is needed. Shown in the approval prompt and recorded in the grants file."
                }
            },
            "required": ["domain"]
        })
    }

    fn approval_precheck(&self, args: &serde_json::Value) -> Result<(), String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing 'domain' parameter".to_string())?;
        validate_grantable_domain(domain).map(|_| ())
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let domain = match args.get("domain").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("missing 'domain' parameter".into()),
                });
            }
        };
        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let channel = args
            .get("__approval_channel")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let user = args
            .get("__approval_user")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        match self
            .allowlist
            .grant(domain, channel.clone(), user.clone(), reason.clone())
            .await
        {
            Ok(entry) => {
                info!(
                    target: "topclaw::audit",
                    event = "browser_domain_grant",
                    domain = %entry.domain,
                    granted_at = %entry.granted_at,
                    channel = ?channel,
                    user = ?user,
                    reason = ?reason,
                    "browser domain granted"
                );
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Granted browser allowlist access for '{}'. Persisted to ~/.topclaw/browser-allowed-domains-grants.json.",
                        entry.domain
                    ),
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
    use tempfile::TempDir;

    fn tool_with_tempdir() -> (ConfigGrantBrowserDomainTool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let allowlist = BrowserAllowlist::load(tmp.path(), vec![]).unwrap();
        (ConfigGrantBrowserDomainTool::new(allowlist), tmp)
    }

    #[test]
    fn precheck_rejects_wildcards() {
        let (tool, _tmp) = tool_with_tempdir();
        let err = tool.approval_precheck(&json!({"domain": "*"})).unwrap_err();
        assert!(err.contains("wildcards"));
    }

    #[test]
    fn precheck_rejects_ip_literals() {
        let (tool, _tmp) = tool_with_tempdir();
        assert!(tool
            .approval_precheck(&json!({"domain": "8.8.8.8"}))
            .is_err());
    }

    #[test]
    fn precheck_accepts_public_domain() {
        let (tool, _tmp) = tool_with_tempdir();
        assert!(tool
            .approval_precheck(&json!({"domain": "baidu.com"}))
            .is_ok());
    }

    #[tokio::test]
    async fn execute_grants_and_persists() {
        let tmp = TempDir::new().unwrap();
        let allowlist = BrowserAllowlist::load(tmp.path(), vec![]).unwrap();
        let tool = ConfigGrantBrowserDomainTool::new(allowlist.clone());

        let result = tool
            .execute(json!({
                "domain": "baidu.com",
                "reason": "open search engine"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(allowlist.snapshot().contains(&"baidu.com".to_string()));

        let reloaded = BrowserAllowlist::load(tmp.path(), vec![]).unwrap();
        assert!(reloaded.snapshot().contains(&"baidu.com".to_string()));
    }

    #[tokio::test]
    async fn execute_returns_structured_error_on_invalid_domain() {
        let (tool, _tmp) = tool_with_tempdir();
        let result = tool.execute(json!({"domain": "localhost"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("local-only"));
    }
}
