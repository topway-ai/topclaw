use super::traits::{Tool, ToolResult};
use crate::agent::loop_::lossless::search_store;
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
use std::path::PathBuf;

pub struct LosslessSearchTool {
    workspace_dir: PathBuf,
}

impl LosslessSearchTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for LosslessSearchTool {
    fn name(&self) -> &str {
        "lossless_search"
    }

    fn description(&self) -> &str {
        "Search preserved lossless conversation messages and summaries by keyword across interactive, channel, and gateway sessions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword or phrase to search for in preserved context"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of hits to return (default: 10)"
                },
                "session_scope": {
                    "type": "string",
                    "description": "Optional session scope filter, such as interactive, channel, or gateway_ws"
                },
                "session_key": {
                    "type": "string",
                    "description": "Optional exact session key filter"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?;
        #[allow(clippy::cast_possible_truncation)]
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(10, |value| value as usize);
        let session_scope = args
            .get("session_scope")
            .and_then(serde_json::Value::as_str);
        let session_key = args.get("session_key").and_then(serde_json::Value::as_str);
        let hits = search_store(
            &self.workspace_dir,
            query,
            limit,
            session_scope,
            session_key,
        )?;

        if hits.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No preserved lossless context matched that query.".to_string(),
                error: None,
            });
        }

        let mut output = format!("Found {} lossless context hits:\n", hits.len());
        for hit in hits {
            let scope = hit
                .session_scope
                .unwrap_or_else(|| "interactive".to_string());
            let key = hit.session_key.unwrap_or_else(|| "<ephemeral>".to_string());
            let role = hit.role.unwrap_or_else(|| hit.source_kind.clone());
            let _ = writeln!(
                output,
                "- scope={scope} key={key} conversation={} source={} ord={} role={} excerpt={}",
                hit.conversation_id,
                hit.source_kind,
                hit.ordinal_hint,
                role,
                hit.excerpt.replace('\n', " ")
            );
        }

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}
