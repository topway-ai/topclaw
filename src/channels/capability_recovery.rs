use super::capability_detection::{
    extract_json_object, looks_like_file_read_task, looks_like_file_write_task,
    looks_like_remote_repo_review_request, looks_like_shell_task, looks_like_web_task,
    should_try_llm_capability_recovery,
};
use super::runtime_helpers::exclusion_set;
use super::{traits, ChannelRuntimeContext};
use crate::approval::{ApprovalManager, PendingNonCliResumeRequest};
use crate::providers::Provider;
use crate::tools::Tool;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(super) enum CapabilityRecoveryKind {
    WebAccess,
    ShellAccess,
    FileReadAccess,
    FileWriteAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CapabilityState {
    Available,
    NeedsApproval,
    Excluded,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CapabilityToolCandidate {
    tool_name: &'static str,
    setup_hint: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapabilityRecoveryApprovalPrompt {
    pub(super) request_id: String,
    pub(super) title: String,
    pub(super) details: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapabilityRecoveryPlan {
    pub(super) kind: CapabilityRecoveryKind,
    pub(super) tool_name: String,
    pub(super) state: CapabilityState,
    pub(super) reason: String,
    pub(super) message: String,
    pub(super) approval_prompt: Option<CapabilityRecoveryApprovalPrompt>,
}

fn build_capability_recovery_approval_prompt(
    tool_name: &str,
    request_id: &str,
) -> CapabilityRecoveryApprovalPrompt {
    CapabilityRecoveryApprovalPrompt {
        request_id: request_id.to_string(),
        title: format!("I can finish this, but I need supervised access to `{tool_name}` first."),
        details:
            "Confirm from this same chat/channel and I’ll resume the blocked request automatically."
                .to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChannelTurnIntent {
    DirectReply,
    NeedsTools,
    NeedsClarification,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChannelTurnIntentPlan {
    pub(super) intent: ChannelTurnIntent,
    pub(super) reason: String,
}

#[derive(Debug, Deserialize)]
struct LlmTurnIntentSuggestion {
    #[serde(default)]
    intent: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlmCapabilityRecoverySuggestion {
    #[serde(default)]
    need_recovery: bool,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn tool_is_registered(tools_registry: &[Box<dyn Tool>], tool_name: &str) -> bool {
    tools_registry.iter().any(|tool| tool.name() == tool_name)
}

/// Determine capability state using a pre-built exclusion set (avoids
/// re-hashing on every call when checking multiple tools in a loop).
fn capability_tool_state_with_set(
    tools_registry: &[Box<dyn Tool>],
    excluded: &HashSet<String>,
    approval_manager: &ApprovalManager,
    tool_name: &str,
) -> CapabilityState {
    if !tool_is_registered(tools_registry, tool_name) {
        return CapabilityState::Missing;
    }
    if excluded.contains(&tool_name.trim().to_ascii_lowercase()) {
        CapabilityState::Excluded
    } else if approval_manager.non_cli_allow_all_once_remaining() > 0
        || approval_manager.is_non_cli_session_granted(tool_name)
    {
        CapabilityState::Available
    } else if approval_manager.needs_approval(tool_name) {
        CapabilityState::NeedsApproval
    } else {
        CapabilityState::Available
    }
}

/// Convenience wrapper for single-tool checks (builds the set internally).
pub(super) fn capability_tool_state(
    tools_registry: &[Box<dyn Tool>],
    excluded_tools: &[String],
    approval_manager: &ApprovalManager,
    tool_name: &str,
) -> CapabilityState {
    let excluded = exclusion_set(excluded_tools);
    capability_tool_state_with_set(tools_registry, &excluded, approval_manager, tool_name)
}

fn create_capability_recovery_plan(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    excluded_tools: &[String],
    kind: CapabilityRecoveryKind,
    candidates: &[CapabilityToolCandidate],
    reason: &'static str,
) -> Option<CapabilityRecoveryPlan> {
    let excluded = exclusion_set(excluded_tools);
    let mut first_missing: Option<CapabilityToolCandidate> = None;
    let mut first_excluded: Option<CapabilityToolCandidate> = None;

    for candidate in candidates {
        match capability_tool_state_with_set(
            ctx.tools_registry.as_ref(),
            &excluded,
            ctx.approval_manager.as_ref(),
            candidate.tool_name,
        ) {
            CapabilityState::Available => return None,
            CapabilityState::NeedsApproval => {
                let req = ctx.approval_manager.create_non_cli_pending_request(
                    candidate.tool_name,
                    &msg.sender,
                    &msg.channel,
                    &msg.reply_target,
                    Some(PendingNonCliResumeRequest {
                        message_id: msg.id.clone(),
                        content: msg.content.clone(),
                        timestamp: msg.timestamp,
                        thread_ts: msg.thread_ts.clone(),
                    }),
                    Some(reason.to_string()),
                    Vec::new(),
                );
                let approval_prompt =
                    build_capability_recovery_approval_prompt(candidate.tool_name, &req.request_id);
                let message = format!(
                    "{}\nRequest ID: `{}`\nConfirm with `/approve-confirm {}` from this same chat/channel and I’ll resume the blocked request automatically.",
                    approval_prompt.title, req.request_id, req.request_id
                );
                return Some(CapabilityRecoveryPlan {
                    kind,
                    tool_name: candidate.tool_name.to_string(),
                    state: CapabilityState::NeedsApproval,
                    reason: reason.to_string(),
                    message,
                    approval_prompt: Some(approval_prompt),
                });
            }
            CapabilityState::Excluded => {
                if first_excluded.is_none() {
                    first_excluded = Some(*candidate);
                }
            }
            CapabilityState::Missing => {
                if first_missing.is_none() {
                    first_missing = Some(*candidate);
                }
            }
        }
    }

    if let Some(candidate) = first_excluded {
        return Some(CapabilityRecoveryPlan {
            kind,
            tool_name: candidate.tool_name.to_string(),
            state: CapabilityState::Excluded,
            reason: reason.to_string(),
            message: format!(
                "I need `{}` for this request, but it’s currently blocked for chat channels.\nUse `/approve {}` to enable it, then retry your request.",
                candidate.tool_name, candidate.tool_name
            ),
            approval_prompt: None,
        });
    }

    first_missing.map(|candidate| CapabilityRecoveryPlan {
        kind,
        tool_name: candidate.tool_name.to_string(),
        state: CapabilityState::Missing,
        reason: reason.to_string(),
        message: candidate.setup_hint.to_string(),
        approval_prompt: None,
    })
}

pub(super) fn infer_capability_recovery_plan(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    excluded_tools: &[String],
) -> Option<CapabilityRecoveryPlan> {
    // Requests asking to expose commands/tool calls already used in this turn
    // should stay on the normal response path instead of being misclassified
    // as a fresh shell-capability deficit.
    if should_expose_internal_tool_details(&msg.content) {
        return None;
    }

    if looks_like_web_task(&msg.content) {
        // Repo-review prompts often include a remote origin URL even when the
        // local workspace is the authoritative source. More broadly, when no
        // remote-web tool is actually registered, let the normal agent path
        // continue so the model can choose an available local/tool-less path
        // instead of hard-failing before tool selection begins.
        if looks_like_remote_repo_review_request(&msg.content) {
            return None;
        }
        let plan = create_capability_recovery_plan(
            ctx,
            msg,
            excluded_tools,
            CapabilityRecoveryKind::WebAccess,
            &[
                CapabilityToolCandidate {
                    tool_name: "web_fetch",
                    setup_hint: "I can’t inspect that link from this chat yet because channel web access is not enabled.\nEnable `web_fetch`, `web_search_tool`, `http_request`, or `browser` for channel use, or send me local files/workspace content instead.",
                },
                CapabilityToolCandidate {
                    tool_name: "web_search_tool",
                    setup_hint: "I can’t search the web for this from this chat yet because web-search tooling is not enabled.\nEnable `web_search_tool`, `web_fetch`, `http_request`, or `browser` for channel use, or give me the relevant local material.",
                },
                CapabilityToolCandidate {
                    tool_name: "http_request",
                    setup_hint: "I can’t retrieve that remote content from this chat yet because channel HTTP access is not enabled.\nEnable `http_request`, `web_fetch`, `web_search_tool`, or `browser` for channel use, or give me the relevant local material.",
                },
                CapabilityToolCandidate {
                    tool_name: "browser",
                    setup_hint: "I can’t open that remote page from this chat yet because channel browser access is not enabled.\nEnable `browser`, `web_fetch`, `web_search_tool`, or `http_request` for channel use, or give me the relevant local material.",
                },
            ],
            "user request appears to require remote web access in a non-CLI channel",
        );
        return match plan {
            Some(plan) if matches!(plan.state, CapabilityState::Missing) => None,
            other => other,
        };
    }

    if looks_like_shell_task(&msg.content) {
        return create_capability_recovery_plan(
            ctx,
            msg,
            excluded_tools,
            CapabilityRecoveryKind::ShellAccess,
            &[CapabilityToolCandidate {
                tool_name: "shell",
                setup_hint: "I can’t run terminal commands for this chat right now.\nEnable the `shell` tool for channel use, or run the command locally and send me the output to continue.",
            }],
            "user request appears to require shell or terminal execution in a non-CLI channel",
        );
    }

    if looks_like_file_write_task(&msg.content) {
        return create_capability_recovery_plan(
            ctx,
            msg,
            excluded_tools,
            CapabilityRecoveryKind::FileWriteAccess,
            &[
                CapabilityToolCandidate {
                    tool_name: "file_edit",
                    setup_hint: "I can’t edit files for this chat right now.\nEnable `file_edit` or `file_write` for channel use, or switch to the local workspace flow.",
                },
                CapabilityToolCandidate {
                    tool_name: "file_write",
                    setup_hint: "I can’t write files for this chat right now.\nEnable `file_write` or `file_edit` for channel use, or switch to the local workspace flow.",
                },
            ],
            "user request appears to require file modification in a non-CLI channel",
        );
    }

    if looks_like_file_read_task(&msg.content) {
        return create_capability_recovery_plan(
            ctx,
            msg,
            excluded_tools,
            CapabilityRecoveryKind::FileReadAccess,
            &[CapabilityToolCandidate {
                tool_name: "file_read",
                setup_hint: "I can’t read local files for this chat right now.\nEnable `file_read` for channel use, or paste the file contents directly.",
            }],
            "user request appears to require local file inspection in a non-CLI channel",
        );
    }

    None
}

fn map_tool_to_capability_kind(tool_name: &str) -> Option<CapabilityRecoveryKind> {
    match tool_name {
        "web_fetch" | "web_search_tool" | "http_request" | "browser" => {
            Some(CapabilityRecoveryKind::WebAccess)
        }
        "shell" => Some(CapabilityRecoveryKind::ShellAccess),
        "file_read" => Some(CapabilityRecoveryKind::FileReadAccess),
        "file_edit" | "file_write" => Some(CapabilityRecoveryKind::FileWriteAccess),
        _ => None,
    }
}

fn parse_channel_turn_intent(raw: &str) -> Option<ChannelTurnIntent> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "direct_reply" => Some(ChannelTurnIntent::DirectReply),
        "needs_tools" => Some(ChannelTurnIntent::NeedsTools),
        "needs_clarification" => Some(ChannelTurnIntent::NeedsClarification),
        _ => None,
    }
}

pub(super) async fn try_llm_turn_intent(
    provider: &dyn Provider,
    msg: &traits::ChannelMessage,
    model: &str,
    temperature: f64,
    tools_registry: &[Box<dyn Tool>],
    excluded_tools: &[String],
) -> Option<ChannelTurnIntentPlan> {
    let trimmed = msg.content.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }

    let available_tools = if tools_registry.is_empty() {
        "(none)".to_string()
    } else {
        tools_registry
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let prompt = format!(
        "Classify how this non-CLI user turn should be handled before any tool or approval flow.\n\
Return JSON only with keys: intent (string), reason (string).\n\
Allowed intent values: direct_reply, needs_tools, needs_clarification.\n\
- direct_reply: answer naturally from current context without tools. Use for greetings, meta discussion, capability/model questions, brainstorming, or requests answerable from the current conversation/runtime context.\n\
- needs_tools: the user explicitly wants you to inspect external material, run commands, edit files, search/browse, or otherwise perform actions that require tool access.\n\
- needs_clarification: the user likely wants action, but the target repo/file/link/command or desired outcome is still underspecified.\n\
Do not choose needs_tools just because the topic mentions code, tooling, or a codebase.\n\
Available runtime tools: {available_tools}\n\
Excluded runtime tools: {}\n\
User request:\n{}",
        if excluded_tools.is_empty() {
            "(none)".to_string()
        } else {
            excluded_tools.join(", ")
        },
        msg.content
    );

    let raw = provider
        .chat_with_system(
            Some(
                "You are a strict turn-intent classifier for non-CLI channel messages. Output valid JSON only. Do not explain.",
            ),
            &prompt,
            model,
            temperature,
        )
        .await
        .ok()?;
    let json = extract_json_object(&raw)?;
    let suggestion: LlmTurnIntentSuggestion = serde_json::from_str(json).ok()?;
    let intent = parse_channel_turn_intent(suggestion.intent.as_deref()?)?;
    Some(ChannelTurnIntentPlan {
        intent,
        reason: suggestion.reason.unwrap_or_else(|| {
            "LLM classifier selected the turn-handling mode for this non-CLI message".to_string()
        }),
    })
}

pub(super) async fn try_llm_capability_recovery_plan(
    provider: &dyn Provider,
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    model: &str,
    temperature: f64,
    excluded_tools: &[String],
) -> Option<CapabilityRecoveryPlan> {
    if !should_try_llm_capability_recovery(&msg.content) {
        return None;
    }

    let available_tools = ctx
        .tools_registry
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let prompt = format!(
        "Classify whether this non-CLI request is blocked by a missing or gated capability.\n\
Return JSON only with keys: need_recovery (bool), tool_name (string), reason (string).\n\
Allowed tool_name values: web_fetch, web_search_tool, http_request, browser, shell, file_read, file_edit, file_write, none.\n\
Available runtime tools: {available_tools}\n\
Excluded runtime tools: {}\n\
User request:\n{}",
        if excluded_tools.is_empty() {
            "(none)".to_string()
        } else {
            excluded_tools.join(", ")
        },
        msg.content
    );

    let raw = provider
        .chat_with_system(
            Some(
                "You are a strict capability-recovery classifier. Output valid JSON only. Do not explain.",
            ),
            &prompt,
            model,
            temperature,
        )
        .await
        .ok()?;
    let json = extract_json_object(&raw)?;
    let suggestion: LlmCapabilityRecoverySuggestion = serde_json::from_str(json).ok()?;
    if !suggestion.need_recovery {
        return None;
    }

    let tool_name = suggestion.tool_name.as_deref()?.trim();
    if tool_name.is_empty() || tool_name.eq_ignore_ascii_case("none") {
        return None;
    }
    let kind = map_tool_to_capability_kind(tool_name)?;
    let reason = suggestion
        .reason
        .unwrap_or_else(|| "LLM classifier identified a likely missing capability".to_string());
    let state = capability_tool_state(
        ctx.tools_registry.as_ref(),
        excluded_tools,
        ctx.approval_manager.as_ref(),
        tool_name,
    );
    match state {
        CapabilityState::Available => None,
        CapabilityState::NeedsApproval => {
            let req = ctx.approval_manager.create_non_cli_pending_request(
                tool_name,
                &msg.sender,
                &msg.channel,
                &msg.reply_target,
                Some(PendingNonCliResumeRequest {
                    message_id: msg.id.clone(),
                    content: msg.content.clone(),
                    timestamp: msg.timestamp,
                    thread_ts: msg.thread_ts.clone(),
                }),
                Some(reason.clone()),
                Vec::new(),
            );
            Some(CapabilityRecoveryPlan {
                kind,
                tool_name: tool_name.to_string(),
                state,
                reason,
                message: format!(
                    "{}\nRequest ID: `{}`\nConfirm with `/approve-confirm {}` from this same chat/channel and I’ll resume the blocked request automatically.",
                    build_capability_recovery_approval_prompt(tool_name, &req.request_id).title,
                    req.request_id,
                    req.request_id
                ),
                approval_prompt: Some(build_capability_recovery_approval_prompt(
                    tool_name,
                    &req.request_id,
                )),
            })
        }
        CapabilityState::Excluded => Some(CapabilityRecoveryPlan {
            kind,
            tool_name: tool_name.to_string(),
            state,
            reason,
            message: format!(
                "I need `{tool_name}` for this request, but it's currently blocked for chat channels.\nUse `/approve {tool_name}` to enable it, then retry your request.",
            ),
            approval_prompt: None,
        }),
        CapabilityState::Missing => Some(CapabilityRecoveryPlan {
            kind,
            tool_name: tool_name.to_string(),
            state,
            reason,
            message: format!(
                "I identified `{}` as the missing capability for this request, but this runtime does not currently expose it.\nEnable that tool for channel use, or provide the needed material manually so I can continue.",
                tool_name
            ),
            approval_prompt: None,
        }),
    }
}

pub(super) fn should_expose_internal_tool_details(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let mentions_internal_details_en = lower.contains("command")
        || lower.contains("tool call")
        || lower.contains("function call")
        || lower.contains("execution trace")
        || lower.contains("internal step");
    let mentions_internal_details_cjk = trimmed.contains("命令")
        || trimmed.contains("工具调用")
        || trimmed.contains("函数调用")
        || trimmed.contains("执行过程");

    // Fail closed for negated phrasing ("don't show commands", "不要显示命令").
    const ENGLISH_NEGATIVE_HINTS: [&str; 18] = [
        "don't show command",
        "don't show commands",
        "do not show command",
        "do not show commands",
        "don't output command",
        "do not output command",
        "without command",
        "without commands",
        "no command output",
        "hide command",
        "hide commands",
        "omit command",
        "omit commands",
        "skip command",
        "skip commands",
        "don't show tool call",
        "do not show tool call",
        "do not show function call",
    ];
    if mentions_internal_details_en
        && ENGLISH_NEGATIVE_HINTS
            .iter()
            .any(|hint| lower.contains(hint))
    {
        return false;
    }

    const CJK_NEGATIVE_HINTS: [&str; 22] = [
        "不要输出命令",
        "不要显示命令",
        "不要展示命令",
        "不要带上命令",
        "不要附上命令",
        "别输出命令",
        "别显示命令",
        "别展示命令",
        "不要输出工具调用",
        "不要显示工具调用",
        "不要展示工具调用",
        "别输出工具调用",
        "别显示工具调用",
        "不要输出函数调用",
        "不要显示函数调用",
        "不要展示函数调用",
        "别输出函数调用",
        "别显示函数调用",
        "不要执行过程",
        "不要过程",
        "不要内部步骤",
        "别把命令",
    ];
    if mentions_internal_details_cjk && CJK_NEGATIVE_HINTS.iter().any(|hint| trimmed.contains(hint))
    {
        return false;
    }

    const ENGLISH_HINTS: [&str; 20] = [
        "show command",
        "show commands",
        "output command",
        "output commands",
        "print command",
        "print commands",
        "include command",
        "include commands",
        "with command",
        "with commands",
        "show tool call",
        "show tool calls",
        "show function call",
        "show function calls",
        "reveal tool call",
        "reveal function call",
        "tool call json",
        "function call json",
        "execution trace",
        "internal steps",
    ];
    if ENGLISH_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    const ENGLISH_VERBS: [&str; 7] = [
        "show", "output", "print", "include", "reveal", "display", "share",
    ];
    if mentions_internal_details_en && ENGLISH_VERBS.iter().any(|verb| lower.contains(verb)) {
        return true;
    }

    const CJK_HINTS: [&str; 14] = [
        "输出命令",
        "显示命令",
        "展示命令",
        "命令发给我",
        "带上命令",
        "输出工具调用",
        "显示工具调用",
        "展示工具调用",
        "输出函数调用",
        "显示函数调用",
        "展示函数调用",
        "函数指令",
        "工具指令",
        "执行过程",
    ];
    if CJK_HINTS.iter().any(|hint| trimmed.contains(hint)) {
        return true;
    }

    const CJK_VERBS: [&str; 9] = [
        "输出", "显示", "展示", "发我", "给我", "带上", "附上", "贴出", "列出",
    ];
    mentions_internal_details_cjk && CJK_VERBS.iter().any(|verb| trimmed.contains(verb))
}
