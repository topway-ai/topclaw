use super::parsing::ParsedToolCall;
use super::{scrub_credentials, ToolLoopCancelled};
use crate::approval::{ApprovalManager, NonCliTurnApprovalGrant};
use crate::observability::{Observer, ObserverEvent};
use crate::tools::Tool;
use anyhow::Result;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

fn find_tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> Option<&'a dyn Tool> {
    tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
}

pub(super) fn blocked_non_cli_approval_plan_reason(
    tool_calls: &[ParsedToolCall],
    tools_registry: &[Box<dyn Tool>],
) -> Option<String> {
    for call in tool_calls {
        let Some(tool) = find_tool(tools_registry, &call.name) else {
            continue;
        };
        if let Err(reason) = tool.approval_precheck(&call.arguments) {
            let reason = scrub_credentials(&reason);
            return Some(format!(
                "Approval not requested because the current execution plan includes `{}` that runtime policy would still reject: {}",
                call.name, reason
            ));
        }
    }

    None
}

fn tool_uses_shell_command(call_name: &str, args: &Value) -> bool {
    match call_name {
        "shell" => true,
        "process" => matches!(
            args.get("action").and_then(serde_json::Value::as_str),
            Some("spawn_shell")
        ),
        _ => false,
    }
}

fn apply_turn_grant_to_tool_args(
    call_name: &str,
    mut call_arguments: Value,
    turn_grant: Option<&NonCliTurnApprovalGrant>,
) -> Value {
    let Some(grant) = turn_grant else {
        return call_arguments;
    };
    let uses_shell_command = tool_uses_shell_command(call_name, &call_arguments);
    let Some(args_obj) = call_arguments.as_object_mut() else {
        return call_arguments;
    };

    args_obj.insert("approved".to_string(), Value::Bool(true));

    if uses_shell_command && !grant.approved_shell_commands.is_empty() {
        args_obj.insert(
            crate::tools::shell::APPROVED_PLAN_SHELL_COMMANDS_ARG.to_string(),
            Value::Array(
                grant
                    .approved_shell_commands
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }

    call_arguments
}

/// Strip internal approval fields that only the turn-grant mechanism may set.
/// Without this, a model could self-approve by emitting `"approved": true` or
/// `"__approved_plan_shell_commands": [...]` in its raw tool-call arguments.
fn strip_internal_approval_fields(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        obj.remove("approved");
        obj.remove(crate::tools::shell::APPROVED_PLAN_SHELL_COMMANDS_ARG);
    }
    args
}

async fn execute_one_tool(
    call_name: &str,
    call_arguments: serde_json::Value,
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    cancellation_token: Option<&CancellationToken>,
    turn_grant: Option<&NonCliTurnApprovalGrant>,
) -> Result<ToolExecutionOutcome> {
    observer.record_event(&ObserverEvent::ToolCallStart {
        tool: call_name.to_string(),
    });
    let start = Instant::now();

    let Some(tool) = find_tool(tools_registry, call_name) else {
        let reason = format!("Unknown tool: {call_name}");
        let duration = start.elapsed();
        observer.record_event(&ObserverEvent::ToolCall {
            tool: call_name.to_string(),
            duration,
            success: false,
        });
        return Ok(ToolExecutionOutcome {
            output: reason.clone(),
            success: false,
            error_reason: Some(scrub_credentials(&reason)),
            duration,
        });
    };

    // Strip internal fields before applying the turn grant so a model
    // cannot self-approve by emitting "approved": true in its raw args.
    let sanitized_arguments = strip_internal_approval_fields(call_arguments);
    let effective_arguments =
        apply_turn_grant_to_tool_args(call_name, sanitized_arguments, turn_grant);
    let tool_future = tool.execute(effective_arguments);
    let tool_result = if let Some(token) = cancellation_token {
        tokio::select! {
            () = token.cancelled() => return Err(ToolLoopCancelled.into()),
            result = tool_future => result,
        }
    } else {
        tool_future.await
    };

    match tool_result {
        Ok(r) => {
            let duration = start.elapsed();
            observer.record_event(&ObserverEvent::ToolCall {
                tool: call_name.to_string(),
                duration,
                success: r.success,
            });
            if r.success {
                Ok(ToolExecutionOutcome {
                    output: scrub_credentials(&r.output),
                    success: true,
                    error_reason: None,
                    duration,
                })
            } else {
                let reason = r.error.unwrap_or(r.output);
                Ok(ToolExecutionOutcome {
                    output: format!("Error: {reason}"),
                    success: false,
                    error_reason: Some(scrub_credentials(&reason)),
                    duration,
                })
            }
        }
        Err(e) => {
            let duration = start.elapsed();
            observer.record_event(&ObserverEvent::ToolCall {
                tool: call_name.to_string(),
                duration,
                success: false,
            });
            let reason = format!("Error executing {call_name}: {e}");
            Ok(ToolExecutionOutcome {
                output: reason.clone(),
                success: false,
                error_reason: Some(scrub_credentials(&reason)),
                duration,
            })
        }
    }
}

pub(super) struct ToolExecutionOutcome {
    pub(super) output: String,
    pub(super) success: bool,
    pub(super) error_reason: Option<String>,
    pub(super) duration: Duration,
}

pub(super) fn should_execute_tools_in_parallel(
    tool_calls: &[ParsedToolCall],
    approval: Option<&ApprovalManager>,
    approval_bypassed_for_turn: bool,
) -> bool {
    if tool_calls.len() <= 1 {
        return false;
    }

    if approval_bypassed_for_turn {
        return true;
    }

    if let Some(mgr) = approval {
        if tool_calls.iter().any(|call| mgr.needs_approval(&call.name)) {
            // Approval-gated calls must keep sequential handling so the caller can
            // enforce CLI prompt/deny policy consistently.
            return false;
        }
    }

    true
}

pub(super) async fn execute_tools_parallel(
    tool_calls: &[ParsedToolCall],
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    cancellation_token: Option<&CancellationToken>,
    turn_grant: Option<&NonCliTurnApprovalGrant>,
) -> Result<Vec<ToolExecutionOutcome>> {
    let futures: Vec<_> = tool_calls
        .iter()
        .map(|call| {
            execute_one_tool(
                &call.name,
                call.arguments.clone(),
                tools_registry,
                observer,
                cancellation_token,
                turn_grant,
            )
        })
        .collect();

    let results = futures_util::future::join_all(futures).await;
    results.into_iter().collect()
}

pub(super) async fn execute_tools_sequential(
    tool_calls: &[ParsedToolCall],
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    cancellation_token: Option<&CancellationToken>,
    turn_grant: Option<&NonCliTurnApprovalGrant>,
) -> Result<Vec<ToolExecutionOutcome>> {
    let mut outcomes = Vec::with_capacity(tool_calls.len());

    for call in tool_calls {
        outcomes.push(
            execute_one_tool(
                &call.name,
                call.arguments.clone(),
                tools_registry,
                observer,
                cancellation_token,
                turn_grant,
            )
            .await?,
        );
    }

    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_internal_approval_fields_removes_approved_and_plan_commands() {
        let args = serde_json::json!({
            "command": "rm -rf /tmp/test",
            "approved": true,
            crate::tools::shell::APPROVED_PLAN_SHELL_COMMANDS_ARG: ["rm -rf /tmp/test"]
        });

        let stripped = strip_internal_approval_fields(args);
        assert!(stripped.get("command").is_some());
        assert!(stripped.get("approved").is_none());
        assert!(stripped
            .get(crate::tools::shell::APPROVED_PLAN_SHELL_COMMANDS_ARG)
            .is_none());
    }

    #[test]
    fn strip_internal_approval_fields_preserves_normal_args() {
        let args = serde_json::json!({"command": "echo hello", "otp_code": "123456"});
        let stripped = strip_internal_approval_fields(args.clone());
        assert_eq!(stripped, args);
    }
}
