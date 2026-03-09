use crate::config::Config;
use crate::security::{policy::ToolOperation, SecurityPolicy};
use crate::self_improvement::{self, SelfImprovementManager, SelfImprovementTaskStatus};
use crate::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

pub struct SelfImprovementTaskTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl SelfImprovementTaskTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }

    fn enforce_act(&self) -> Result<(), ToolResult> {
        self.security
            .enforce_tool_operation(ToolOperation::Act, "self_improvement_task")
            .map_err(|msg| ToolResult {
                success: false,
                output: String::new(),
                error: Some(msg),
            })
    }

    fn parse_status(raw: Option<&str>) -> Result<Option<SelfImprovementTaskStatus>, ToolResult> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        let status = match raw {
            "queued" => SelfImprovementTaskStatus::Queued,
            "in_progress" => SelfImprovementTaskStatus::InProgress,
            "blocked" => SelfImprovementTaskStatus::Blocked,
            "pr_opened" => SelfImprovementTaskStatus::PrOpened,
            "completed" => SelfImprovementTaskStatus::Completed,
            other => {
                return Err(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Invalid status '{other}'. Use queued, in_progress, blocked, pr_opened, or completed"
                    )),
                })
            }
        };
        Ok(Some(status))
    }
}

#[async_trait]
impl Tool for SelfImprovementTaskTool {
    fn name(&self) -> &str {
        "self_improvement_task"
    }

    fn description(&self) -> &str {
        "Queue and manage explicit TopClaw self-improvement tasks. Use only for concrete TopClaw bugs or product improvements explicitly requested or clearly confirmed by the user."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["enqueue", "list", "update", "readiness", "sync", "publish_pr"]
                },
                "title": {"type": "string"},
                "problem": {"type": "string"},
                "evidence": {"type": "string"},
                "requested_by": {"type": "string"},
                "requested_channel": {"type": "string"},
                "task_id": {"type": "string"},
                "status": {"type": "string"},
                "branch_name": {"type": "string"},
                "pr_url": {"type": "string"},
                "last_error": {"type": "string"},
                "validation_summary": {"type": "string"},
                "commit_message": {"type": "string"},
                "pr_title": {"type": "string"},
                "pr_body": {"type": "string"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let manager = SelfImprovementManager::new(&self.config.workspace_dir);

        let result: serde_json::Value = match action {
            "enqueue" => {
                if let Err(result) = self.enforce_act() {
                    return Ok(result);
                }
                let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
                let problem = args
                    .get("problem")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if title.is_empty() || problem.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("enqueue requires non-empty title and problem".into()),
                    });
                }
                let task = manager
                    .enqueue_task(
                        &self.config,
                        title,
                        problem,
                        args.get("evidence")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        args.get("requested_by")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        args.get("requested_channel")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    )
                    .await?;
                let sync = self_improvement::sync_scheduled_job(&self.config).await?;
                json!({"task": task, "sync": sync})
            }
            "list" => {
                let state = manager.load_state().await?;
                json!(state)
            }
            "update" => {
                if let Err(result) = self.enforce_act() {
                    return Ok(result);
                }
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("").trim();
                if task_id.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("update requires task_id".into()),
                    });
                }
                let status = match Self::parse_status(args.get("status").and_then(|v| v.as_str()))
                {
                    Ok(status) => status,
                    Err(result) => return Ok(result),
                };
                let task = manager
                    .update_task(
                        task_id,
                        status,
                        args.get("branch_name")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        args.get("pr_url")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        args.get("last_error")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        args.get("validation_summary")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    )
                    .await?;
                let sync = self_improvement::sync_scheduled_job(&self.config).await?;
                json!({"task": task, "sync": sync})
            }
            "readiness" => json!(self_improvement::check_git_readiness(&self.config).await),
            "sync" => {
                if let Err(result) = self.enforce_act() {
                    return Ok(result);
                }
                json!(self_improvement::sync_scheduled_job(&self.config).await?)
            }
            "publish_pr" => {
                if let Err(result) = self.enforce_act() {
                    return Ok(result);
                }
                let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or("").trim();
                let commit_message = args
                    .get("commit_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let pr_title = args.get("pr_title").and_then(|v| v.as_str()).unwrap_or("").trim();
                let pr_body = args.get("pr_body").and_then(|v| v.as_str()).unwrap_or("").trim();
                if task_id.is_empty()
                    || commit_message.is_empty()
                    || pr_title.is_empty()
                    || pr_body.is_empty()
                {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(
                            "publish_pr requires task_id, commit_message, pr_title, and pr_body"
                                .into(),
                        ),
                    });
                }
                let task = self_improvement::publish_draft_pr_for_task(
                    &self.config,
                    task_id,
                    commit_message,
                    pr_title,
                    pr_body,
                    args.get("validation_summary")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                )
                .await?;
                let sync = self_improvement::sync_scheduled_job(&self.config).await?;
                json!({"task": task, "sync": sync})
            }
            other => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Unknown action '{other}'. Use enqueue, list, update, readiness, sync, or publish_pr"
                    )),
                })
            }
        };

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result)?,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Arc<Config> {
        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        Arc::new(config)
    }

    #[test]
    fn parse_status_accepts_known_values() {
        assert_eq!(
            SelfImprovementTaskTool::parse_status(Some("queued")).unwrap(),
            Some(SelfImprovementTaskStatus::Queued)
        );
        assert!(SelfImprovementTaskTool::parse_status(Some("bad")).is_err());
    }

    #[tokio::test]
    async fn list_returns_valid_json_state() {
        let tmp = TempDir::new().unwrap();
        let tool =
            SelfImprovementTaskTool::new(test_config(&tmp), Arc::new(SecurityPolicy::default()));
        let result = tool.execute(json!({"action":"list"})).await.unwrap();
        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed.get("tasks").is_some());
    }
}
