//! Runtime command handling for channel messages.
//!
//! Processes slash commands and natural-language approval commands
//! received through channels (e.g. `/models`, `/approve`, `/new`).

use crate::approval::{ApprovalResponse, PendingApprovalError};
use crate::config::NonCliNaturalLanguageApprovalMode;
use crate::observability::runtime_trace;
use crate::providers;
use std::fmt::Write;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::context::*;
use super::helpers::*;
use super::runtime_commands::non_cli_natural_language_mode_label;
use super::runtime_commands::{
    approval_target_label, is_approval_management_command, parse_runtime_command,
    ChannelRuntimeCommand, APPROVAL_ALL_TOOLS_ONCE_TOKEN,
};
use super::runtime_commands::{build_models_help_response, build_providers_help_response};
use super::runtime_config::{
    auto_unexclude_tool, is_non_cli_tool_excluded, snapshot_non_cli_excluded_tools,
};
use super::runtime_config::{
    describe_non_cli_approvals, persist_non_cli_approval_to_config,
    remove_non_cli_approval_from_config,
};
use super::traits::{self, SendMessage};

/// Format a pending-approval error with a trace event.
///
/// Shared by `/approve-confirm` and `/approve-deny` error arms.
fn format_pending_approval_error(
    error: PendingApprovalError,
    request_id: &str,
    sender: &str,
    source_channel: &str,
    event_name: &str,
    past_participle: &str,
) -> String {
    let (description, message) = match error {
        PendingApprovalError::NotFound => (
            "pending request not found".to_string(),
            format!(
                "Pending approval request `{request_id}` was not found. Create one with `/approve-request <tool-name>` or `/approve-all-once`."
            ),
        ),
        PendingApprovalError::Expired => (
            "pending request expired".to_string(),
            format!("Pending approval request `{request_id}` has expired."),
        ),
        PendingApprovalError::RequesterMismatch => (
            format!("pending request {past_participle} mismatch"),
            format!(
                "Pending approval request `{request_id}` can only be {past_participle} by the same sender in the same chat/channel that created it."
            ),
        ),
    };

    runtime_trace::record_event(
        event_name,
        Some(source_channel),
        None,
        None,
        None,
        Some(false),
        Some(&description),
        serde_json::json!({
            "request_id": request_id,
            "sender": sender,
            "channel": source_channel,
        }),
    );

    message
}

fn split_runtime_request_id_and_followup(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (String::new(), None);
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let request_id = parts.next().unwrap_or_default().trim().to_string();
    let follow_up = parts
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    (request_id, follow_up)
}

fn resolve_scoped_pending_request_id(
    ctx: &ChannelRuntimeContext,
    sender: &str,
    source_channel: &str,
    reply_target: &str,
    action_label: &str,
) -> Result<String, String> {
    let rows = ctx.approval_manager.list_non_cli_pending_requests(
        Some(sender),
        Some(source_channel),
        Some(reply_target),
    );

    match rows.as_slice() {
        [req] => Ok(req.request_id.clone()),
        [] => Err(format!(
            "No pending approval requests were found for this sender+chat/channel scope, so I could not {action_label} anything. Use `/approve-pending` to inspect pending approvals or retry the task so the runtime can create a fresh request."
        )),
        many => {
            let preview = many
                .iter()
                .map(|req| {
                    format!(
                        "`{}` ({})",
                        req.request_id,
                        approval_target_label(&req.tool_name)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "Multiple pending approval requests match this sender+chat/channel scope: {preview}\nUse `/approve-confirm <request-id>` or `/approve-deny <request-id>` explicitly."
            ))
        }
    }
}

pub(super) async fn handle_runtime_command_if_needed(
    ctx: Arc<ChannelRuntimeContext>,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<dyn traits::Channel>>,
) -> bool {
    let is_slash_command = msg.content.trim_start().starts_with('/');
    let Some(mut command) = parse_runtime_command(&msg.channel, &msg.content) else {
        return false;
    };

    let Some(channel) = target_channel else {
        return true;
    };

    let sender_key = conversation_history_key(msg);
    let mut current = super::route_state::get_route_selection(ctx.as_ref(), &sender_key);
    let sender = msg.sender.as_str();
    let source_channel = msg.channel.as_str();
    let reply_target = msg.reply_target.as_str();
    let is_natural_language_approval_command =
        !is_slash_command && is_approval_management_command(&command);

    if is_approval_management_command(&command)
        && !ctx
            .approval_manager
            .is_non_cli_approval_actor_allowed(source_channel, sender)
    {
        let mut approvers = ctx
            .approval_manager
            .non_cli_approval_approvers()
            .into_iter()
            .collect::<Vec<_>>();
        approvers.sort();
        let allowed = if approvers.is_empty() {
            "(any channel-allowed sender)".to_string()
        } else {
            approvers.join(", ")
        };
        let response = format!(
            "Approval-management command denied for sender `{sender}` on channel `{source_channel}`.\nAllowed approvers: {allowed}\nConfigure `[autonomy].non_cli_approval_approvers` to adjust this policy."
        );
        runtime_trace::record_event(
            "approval_management_denied",
            Some(source_channel),
            None,
            None,
            None,
            Some(false),
            Some("sender not allowed to manage non-cli approvals"),
            serde_json::json!({
                "sender": sender,
                "channel": source_channel,
                "allowed_approvers": approvers,
            }),
        );

        if let Err(err) = channel
            .send(&SendMessage::new(response, &msg.reply_target).in_thread(msg.thread_ts.clone()))
            .await
        {
            tracing::warn!(
                "Failed to send runtime command response on {}: {err}",
                channel.name()
            );
        }
        return true;
    }

    if is_natural_language_approval_command {
        let mode = ctx
            .approval_manager
            .non_cli_natural_language_approval_mode_for_channel(source_channel);
        match mode {
            NonCliNaturalLanguageApprovalMode::Disabled => {
                let response = "Natural-language approval commands are disabled by runtime policy.\nUse explicit slash commands such as `/approve <tool-name>`, `/approve-request <tool-name>`, `/approve-all-once`, `/approve-allow <request-id>`, `/approve-confirm <request-id>`, `/approve-deny <request-id>`, `/unapprove <tool-name>`, and `/approvals`.".to_string();
                runtime_trace::record_event(
                    "approval_management_natural_language_denied",
                    Some(source_channel),
                    None,
                    None,
                    None,
                    Some(false),
                    Some("natural-language approval commands disabled by policy"),
                    serde_json::json!({
                        "sender": sender,
                        "channel": source_channel,
                        "mode": non_cli_natural_language_mode_label(mode),
                    }),
                );
                if let Err(err) = channel
                    .send(
                        &SendMessage::new(response, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await
                {
                    tracing::warn!(
                        "Failed to send runtime command response on {}: {err}",
                        channel.name()
                    );
                }
                return true;
            }
            NonCliNaturalLanguageApprovalMode::RequestConfirm => {}
            NonCliNaturalLanguageApprovalMode::Direct => {
                if let ChannelRuntimeCommand::RequestToolApproval(tool_name) = &command {
                    command = ChannelRuntimeCommand::ApproveTool(tool_name.clone());
                    runtime_trace::record_event(
                        "approval_management_natural_language_promoted_to_direct",
                        Some(source_channel),
                        None,
                        None,
                        None,
                        Some(true),
                        Some("natural-language request promoted to direct approval"),
                        serde_json::json!({
                            "sender": sender,
                            "channel": source_channel,
                            "mode": non_cli_natural_language_mode_label(mode),
                        }),
                    );
                }
            }
        }
    }

    let mut auto_resume_message: Option<traits::ChannelMessage> = None;
    let response = match command {
        ChannelRuntimeCommand::ShowProviders => {
            build_providers_help_response(&current.provider, &current.model)
        }
        ChannelRuntimeCommand::SetProvider(raw_provider) => {
            match super::runtime_config::resolve_product_priority_provider_alias(&raw_provider) {
                Some(provider_name) => {
                    match get_or_create_provider(ctx.as_ref(), &provider_name).await {
                        Ok(_) => {
                            if provider_name != current.provider {
                                current.provider = provider_name.clone();
                                super::route_state::set_route_selection(
                                    ctx.as_ref(),
                                    &sender_key,
                                    current.clone(),
                                );
                                super::route_state::clear_sender_history(ctx.as_ref(), &sender_key);
                            }

                            format!(
                            "Provider switched to `{provider_name}` for this sender session. Current model is `{}`.\nUse `/model <model-id>` to set a provider-compatible model.",
                            current.model
                        )
                        }
                        Err(err) => {
                            let safe_err = providers::sanitize_api_error(&err.to_string());
                            format!(
                            "Failed to initialize provider `{provider_name}`. Route unchanged.\nDetails: {safe_err}"
                        )
                        }
                    }
                }
                None => match super::runtime_config::canonical_known_provider_name(&raw_provider) {
                    Some(provider_name) => format!(
                        "Channel provider switching is limited to product-priority providers: {}. `{provider_name}` remains available through config.toml or CLI, not `/models`.",
                        providers::PRODUCT_PROVIDER_PRIORITY.join(", ")
                    ),
                    None => format!(
                        "Unknown provider `{raw_provider}`. Use `/models` to list valid providers."
                    ),
                },
            }
        }
        ChannelRuntimeCommand::ShowModel => build_models_help_response(
            &current.provider,
            &current.model,
            &load_cached_model_preview(ctx.workspace_dir.as_path(), &current.provider),
        ),
        ChannelRuntimeCommand::SetModel(raw_model) => {
            let model = raw_model.trim().trim_matches('`').to_string();
            if model.is_empty() {
                "Model ID cannot be empty. Use `/model <model-id>`.".to_string()
            } else {
                current.model = model.clone();
                super::route_state::set_route_selection(ctx.as_ref(), &sender_key, current.clone());
                super::route_state::clear_sender_history(ctx.as_ref(), &sender_key);

                format!(
                    "Model switched to `{model}` for provider `{}` in this sender session.",
                    current.provider
                )
            }
        }
        ChannelRuntimeCommand::NewSession => {
            super::route_state::clear_sender_history(ctx.as_ref(), &sender_key);
            "Conversation history cleared. Starting fresh.".to_string()
        }
        ChannelRuntimeCommand::RequestAllToolsOnce => {
            let req = ctx.approval_manager.create_non_cli_pending_request(
                APPROVAL_ALL_TOOLS_ONCE_TOKEN,
                sender,
                source_channel,
                reply_target,
                None,
                Some("human-confirmed one-time bypass request for all tools/commands".to_string()),
                Vec::new(),
            );
            runtime_trace::record_event(
                "approval_request_created",
                Some(source_channel),
                None,
                None,
                None,
                Some(true),
                Some("pending one-time all-tools request created"),
                serde_json::json!({
                    "request_id": req.request_id,
                    "tool_name": req.tool_name,
                    "sender": sender,
                    "channel": source_channel,
                    "expires_at": req.expires_at,
                }),
            );
            format!(
                "One-time all-tools approval request created.\nRequest ID: `{}`\nScope: next non-CLI agent tool-execution turn may run without per-tool approval prompts.\nExpires: `{}`\nConfirm with `/approve-confirm {}` (must be the same sender in this chat/channel).",
                req.request_id, req.expires_at, req.request_id
            )
        }
        ChannelRuntimeCommand::RequestToolApproval(raw_tool_name) => {
            let tool_name = raw_tool_name.trim().to_string();
            if tool_name.is_empty() {
                "Usage: `/approve-request <tool-name>`".to_string()
            } else if !ctx
                .tools_registry
                .iter()
                .any(|tool| tool.name() == tool_name)
            {
                let mut available_tools = ctx
                    .tools_registry
                    .iter()
                    .map(|tool| tool.name().to_string())
                    .collect::<Vec<_>>();
                available_tools.sort();
                let preview = available_tools
                    .into_iter()
                    .take(12)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Unknown tool `{tool_name}`.\nKnown tools (top 12): {preview}\nUse `/approve-request <tool-name>` with an exact tool name."
                )
            } else if !ctx.approval_manager.needs_approval(&tool_name) {
                format!(
                    "`{tool_name}` is already approved in the current runtime policy. You can use it directly."
                )
            } else {
                let req = ctx.approval_manager.create_non_cli_pending_request(
                    &tool_name,
                    sender,
                    source_channel,
                    reply_target,
                    None,
                    None,
                    Vec::new(),
                );
                runtime_trace::record_event(
                    "approval_request_created",
                    Some(source_channel),
                    None,
                    None,
                    None,
                    Some(true),
                    Some("pending request created"),
                    serde_json::json!({
                        "request_id": req.request_id,
                        "tool_name": req.tool_name,
                        "sender": sender,
                        "channel": source_channel,
                        "expires_at": req.expires_at,
                    }),
                );
                format!(
                    "Approval request created.\nRequest ID: `{}`\nTool: `{}`\nExpires: `{}`\nConfirm with `/approve-confirm {}` (must be the same sender in this chat/channel).",
                    req.request_id, req.tool_name, req.expires_at, req.request_id
                )
            }
        }
        ChannelRuntimeCommand::ApprovePendingRequest(raw_request_id) => {
            let (request_id, _) = split_runtime_request_id_and_followup(&raw_request_id);
            if request_id.is_empty() {
                "Usage: `/approve-allow <request-id>`".to_string()
            } else {
                match ctx.approval_manager.confirm_non_cli_pending_request(
                    &request_id,
                    sender,
                    source_channel,
                    reply_target,
                ) {
                    Ok(req) => {
                        ctx.approval_manager
                            .record_non_cli_pending_resolution(&request_id, ApprovalResponse::Yes);
                        runtime_trace::record_event(
                            "approval_request_allowed",
                            Some(source_channel),
                            None,
                            None,
                            None,
                            Some(true),
                            Some("pending request allowed for current tool invocation"),
                            serde_json::json!({
                                "request_id": request_id,
                                "tool_name": req.tool_name,
                                "sender": sender,
                                "channel": source_channel,
                            }),
                        );
                        format!(
                            "Approved pending request `{}` for this invocation of `{}`.",
                            req.request_id, req.tool_name
                        )
                    }
                    Err(PendingApprovalError::NotFound) => {
                        format!("Pending approval request `{request_id}` was not found.")
                    }
                    Err(PendingApprovalError::Expired) => {
                        format!("Pending approval request `{request_id}` has expired.")
                    }
                    Err(PendingApprovalError::RequesterMismatch) => {
                        format!(
                            "Pending approval request `{request_id}` can only be approved by the same sender in the same chat/channel that created it."
                        )
                    }
                }
            }
        }
        ChannelRuntimeCommand::ConfirmToolApproval(raw_request_id) => {
            let (mut request_id, follow_up_content) =
                split_runtime_request_id_and_followup(&raw_request_id);
            let request_resolution = if request_id.is_empty() {
                if is_slash_command {
                    Err("Usage: `/approve-confirm <request-id>`".to_string())
                } else {
                    resolve_scoped_pending_request_id(
                        ctx.as_ref(),
                        sender,
                        source_channel,
                        reply_target,
                        "confirm",
                    )
                }
            } else {
                Ok(request_id.clone())
            };

            match request_resolution {
                Err(message) => message,
                Ok(scoped_request_id) => {
                    request_id = scoped_request_id;
                    match ctx.approval_manager.confirm_non_cli_pending_request(
                        &request_id,
                        sender,
                        source_channel,
                        reply_target,
                    ) {
                        Ok(req) => {
                            ctx.approval_manager.record_non_cli_pending_resolution(
                                &request_id,
                                ApprovalResponse::Yes,
                            );
                            let tool_name = req.tool_name.clone();
                            let resume_request = req.resume_request.clone();
                            let approval_message = if tool_name == APPROVAL_ALL_TOOLS_ONCE_TOKEN {
                                let _remaining = ctx.approval_manager.grant_non_cli_turn_grant(
                                    crate::approval::NonCliTurnApprovalGrant {
                                        approved_shell_commands: req
                                            .approved_shell_commands
                                            .clone(),
                                    },
                                );
                                if resume_request.is_some() {
                                    String::new()
                                } else {
                                    format!(
                                        "Approved one-time all-tools bypass from request `{request_id}`.\nApplies to the next non-CLI agent tool-execution turn only.\nThis bypass is runtime-only and does not persist to config.\nChannel exclusions from `autonomy.non_cli_excluded_tools` still apply."
                                    )
                                }
                            } else {
                                ctx.approval_manager.grant_non_cli_session(&tool_name);
                                ctx.approval_manager
                                    .apply_persistent_runtime_grant(&tool_name);
                                match persist_non_cli_approval_to_config(ctx.as_ref(), &tool_name)
                                    .await
                                {
                                    Ok(Some(path)) => format!(
                                        "Approved supervised execution for `{tool_name}` from request `{request_id}`.\nPersisted to `{}` so future channel sessions (including after restart) remain approved.",
                                        path.display()
                                    ),
                                    Ok(None) => format!(
                                        "Approved supervised execution for `{tool_name}` from request `{request_id}`.\nNo runtime config path was found, so this approval is active for the current daemon runtime only."
                                    ),
                                    Err(err) => format!(
                                        "Approved supervised execution for `{tool_name}` from request `{request_id}` in-memory.\nFailed to persist this approval to config: {err}"
                                    ),
                                }
                            };
                            runtime_trace::record_event(
                                "approval_request_confirmed",
                                Some(source_channel),
                                None,
                                None,
                                None,
                                Some(true),
                                Some("pending request confirmed"),
                                serde_json::json!({
                                    "request_id": request_id,
                                    "tool_name": tool_name,
                                    "sender": sender,
                                    "channel": source_channel,
                                }),
                            );

                            if let Some(content) = follow_up_content {
                                auto_resume_message = Some(traits::ChannelMessage {
                                    id: format!("{}:approval-followup", msg.id),
                                    sender: msg.sender.clone(),
                                    reply_target: msg.reply_target.clone(),
                                    content,
                                    channel: msg.channel.clone(),
                                    timestamp: msg.timestamp,
                                    thread_ts: msg.thread_ts.clone(),
                                });
                            } else if let Some(resume_request) = resume_request {
                                auto_resume_message = Some(traits::ChannelMessage {
                                    id: resume_request.message_id,
                                    sender: req.requested_by.clone(),
                                    reply_target: req.requested_reply_target.clone(),
                                    content: resume_request.content,
                                    channel: req.requested_channel.clone(),
                                    timestamp: resume_request.timestamp,
                                    thread_ts: resume_request.thread_ts,
                                });
                            }

                            if tool_name != APPROVAL_ALL_TOOLS_ONCE_TOKEN
                                && is_non_cli_tool_excluded(ctx.as_ref(), &tool_name)
                            {
                                let note = auto_unexclude_tool(ctx.as_ref(), &tool_name).await;
                                format!("{approval_message}\n{note}")
                            } else {
                                approval_message
                            }
                        }
                        Err(err) => format_pending_approval_error(
                            err,
                            &request_id,
                            sender,
                            source_channel,
                            "approval_request_confirmed",
                            "confirmed",
                        ),
                    }
                }
            }
        }
        ChannelRuntimeCommand::ConfirmToolApprovalAlways(raw_request_id) => {
            let (mut request_id, follow_up_content) =
                split_runtime_request_id_and_followup(&raw_request_id);
            let request_resolution = if request_id.is_empty() {
                if is_slash_command {
                    Err("Usage: `/approve-confirm-always <request-id>`".to_string())
                } else {
                    resolve_scoped_pending_request_id(
                        ctx.as_ref(),
                        sender,
                        source_channel,
                        reply_target,
                        "confirm-always",
                    )
                }
            } else {
                Ok(request_id.clone())
            };

            match request_resolution {
                Err(message) => message,
                Ok(scoped_request_id) => {
                    request_id = scoped_request_id;
                    match ctx.approval_manager.confirm_non_cli_pending_request(
                        &request_id,
                        sender,
                        source_channel,
                        reply_target,
                    ) {
                        Ok(req) => {
                            ctx.approval_manager.record_non_cli_pending_resolution(
                                &request_id,
                                ApprovalResponse::Yes,
                            );
                            let tool_name = req.tool_name.clone();
                            let resume_request = req.resume_request.clone();

                            // Turn grant so the current request executes immediately
                            let _remaining = ctx.approval_manager.grant_non_cli_turn_grant(
                                crate::approval::NonCliTurnApprovalGrant {
                                    approved_shell_commands: req
                                        .approved_shell_commands
                                        .clone(),
                                },
                            );

                            if tool_name != APPROVAL_ALL_TOOLS_ONCE_TOKEN {
                                ctx.approval_manager.grant_non_cli_session(&tool_name);
                                ctx.approval_manager
                                    .apply_persistent_runtime_grant(&tool_name);
                            }

                            runtime_trace::record_event(
                                "approval_request_confirmed_always",
                                Some(source_channel),
                                None,
                                None,
                                None,
                                Some(true),
                                Some("pending request confirmed with session-level grant"),
                                serde_json::json!({
                                    "request_id": request_id,
                                    "tool_name": tool_name,
                                    "sender": sender,
                                    "channel": source_channel,
                                }),
                            );

                            let has_resume = resume_request.is_some();
                            if let Some(content) = follow_up_content {
                                auto_resume_message = Some(traits::ChannelMessage {
                                    id: format!("{}:approval-followup", msg.id),
                                    sender: msg.sender.clone(),
                                    reply_target: msg.reply_target.clone(),
                                    content,
                                    channel: msg.channel.clone(),
                                    timestamp: msg.timestamp,
                                    thread_ts: msg.thread_ts.clone(),
                                });
                            } else if let Some(resume_request) = resume_request {
                                auto_resume_message = Some(traits::ChannelMessage {
                                    id: resume_request.message_id,
                                    sender: req.requested_by.clone(),
                                    reply_target: req.requested_reply_target.clone(),
                                    content: resume_request.content,
                                    channel: req.requested_channel.clone(),
                                    timestamp: resume_request.timestamp,
                                    thread_ts: resume_request.thread_ts,
                                });
                            }

                            if has_resume {
                                String::new()
                            } else if tool_name == APPROVAL_ALL_TOOLS_ONCE_TOKEN {
                                format!(
                                    "Approved one-time all-tools bypass from request `{request_id}`.\nApplies to the next non-CLI agent tool-execution turn only.\nThis bypass is runtime-only and does not persist to config."
                                )
                            } else {
                                format!(
                                    "Approved supervised execution for `{tool_name}` for this daemon session from request `{request_id}`.\nFuture `{tool_name}` calls will not require approval until the daemon restarts."
                                )
                            }
                        }
                        Err(err) => format_pending_approval_error(
                            err,
                            &request_id,
                            sender,
                            source_channel,
                            "approval_request_confirmed_always",
                            "confirmed-always",
                        ),
                    }
                }
            }
        }
        ChannelRuntimeCommand::DenyToolApproval(raw_request_id) => {
            let (mut request_id, _) = split_runtime_request_id_and_followup(&raw_request_id);
            let request_resolution = if request_id.is_empty() {
                if is_slash_command {
                    Err("Usage: `/approve-deny <request-id>`".to_string())
                } else {
                    resolve_scoped_pending_request_id(
                        ctx.as_ref(),
                        sender,
                        source_channel,
                        reply_target,
                        "deny",
                    )
                }
            } else {
                Ok(request_id.clone())
            };

            match request_resolution {
                Err(message) => message,
                Ok(scoped_request_id) => {
                    request_id = scoped_request_id;
                    match ctx.approval_manager.reject_non_cli_pending_request(
                        &request_id,
                        sender,
                        source_channel,
                        reply_target,
                    ) {
                        Ok(req) => {
                            ctx.approval_manager.record_non_cli_pending_resolution(
                                &request_id,
                                ApprovalResponse::No,
                            );
                            runtime_trace::record_event(
                                "approval_request_denied",
                                Some(source_channel),
                                None,
                                None,
                                None,
                                Some(true),
                                Some("pending request denied"),
                                serde_json::json!({
                                    "request_id": request_id,
                                    "tool_name": req.tool_name,
                                    "sender": sender,
                                    "channel": source_channel,
                                }),
                            );
                            format!(
                                "Denied pending approval request `{}` for tool `{}`.",
                                req.request_id, req.tool_name
                            )
                        }
                        Err(err) => format_pending_approval_error(
                            err,
                            &request_id,
                            sender,
                            source_channel,
                            "approval_request_denied",
                            "denied",
                        ),
                    }
                }
            }
        }
        ChannelRuntimeCommand::ListPendingApprovals => {
            let rows = ctx.approval_manager.list_non_cli_pending_requests(
                Some(sender),
                Some(source_channel),
                Some(reply_target),
            );
            if rows.is_empty() {
                "No pending approval requests for your current sender+chat/channel scope."
                    .to_string()
            } else {
                let mut response = String::new();
                response.push_str("Pending approval requests (sender+chat/channel scoped):\n");
                for req in rows {
                    let reason = req
                        .reason
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or("n/a");
                    let _ = writeln!(
                        response,
                        "- {}: tool={}, expires_at={}, reason={}",
                        req.request_id,
                        approval_target_label(&req.tool_name),
                        req.expires_at,
                        reason
                    );
                }
                response
            }
        }
        ChannelRuntimeCommand::ApproveTool(raw_tool_name) => {
            let tool_name = raw_tool_name.trim().to_string();
            if tool_name.is_empty() {
                "Usage: `/approve <tool-name>`".to_string()
            } else if !ctx
                .tools_registry
                .iter()
                .any(|tool| tool.name() == tool_name)
            {
                let mut available_tools = ctx
                    .tools_registry
                    .iter()
                    .map(|tool| tool.name().to_string())
                    .collect::<Vec<_>>();
                available_tools.sort();
                let preview = available_tools
                    .into_iter()
                    .take(12)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Unknown tool `{tool_name}`.\nKnown tools (top 12): {preview}\nUse `/approve <tool-name>` with an exact tool name."
                )
            } else {
                let cleared_pending = ctx
                    .approval_manager
                    .clear_non_cli_pending_requests_for_tool(&tool_name);
                ctx.approval_manager.grant_non_cli_session(&tool_name);
                ctx.approval_manager
                    .apply_persistent_runtime_grant(&tool_name);
                let persistence_message = match persist_non_cli_approval_to_config(ctx.as_ref(), &tool_name).await {
                    Ok(Some(path)) => format!(
                        "Approved supervised execution for `{tool_name}`.\nPersisted to `{}` so future channel sessions (including after restart) remain approved.",
                        path.display()
                    ),
                    Ok(None) => format!(
                        "Approved supervised execution for `{tool_name}`.\nNo runtime config path was found, so this approval is active for the current daemon runtime only."
                    ),
                    Err(err) => format!(
                        "Approved supervised execution for `{tool_name}` in-memory.\nFailed to persist this approval to config: {err}"
                    ),
                };

                if is_non_cli_tool_excluded(ctx.as_ref(), &tool_name) {
                    let note = auto_unexclude_tool(ctx.as_ref(), &tool_name).await;
                    format!("{persistence_message}\nRuntime pending requests cleared: {cleared_pending}.\n{note}")
                } else {
                    format!("{persistence_message}\nRuntime pending requests cleared: {cleared_pending}.")
                }
            }
        }
        ChannelRuntimeCommand::UnapproveTool(raw_tool_name) => {
            let tool_name = raw_tool_name.trim().to_string();
            if tool_name.is_empty() {
                "Usage: `/unapprove <tool-name>`".to_string()
            } else {
                let removed_session = ctx.approval_manager.revoke_non_cli_session(&tool_name);
                let removed_runtime_persistent = ctx
                    .approval_manager
                    .apply_persistent_runtime_revoke(&tool_name);
                let removed_pending = ctx
                    .approval_manager
                    .clear_non_cli_pending_requests_for_tool(&tool_name);
                match remove_non_cli_approval_from_config(ctx.as_ref(), &tool_name).await {
                    Ok(Some((path, removed_persistent))) => format!(
                        "Persistent approval removed for `{tool_name}`: {}.\nRuntime effective auto_approve removed: {}.\nRuntime pending requests cleared: {}.\nConfig path: `{}`.\nRuntime session grant removed: {}.",
                        if removed_persistent { "yes" } else { "no (not present)" },
                        if removed_runtime_persistent { "yes" } else { "no (not present)" },
                        removed_pending,
                        path.display(),
                        if removed_session { "yes" } else { "no" }
                    ),
                    Ok(None) => format!(
                        "Runtime config path was not found.\nRuntime session grant removed for `{tool_name}`: {}.",
                        if removed_session { "yes" } else { "no" }
                    ),
                    Err(err) => format!(
                        "Removed runtime session grant for `{tool_name}`: {}.\nFailed to persist removal to config: {err}",
                        if removed_session { "yes" } else { "no" }
                    ),
                }
            }
        }
        ChannelRuntimeCommand::ListApprovals => {
            match describe_non_cli_approvals(
                ctx.as_ref(),
                sender,
                source_channel,
                reply_target,
                &snapshot_non_cli_excluded_tools(ctx.as_ref()),
            )
            .await
            {
                Ok(summary) => summary,
                Err(err) => format!("Failed to read approval state: {err}"),
            }
        }
    };

    if !response.trim().is_empty() {
        if let Err(err) = channel
            .send(&SendMessage::new(response, &msg.reply_target).in_thread(msg.thread_ts.clone()))
            .await
        {
            tracing::warn!(
                "Failed to send runtime command response on {}: {err}",
                channel.name()
            );
        }
    }

    if let Some(resume_msg) = auto_resume_message {
        runtime_trace::record_event(
            "approval_request_auto_resume_started",
            Some(source_channel),
            None,
            None,
            None,
            Some(true),
            Some("resuming original non-cli request after approval"),
            serde_json::json!({
                "approval_message_id": msg.id,
                "resumed_message_id": resume_msg.id.clone(),
                "sender": resume_msg.sender.clone(),
                "channel": resume_msg.channel.clone(),
            }),
        );
        Box::pin(
            super::message_processing::process_channel_message_with_options(
                Arc::clone(&ctx),
                resume_msg,
                CancellationToken::new(),
                ProcessChannelMessageOptions {
                    resume_existing_user_turn: true,
                    approved_auto_resume: true,
                },
            ),
        )
        .await;
    }

    true
}
