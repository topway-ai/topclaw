//! Approval-gated launcher for the built-in computer-use sidecar.
//!
//! On Approve, this tool spawns `<current_exe> computer-use-sidecar --bind ...`
//! as a detached child process and polls `GET /health` until the sidecar is
//! ready (or a short timeout elapses). This turns the "enable desktop
//! automation" flow into a single user tap in the Telegram channel.
//!
//! # Security
//!
//! - The tool is approval-gated: nothing spawns until the user taps Approve.
//! - The spawned binary is `std::env::current_exe()` — the same signed binary
//!   the agent is already running from. The tool does not resolve `PATH` or
//!   accept an executable path from the model.
//! - `bind` is validated as a `SocketAddr` before spawn.
//! - `api_key` is forwarded via the `TOPCLAW_SIDECAR_API_KEY` env var, never
//!   placed on argv where it would be visible in process listings.
//! - If a sidecar is already healthy at the endpoint, the tool returns success
//!   without spawning a duplicate.

use super::traits::{Tool, ToolResult};
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

const DEFAULT_BIND: &str = "127.0.0.1:8787";
const HEALTH_POLL_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct ComputerUseSidecarStartTool {
    security: Arc<SecurityPolicy>,
}

impl ComputerUseSidecarStartTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

fn extract_bind(args: &Value) -> Result<String, String> {
    let bind = args
        .get("bind")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_BIND)
        .trim()
        .to_string();
    bind.parse::<SocketAddr>()
        .map_err(|e| format!("invalid 'bind' address '{bind}': {e}"))?;
    Ok(bind)
}

async fn health_ok(endpoint: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    else {
        return false;
    };
    matches!(
        client.get(endpoint).send().await,
        Ok(resp) if resp.status().is_success()
    )
}

#[async_trait]
impl Tool for ComputerUseSidecarStartTool {
    fn name(&self) -> &str {
        "computer_use_sidecar_start"
    }

    fn description(&self) -> &str {
        "Spawn the built-in computer-use sidecar so the browser tool with \
         backend='computer_use' can reach a healthy endpoint. Requires user \
         approval. Idempotent: if a healthy sidecar is already running at the \
         configured endpoint, returns success without spawning another. \
         Linux-only at runtime."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "bind": {
                    "type": "string",
                    "description": "Address the sidecar binds to, e.g. '127.0.0.1:8787'. Must parse as SocketAddr.",
                    "default": DEFAULT_BIND
                },
                "api_key": {
                    "type": "string",
                    "description": "Optional Bearer key the sidecar will require on /v1/actions requests. Forwarded via env var, never on argv."
                },
                "reason": {
                    "type": "string",
                    "description": "Short human-readable reason, shown in the approval prompt."
                }
            }
        })
    }

    fn approval_precheck(&self, args: &Value) -> Result<(), String> {
        extract_bind(args)?;
        Ok(())
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

        let bind = match extract_bind(&args) {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e),
                });
            }
        };

        let health_url = format!("http://{bind}/health");
        if health_ok(&health_url).await {
            return Ok(ToolResult {
                success: true,
                output: format!("computer-use sidecar already healthy at http://{bind}/v1/actions"),
                error: None,
            });
        }

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("failed to resolve current executable: {e}")),
                });
            }
        };

        let mut cmd = tokio::process::Command::new(&exe);
        cmd.arg("computer-use-sidecar").arg("--bind").arg(&bind);
        if let Some(key) = args.get("api_key").and_then(Value::as_str) {
            if !key.is_empty() {
                cmd.env("TOPCLAW_SIDECAR_API_KEY", key);
            }
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        cmd.process_group(0); // detach from parent's process group

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("failed to spawn sidecar: {e}")),
                });
            }
        };
        let pid = child.id();

        // Poll /health until ready or timeout.
        let start = std::time::Instant::now();
        let mut healthy = false;
        while start.elapsed() < HEALTH_POLL_TIMEOUT {
            if health_ok(&health_url).await {
                healthy = true;
                break;
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }

        let reason = args.get("reason").and_then(Value::as_str);
        info!(
            target: "topclaw::audit",
            event = "computer_use_sidecar_start",
            bind = %bind,
            pid = ?pid,
            healthy,
            reason = ?reason,
            "computer-use sidecar spawn requested"
        );

        if healthy {
            // Detach a reaper so the Child handle can be dropped without
            // leaving a zombie. The sidecar continues running until killed.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(ToolResult {
                success: true,
                output: format!(
                    "computer-use sidecar started on http://{bind} (pid {pid:?}); /health is ready"
                ),
                error: None,
            })
        } else {
            // Sidecar never became healthy — kill it so we don't leave a
            // half-started process around.
            let _ = child.kill().await;
            let _ = child.wait().await;
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "spawned sidecar (pid {pid:?}) but /health at {health_url} did not become ready within {}s; check xdotool/wmctrl/scrot are installed and X session is available",
                    HEALTH_POLL_TIMEOUT.as_secs()
                )),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ComputerUseSidecarStartTool {
        ComputerUseSidecarStartTool::new(Arc::new(SecurityPolicy::default()))
    }

    #[test]
    fn precheck_rejects_invalid_bind() {
        let t = tool();
        assert!(t
            .approval_precheck(&json!({"bind": "not-an-address"}))
            .is_err());
    }

    #[test]
    fn precheck_accepts_default_and_explicit() {
        let t = tool();
        assert!(t.approval_precheck(&json!({})).is_ok());
        assert!(t
            .approval_precheck(&json!({"bind": "127.0.0.1:9999"}))
            .is_ok());
        assert!(t
            .approval_precheck(&json!({"bind": "0.0.0.0:8787"}))
            .is_ok());
    }

    #[test]
    fn schema_lists_expected_properties() {
        let t = tool();
        let schema = t.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("bind"));
        assert!(props.contains_key("api_key"));
        assert!(props.contains_key("reason"));
    }
}
