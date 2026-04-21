use super::parsing::ParsedToolCall;
use crate::util::truncate_with_ellipsis;
use std::collections::BTreeSet;
use std::fmt::Write;

const AUTO_CRON_DELIVERY_CHANNELS: &[&str] = &["telegram", "discord", "slack", "mattermost"];
const MAX_NON_CLI_APPROVAL_CALLS_DISPLAY: usize = 6;
const MAX_NON_CLI_APPROVAL_DETAIL_CHARS: usize = 140;

/// Extract a short hint from tool call arguments for progress display.
pub(super) fn truncate_tool_args_for_progress(
    name: &str,
    args: &serde_json::Value,
    max_len: usize,
) -> String {
    let hint = match name {
        "shell" => args.get("command").and_then(|v| v.as_str()),
        "file_read" | "file_write" => args.get("path").and_then(|v| v.as_str()),
        "web_fetch" | "browser_open" => args.get("url").and_then(|v| v.as_str()),
        _ => args
            .get("action")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("query").and_then(|v| v.as_str())),
    };
    match hint {
        Some(s) => truncate_with_ellipsis(s, max_len),
        None => String::new(),
    }
}

pub(super) fn qualifies_for_non_cli_investigation_batch(
    tool_name: &str,
    args: &serde_json::Value,
) -> bool {
    match tool_name {
        // Planning is stateful but low-risk and commonly used to organize one
        // investigation turn; avoid prompting for every create/update step.
        "task_plan" | "glob_search" | "content_search" | "lossless_search" | "file_read"
        | "memory_search" | "memory_recall" => true,
        "process" => matches!(
            args.get("action").and_then(serde_json::Value::as_str),
            Some("list" | "output")
        ),
        _ => false,
    }
}

fn approval_arg_preview(args: &serde_json::Value) -> Option<String> {
    for (label, key) in [
        ("command", "command"),
        ("url", "url"),
        ("path", "path"),
        ("operation", "operation"),
        ("query", "query"),
        ("action", "action"),
        ("pattern", "pattern"),
        ("name", "name"),
        ("prompt", "prompt"),
        ("value", "value"),
    ] {
        if let Some(value) = args
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let compact = value.replace('\n', " ");
            return Some(format!(
                "{label}: {}",
                truncate_with_ellipsis(&compact, MAX_NON_CLI_APPROVAL_DETAIL_CHARS)
            ));
        }
    }

    match args {
        serde_json::Value::Null => None,
        serde_json::Value::Object(map) if map.is_empty() => None,
        _ => {
            let compact = args.to_string().replace('\n', " ");
            Some(format!(
                "args: {}",
                truncate_with_ellipsis(&compact, MAX_NON_CLI_APPROVAL_DETAIL_CHARS)
            ))
        }
    }
}

pub(super) fn build_non_cli_approval_plan_prompt(
    tool_calls: &[ParsedToolCall],
) -> (String, String) {
    let title = "Approval required for current execution plan.".to_string();
    let mut details = String::new();

    let unique_tools = tool_calls
        .iter()
        .map(|call| format!("`{}`", call.name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    if tool_calls.len() == 1 {
        let call = &tool_calls[0];
        match approval_arg_preview(&call.arguments) {
            Some(preview) => {
                let _ = writeln!(details, "Run `{}` with {}.", call.name, preview);
            }
            None => {
                let _ = writeln!(details, "Run `{}`.", call.name);
            }
        }
    } else {
        let _ = writeln!(
            details,
            "Run {} planned call(s) across {} tool(s): {}.",
            tool_calls.len(),
            unique_tools.len(),
            unique_tools.join(", ")
        );
        for call in tool_calls.iter().take(MAX_NON_CLI_APPROVAL_CALLS_DISPLAY) {
            match approval_arg_preview(&call.arguments) {
                Some(preview) => {
                    let _ = writeln!(details, "- `{}`: {}", call.name, preview);
                }
                None => {
                    let _ = writeln!(details, "- `{}`", call.name);
                }
            }
        }

        let hidden_calls = tool_calls
            .len()
            .saturating_sub(MAX_NON_CLI_APPROVAL_CALLS_DISPLAY);
        if hidden_calls > 0 {
            let _ = writeln!(details, "- ... {hidden_calls} more");
        }
    }

    details.push_str("Tap Approve to run this turn only. It will not persist.");

    (title, details)
}

pub(super) fn collect_planned_shell_commands(tool_calls: &[ParsedToolCall]) -> Vec<String> {
    tool_calls
        .iter()
        .filter_map(|call| match call.name.as_str() {
            "shell" => call
                .arguments
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(ToString::to_string),
            "process"
                if matches!(
                    call.arguments
                        .get("action")
                        .and_then(serde_json::Value::as_str),
                    Some("spawn_shell")
                ) =>
            {
                call.arguments
                    .get("shell_command")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        call.arguments
                            .get("command")
                            .and_then(serde_json::Value::as_str)
                    })
                    .map(str::trim)
                    .filter(|command| !command.is_empty())
                    .map(ToString::to_string)
            }
            _ => None,
        })
        .collect()
}

pub(super) fn maybe_inject_cron_add_delivery(
    tool_name: &str,
    tool_args: &mut serde_json::Value,
    channel_name: &str,
    reply_target: Option<&str>,
) {
    if tool_name != "cron_add"
        || !AUTO_CRON_DELIVERY_CHANNELS
            .iter()
            .any(|supported| supported == &channel_name)
    {
        return;
    }

    let Some(reply_target) = reply_target.map(str::trim).filter(|v| !v.is_empty()) else {
        return;
    };

    let Some(args_obj) = tool_args.as_object_mut() else {
        return;
    };

    let is_agent_job = match args_obj.get("job_type").and_then(serde_json::Value::as_str) {
        Some("agent") => true,
        Some(_) => false,
        None => args_obj.contains_key("prompt"),
    };
    if !is_agent_job {
        return;
    }

    let delivery = args_obj
        .entry("delivery".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(delivery_obj) = delivery.as_object_mut() else {
        return;
    };

    let mode = delivery_obj
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    if mode.eq_ignore_ascii_case("none") || mode.trim().is_empty() {
        delivery_obj.insert(
            "mode".to_string(),
            serde_json::Value::String("announce".to_string()),
        );
    } else if !mode.eq_ignore_ascii_case("announce") {
        // Respect explicitly chosen non-announce modes.
        return;
    }

    let needs_channel = delivery_obj
        .get("channel")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|value| value.trim().is_empty());
    if needs_channel {
        delivery_obj.insert(
            "channel".to_string(),
            serde_json::Value::String(channel_name.to_string()),
        );
    }

    let needs_target = delivery_obj
        .get("to")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|value| value.trim().is_empty());
    if needs_target {
        delivery_obj.insert(
            "to".to_string(),
            serde_json::Value::String(reply_target.to_string()),
        );
    }
}
