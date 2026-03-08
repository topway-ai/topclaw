use super::traits::{Tool, ToolResult};
use crate::agent::loop_::lossless::inspect_store;
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
use std::path::PathBuf;

pub struct LosslessDescribeTool {
    workspace_dir: PathBuf,
}

impl LosslessDescribeTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for LosslessDescribeTool {
    fn name(&self) -> &str {
        "lossless_describe"
    }

    fn description(&self) -> &str {
        "Describe persisted lossless conversation sessions, including scope, message counts, summary counts, and latest activity."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of sessions to return (default: 10)"
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        #[allow(clippy::cast_possible_truncation)]
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(10, |value| value as usize);
        let sessions = inspect_store(&self.workspace_dir, limit)?;
        if sessions.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No lossless context sessions found.".to_string(),
                error: None,
            });
        }

        let mut output = format!("Found {} lossless context sessions:\n", sessions.len());
        for session in sessions {
            let scope = session
                .session_scope
                .unwrap_or_else(|| "interactive".to_string());
            let key = session
                .session_key
                .unwrap_or_else(|| "<ephemeral>".to_string());
            let _ = writeln!(
                output,
                "- scope={scope} key={key} conversation={} messages={} summaries={} latest={}",
                session.conversation_id,
                session.message_count,
                session.summary_count,
                session.latest_activity_at
            );
        }

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}
