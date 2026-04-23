//! Core message processing pipeline for channel messages.
//!
//! Contains the main `process_channel_message` function that orchestrates
//! receiving a message, running the LLM tool-call loop, and delivering
//! the response back to the channel.

use super::channel_runtime_context::{
    with_channel_runtime_context, ChannelRuntimeContext as ToolChannelRuntimeContext,
};
use crate::agent::loop_::{
    is_non_cli_approval_pending, lossless::LosslessContext,
    run_tool_call_loop_with_non_cli_approval_context, scrub_credentials, NonCliApprovalContext,
};
use crate::config::Config;
use crate::observability::runtime_trace;
use crate::providers::{self, ChatMessage};
use crate::util::truncate_with_ellipsis;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::capability_detection::{
    build_local_capability_response, looks_like_desktop_computer_use_task,
    looks_like_repo_metrics_task, should_answer_local_capability_response_immediately,
    should_try_llm_capability_recovery,
};
use super::capability_recovery::{
    infer_capability_recovery_plan, should_expose_internal_tool_details,
    try_llm_capability_recovery_plan, CapabilityRecoveryPlan, CapabilityState,
};
use super::command_handler::handle_runtime_command_if_needed;
use super::context::*;
use super::dispatch::{spawn_progress_heartbeat_task, spawn_scoped_typing_task};
use super::helpers::*;
use super::prompt::{build_channel_system_prompt, build_current_turn_routing_hint};
use super::route_state::{
    append_sender_turn, compact_sender_history, get_route_selection, rollback_orphan_user_turn,
    set_sender_history,
};
use super::runtime_config::{
    build_runtime_tool_visibility_prompt, snapshot_non_cli_excluded_tools,
};
use super::runtime_config::{
    maybe_apply_runtime_config_update, runtime_config_path, runtime_defaults_snapshot,
};
use super::sanitize::sanitize_channel_response;
use super::traits::{self, SendMessage};

const INTERNAL_PROGRESS_MAX_LINES: usize = 5;

/// Trace + send a capability recovery plan. Returns `true` if a plan was dispatched.
async fn dispatch_capability_recovery(
    plan: &CapabilityRecoveryPlan,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<dyn traits::Channel>>,
    source: Option<&str>,
) {
    let mut metadata = serde_json::json!({
        "sender": msg.sender,
        "message_id": msg.id,
        "kind": format!("{:?}", plan.kind),
        "tool_name": plan.tool_name.as_str(),
        "state": format!("{:?}", plan.state),
    });
    if let Some(src) = source {
        metadata["source"] = serde_json::Value::String(src.to_string());
    }
    runtime_trace::record_event(
        "channel_message_capability_recovery",
        Some(msg.channel.as_str()),
        None,
        None,
        None,
        Some(matches!(plan.state, CapabilityState::NeedsApproval)),
        Some(plan.reason.as_str()),
        metadata,
    );
    if let Some(channel) = target_channel {
        let send_result = if let Some(prompt) = plan.approval_prompt.as_ref() {
            channel
                .send_approval_prompt(
                    &msg.reply_target,
                    &prompt.request_id,
                    &prompt.title,
                    &prompt.details,
                    msg.thread_ts.clone(),
                )
                .await
        } else {
            channel
                .send(
                    &SendMessage::new(&plan.message, &msg.reply_target)
                        .in_thread(msg.thread_ts.clone()),
                )
                .await
        };
        let _ = send_result;
    }
}

fn should_run_llm_capability_classifier(
    skip_pre_tool_turn_routing: bool,
    channel_name: &str,
    has_tools: bool,
    user_message: &str,
) -> bool {
    !skip_pre_tool_turn_routing
        && channel_name != "cli"
        && has_tools
        && should_try_llm_capability_recovery(user_message)
}

struct DirectTaskReply {
    response: String,
    trace_reason: &'static str,
}

fn runtime_tool_visible(visible_tool_names: &[String], tool_name: &str) -> bool {
    visible_tool_names
        .iter()
        .any(|visible| visible.eq_ignore_ascii_case(tool_name))
}

fn first_http_url(user_message: &str) -> Option<String> {
    user_message
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | '.'
                )
            })
        })
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(ToString::to_string)
}

fn repo_slug_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.replace('.', "-");
    let mut segments = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.trim_end_matches(".git"))
        .map(|segment| {
            segment
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                .collect::<String>()
        })
        .filter(|segment| !segment.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return None;
    }
    segments.insert(0, host);
    Some(segments.join("-"))
}

fn shell_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn compact_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx != 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn parse_cloc_report(raw: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let sum = parsed.get("SUM")?;
    let files = sum.get("nFiles").and_then(Value::as_u64).unwrap_or(0);
    let code = sum.get("code").and_then(Value::as_u64).unwrap_or(0);
    let comments = sum.get("comment").and_then(Value::as_u64).unwrap_or(0);
    let blanks = sum.get("blank").and_then(Value::as_u64).unwrap_or(0);

    let mut languages = parsed
        .as_object()?
        .iter()
        .filter_map(|(language, stats)| {
            if language == "header" || language == "SUM" {
                return None;
            }
            let code = stats.get("code").and_then(Value::as_u64)?;
            Some((language.clone(), code))
        })
        .collect::<Vec<_>>();
    languages.sort_by(|a, b| b.1.cmp(&a.1));

    let top_languages = languages
        .into_iter()
        .take(5)
        .map(|(language, code)| format!("`{language}` {}", compact_count(code)))
        .collect::<Vec<_>>();
    let language_line = if top_languages.is_empty() {
        String::new()
    } else {
        format!("\nTop languages by code: {}.", top_languages.join(", "))
    };

    Some(format!(
        "Cloned the repository locally and measured it with `cloc`.\nCode lines: `{}` across `{}` files.\nComments: `{}`. Blank lines: `{}`.{language_line}",
        compact_count(code),
        compact_count(files),
        compact_count(comments),
        compact_count(blanks),
    ))
}

fn find_tool<'a>(
    tools_registry: &'a [Box<dyn crate::tools::Tool>],
    tool_name: &str,
) -> Option<&'a dyn crate::tools::Tool> {
    tools_registry
        .iter()
        .find(|tool| tool.name() == tool_name)
        .map(|tool| tool.as_ref())
}

async fn run_direct_tool(
    ctx: &ChannelRuntimeContext,
    tool_name: &str,
    args: Value,
) -> Result<crate::tools::ToolResult, String> {
    let tool = find_tool(ctx.tools_registry.as_ref(), tool_name)
        .ok_or_else(|| format!("tool `{tool_name}` is not loaded"))?;
    tool.execute(args)
        .await
        .map_err(|err| scrub_credentials(&err.to_string()))
}

async fn try_handle_direct_repo_metrics_reply(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
) -> Option<DirectTaskReply> {
    if !looks_like_repo_metrics_task(&msg.content) {
        return None;
    }

    let repo_url = match first_http_url(&msg.content) {
        Some(url) => url,
        None => {
            return Some(DirectTaskReply {
                response: "I need the repository URL to measure exact line counts locally."
                    .to_string(),
                trace_reason: "direct repo metrics handler could not extract a repository URL",
            });
        }
    };
    let repo_slug = repo_slug_from_url(&repo_url).unwrap_or_else(|| "repo".to_string());
    let clone_root = ctx.workspace_dir.join("state").join("channel_repo_metrics");
    if let Err(err) = tokio::fs::create_dir_all(&clone_root).await {
        return Some(DirectTaskReply {
            response: format!(
                "I couldn’t prepare a local measurement workspace: {}",
                scrub_credentials(&err.to_string())
            ),
            trace_reason: "direct repo metrics handler failed to create clone workspace",
        });
    }

    let unique = chrono::Utc::now().timestamp_millis();
    let destination = format!("state/channel_repo_metrics/{repo_slug}-{unique}");
    let clone_command = format!(
        "git clone --depth 1 {} {}",
        shell_quote(&repo_url),
        shell_quote(&destination),
    );
    let clone_result = match run_direct_tool(
        ctx,
        "shell",
        json!({
            "command": clone_command,
            "approved": true
        }),
    )
    .await
    {
        Ok(result) if result.success => result,
        Ok(result) => {
            let detail = result
                .error
                .or_else(|| (!result.output.trim().is_empty()).then_some(result.output))
                .unwrap_or_else(|| "clone failed without stderr".to_string());
            return Some(DirectTaskReply {
                response: format!(
                    "I couldn’t clone the repository locally for exact measurement: {}",
                    scrub_credentials(&detail)
                ),
                trace_reason: "direct repo metrics handler failed during clone",
            });
        }
        Err(err) => {
            return Some(DirectTaskReply {
                response: format!(
                    "I couldn’t clone the repository locally for exact measurement: {err}"
                ),
                trace_reason: "direct repo metrics handler hit a clone transport error",
            });
        }
    };
    let _ = clone_result;

    let cloc_command = format!(
        "cloc --json --quiet --exclude-dir=.git {}",
        shell_quote(&destination),
    );
    let cloc_result = match run_direct_tool(
        ctx,
        "shell",
        json!({
            "command": cloc_command,
            "approved": true
        }),
    )
    .await
    {
        Ok(result) if result.success => result,
        Ok(result) => {
            let detail = result
                .error
                .or_else(|| (!result.output.trim().is_empty()).then_some(result.output))
                .unwrap_or_else(|| "cloc failed without stderr".to_string());
            return Some(DirectTaskReply {
                response: format!(
                    "I cloned the repository, but exact measurement with `cloc` failed: {}",
                    scrub_credentials(&detail)
                ),
                trace_reason: "direct repo metrics handler failed during cloc",
            });
        }
        Err(err) => {
            return Some(DirectTaskReply {
                response: format!(
                    "I cloned the repository, but exact measurement with `cloc` failed: {err}"
                ),
                trace_reason: "direct repo metrics handler hit a cloc transport error",
            });
        }
    };

    Some(DirectTaskReply {
        response: parse_cloc_report(&cloc_result.output).unwrap_or_else(|| {
            format!(
                "I cloned the repository and ran `cloc`, but I couldn’t parse the report cleanly.\nRaw output:\n```json\n{}\n```",
                cloc_result.output
            )
        }),
        trace_reason: "handled approved repo metrics request with direct local clone and cloc",
    })
}

async fn try_handle_direct_desktop_reply(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
) -> Option<DirectTaskReply> {
    if !looks_like_desktop_computer_use_task(&msg.content) {
        return None;
    }

    let lower = msg.content.to_ascii_lowercase();
    if lower.contains("search")
        || lower.contains("click")
        || lower.contains("type ")
        || lower.contains("select ")
        || lower.contains("fill ")
    {
        return None;
    }

    let scroll_to_bottom = lower.contains("scroll to the bottom")
        || lower.contains("scroll to bottom")
        || lower.contains("go to the bottom");
    let url = first_http_url(&msg.content);
    let app = if lower.contains("firefox") {
        "firefox"
    } else {
        "google-chrome"
    };

    let mut launch_args = json!({
        "action": "app_launch",
        "app": app,
    });
    if let Some(ref target_url) = url {
        launch_args["args"] = json!([target_url]);
    }
    match run_direct_tool(ctx, "computer_use", launch_args).await {
        Ok(result) if result.success => {}
        Ok(result) => {
            let detail = result
                .error
                .or_else(|| (!result.output.trim().is_empty()).then_some(result.output))
                .unwrap_or_else(|| "desktop launch failed without stderr".to_string());
            return Some(DirectTaskReply {
                response: format!(
                    "I couldn’t launch the requested desktop application: {}",
                    scrub_credentials(&detail)
                ),
                trace_reason: "direct desktop handler failed during app launch",
            });
        }
        Err(err) => {
            return Some(DirectTaskReply {
                response: format!("I couldn’t launch the requested desktop application: {err}"),
                trace_reason: "direct desktop handler hit an app-launch transport error",
            });
        }
    }

    let _ = run_direct_tool(
        ctx,
        "computer_use",
        json!({
            "action": "window_focus",
            "app": app,
        }),
    )
    .await;
    if scroll_to_bottom {
        let _ = run_direct_tool(
            ctx,
            "computer_use",
            json!({
                "action": "key_press",
                "key": "End",
            }),
        )
        .await;
    }

    let response = if let Some(target_url) = url {
        if scroll_to_bottom {
            format!(
                "Opened `{app}` with `{target_url}`, focused the page, and sent the page-end key to scroll to the bottom."
            )
        } else {
            format!("Opened `{app}` with `{target_url}`.")
        }
    } else if scroll_to_bottom {
        format!(
            "Opened `{app}`, focused the page, and sent the page-end key to scroll to the bottom."
        )
    } else {
        format!("Opened `{app}`.")
    };

    Some(DirectTaskReply {
        response,
        trace_reason:
            "handled approved desktop automation request with direct computer_use actions",
    })
}

async fn try_handle_direct_approved_task(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    visible_tool_names: &[String],
    approved_auto_resume: bool,
) -> Option<DirectTaskReply> {
    if !approved_auto_resume {
        return None;
    }

    if runtime_tool_visible(visible_tool_names, "shell") {
        if let Some(reply) = try_handle_direct_repo_metrics_reply(ctx, msg).await {
            return Some(reply);
        }
    }

    if runtime_tool_visible(visible_tool_names, "computer_use") {
        if let Some(reply) = try_handle_direct_desktop_reply(ctx, msg).await {
            return Some(reply);
        }
    }

    None
}

async fn send_local_reply(
    ctx: &ChannelRuntimeContext,
    msg: &traits::ChannelMessage,
    target_channel: Option<&Arc<dyn traits::Channel>>,
    history_key: &str,
    timestamped_content: &str,
    provider_name: &str,
    model_name: &str,
    response: &str,
    trace_reason: &str,
) {
    runtime_trace::record_event(
        "channel_message_local_capability_response",
        Some(msg.channel.as_str()),
        Some(provider_name),
        Some(model_name),
        None,
        Some(true),
        Some(trace_reason),
        serde_json::json!({
            "sender": msg.sender,
            "message_id": msg.id,
        }),
    );

    append_sender_turn(ctx, history_key, ChatMessage::user(timestamped_content));
    append_sender_turn(ctx, history_key, ChatMessage::assistant(response));

    if let Ok(mut lossless_context) = LosslessContext::for_session(
        ctx.workspace_dir.as_path(),
        "channel",
        history_key,
        ctx.system_prompt.as_str(),
    ) {
        let _ = lossless_context.record_raw_message(&ChatMessage::user(timestamped_content));
        let _ = lossless_context.record_raw_message(&ChatMessage::assistant(response));
    }

    println!("  🤖 Reply (0ms): {}", truncate_with_ellipsis(response, 80));
    if let Some(channel) = target_channel {
        let _ = channel
            .send(&SendMessage::new(response, &msg.reply_target).in_thread(msg.thread_ts.clone()))
            .await;
    }
}

#[allow(clippy::large_futures)]
pub(super) async fn process_channel_message(
    ctx: Arc<ChannelRuntimeContext>,
    msg: traits::ChannelMessage,
    cancellation_token: CancellationToken,
) {
    process_channel_message_with_options(
        ctx,
        msg,
        cancellation_token,
        ProcessChannelMessageOptions::default(),
    )
    .await;
}

#[allow(clippy::all)]
pub(super) async fn process_channel_message_with_options(
    ctx: Arc<ChannelRuntimeContext>,
    msg: traits::ChannelMessage,
    cancellation_token: CancellationToken,
    options: ProcessChannelMessageOptions,
) {
    if cancellation_token.is_cancelled() {
        return;
    }

    println!(
        "  \u{1F4AC} [{}] from {}: {}",
        msg.channel,
        msg.sender,
        truncate_with_ellipsis(&msg.content, 80)
    );
    runtime_trace::record_event(
        "channel_message_inbound",
        Some(msg.channel.as_str()),
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({
            "sender": msg.sender,
            "message_id": msg.id,
            "reply_target": msg.reply_target,
            "content_preview": truncate_with_ellipsis(&msg.content, 160),
        }),
    );

    // -- Hook: on_message_received (modifying) --
    let msg = if let Some(hooks) = &ctx.hooks {
        match hooks.run_on_message_received(msg).await {
            crate::hooks::HookResult::Cancel(reason) => {
                tracing::info!(%reason, "incoming message dropped by hook");
                return;
            }
            crate::hooks::HookResult::Continue(modified) => modified,
        }
    } else {
        msg
    };

    let target_channel = ctx.channels_by_name.get(&msg.channel).cloned();
    if let Err(err) = maybe_apply_runtime_config_update(ctx.as_ref()).await {
        tracing::warn!("Failed to apply runtime config update: {err}");
    }
    if handle_runtime_command_if_needed(Arc::clone(&ctx), &msg, target_channel.as_ref()).await {
        return;
    }

    // React with eyes immediately to acknowledge we received the message,
    // before any heavy work (config reads, classifier LLM calls, etc.).
    if let Some(channel) = target_channel.as_ref() {
        if let Err(e) = channel
            .add_reaction(&msg.reply_target, &msg.id, "\u{1F440}")
            .await
        {
            tracing::debug!("Failed to add reaction: {e}");
        }
    }

    let mut canary_enabled_for_turn = false;
    if !msg.content.trim_start().starts_with('/') {
        let semantic_cfg = if let Some(config_path) = runtime_config_path(ctx.as_ref()) {
            match tokio::fs::read_to_string(&config_path).await {
                Ok(contents) => match toml::from_str::<Config>(&contents) {
                    Ok(mut cfg) => {
                        cfg.config_path = config_path;
                        cfg.apply_env_overrides();
                        Some((
                            cfg.security.canary_tokens,
                            cfg.security.semantic_guard,
                            cfg.security.semantic_guard_collection.clone(),
                            cfg.security.semantic_guard_threshold,
                        ))
                    }
                    Err(err) => {
                        tracing::debug!("semantic guard: failed to parse runtime config: {err}");
                        None
                    }
                },
                Err(err) => {
                    tracing::debug!("semantic guard: failed to read runtime config: {err}");
                    None
                }
            }
        } else {
            None
        };

        if let Some((canary_enabled, semantic_enabled, semantic_collection, semantic_threshold)) =
            semantic_cfg
        {
            canary_enabled_for_turn = canary_enabled;
            if semantic_enabled {
                let semantic_guard = crate::security::SemanticGuard::new(semantic_enabled);
                if let Some(detection) = semantic_guard.detect(&msg.content) {
                    runtime_trace::record_event(
                        "channel_message_blocked_semantic_guard",
                        Some(msg.channel.as_str()),
                        None,
                        None,
                        None,
                        Some(false),
                        Some("blocked by semantic prompt-injection guard"),
                        serde_json::json!({
                            "sender": msg.sender,
                            "message_id": msg.id,
                            "score": detection.score,
                            "threshold": semantic_threshold,
                            "category": detection.category,
                            "collection": semantic_collection,
                        }),
                    );

                    if let Some(channel) = target_channel.as_ref() {
                        let warning = format!(
                            "Request blocked by `security.semantic_guard` before provider execution.\n\
                        semantic_match={:.2} (threshold {:.2}), category={}.",
                            detection.score, semantic_threshold, detection.category
                        );
                        let _ = channel
                            .send(
                                &SendMessage::new(warning, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                    return;
                }
            }
        }
    }

    let history_key = conversation_history_key(&msg);
    // Try classification first, fall back to sender/default route
    let route = classify_message_route(ctx.as_ref(), &msg.content)
        .unwrap_or_else(|| get_route_selection(ctx.as_ref(), &history_key));
    let excluded_tools_snapshot = if msg.channel == "cli" {
        Vec::new()
    } else {
        snapshot_non_cli_excluded_tools(ctx.as_ref())
    };
    let effective_tools_registry: &[Box<dyn crate::tools::Tool>] = ctx.tools_registry.as_ref();
    let visible_tool_names: Vec<String> = effective_tools_registry
        .iter()
        .filter(|tool| {
            !excluded_tools_snapshot
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(tool.name()))
        })
        .map(|tool| tool.name().to_string())
        .collect();
    let local_capability_response = build_local_capability_response(
        &msg.content,
        ctx.system_prompt.as_str(),
        &route.provider,
        &route.model,
        &visible_tool_names,
    );
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let timestamped_content = format!("[{now}] {}", msg.content);

    if let Some(direct_reply) = try_handle_direct_approved_task(
        ctx.as_ref(),
        &msg,
        &visible_tool_names,
        options.approved_auto_resume,
    )
    .await
    {
        send_local_reply(
            ctx.as_ref(),
            &msg,
            target_channel.as_ref(),
            &history_key,
            &timestamped_content,
            &route.provider,
            &route.model,
            &direct_reply.response,
            direct_reply.trace_reason,
        )
        .await;
        return;
    }

    if let Some(local_response) = local_capability_response
        .as_ref()
        .filter(|_| should_answer_local_capability_response_immediately(&msg.content))
    {
        send_local_reply(
            ctx.as_ref(),
            &msg,
            target_channel.as_ref(),
            &history_key,
            &timestamped_content,
            &route.provider,
            &route.model,
            local_response,
            "answered capability/workflow question locally before provider execution",
        )
        .await;
        return;
    }

    let runtime_defaults = runtime_defaults_snapshot(ctx.as_ref());
    let active_provider = match get_or_create_provider(ctx.as_ref(), &route.provider).await {
        Ok(provider) => provider,
        Err(err) => {
            let message = if let Some(local_response) = local_capability_response.clone() {
                runtime_trace::record_event(
                    "channel_message_local_capability_response",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(true),
                    Some("answered capability question locally after provider init failure"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "message_id": msg.id,
                    }),
                );
                local_response
            } else {
                let safe_err = providers::sanitize_api_error(&err.to_string());
                format!(
                    "\u{26A0}\u{FE0F} Failed to initialize provider `{}`. Please run `/models` to choose another provider.\nDetails: {safe_err}",
                    route.provider
                )
            };
            if let Some(channel) = target_channel.as_ref() {
                let _ = channel
                    .send(
                        &SendMessage::new(message, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await;
            }
            return;
        }
    };
    let skip_pre_tool_turn_routing = options.approved_auto_resume;
    // Only the capability-recovery classifier is left, and it should run only
    // when the user is actually talking about a missing capability/skill. Do
    // not show "Analyzing the request..." for normal work turns that go
    // straight to the main tool loop.
    let will_run_classifiers = should_run_llm_capability_classifier(
        skip_pre_tool_turn_routing,
        &msg.channel,
        !ctx.tools_registry.is_empty(),
        &msg.content,
    );

    // Start typing indicator before the classifier LLM calls so the user
    // sees immediate feedback while we decide the turn intent.
    let early_typing_cancellation = if will_run_classifiers {
        target_channel.as_ref().map(|_| CancellationToken::new())
    } else {
        None
    };
    let early_typing_task = match (target_channel.as_ref(), early_typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(spawn_scoped_typing_task(
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        )),
        _ => None,
    };

    // Send an early draft message so the user sees visible text feedback
    // before the classifier LLM calls (which can take 30-60s).
    let early_draft_message_id = if will_run_classifiers {
        if let Some(channel) = target_channel.as_ref() {
            match channel
                .send_draft(
                    &SendMessage::new("Analyzing the request...\n", &msg.reply_target)
                        .in_thread(msg.thread_ts.clone()),
                )
                .await
            {
                Ok(id) => id,
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    if !options.resume_existing_user_turn
        && ctx.auto_save_memory
        && msg.content.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS
    {
        let autosave_key = conversation_memory_key(&msg);
        let _ = ctx
            .memory
            .store(
                &autosave_key,
                &msg.content,
                crate::memory::MemoryCategory::Conversation,
                None,
            )
            .await;
    }

    println!("  \u{23F3} Processing message...");
    let started_at = Instant::now();

    let mut lossless_context = match LosslessContext::for_session(
        ctx.workspace_dir.as_path(),
        "channel",
        &history_key,
        ctx.system_prompt.as_str(),
    ) {
        Ok(context) => context,
        Err(err) => {
            tracing::warn!(
                channel = %msg.channel,
                sender = %msg.sender,
                "failed to initialize lossless context: {err}"
            );
            match LosslessContext::new(&std::env::temp_dir(), ctx.system_prompt.as_str()) {
                Ok(context) => context,
                Err(temp_err) => {
                    tracing::warn!(
                        channel = %msg.channel,
                        sender = %msg.sender,
                        "failed to initialize temporary lossless context fallback: {temp_err}"
                    );
                    return;
                }
            }
        }
    };
    let had_prior_history = lossless_context.has_non_system_messages().unwrap_or(false);

    let expose_internal_tool_details =
        msg.channel == "cli" || should_expose_internal_tool_details(&msg.content);

    if msg.channel != "cli" && !skip_pre_tool_turn_routing {
        if will_run_classifiers {
            if let Some(plan) = try_llm_capability_recovery_plan(
                active_provider.as_ref(),
                ctx.as_ref(),
                &msg,
                route.model.as_str(),
                runtime_defaults.temperature,
                &excluded_tools_snapshot,
            )
            .await
            {
                if matches!(plan.state, CapabilityState::NeedsApproval) {
                    if let Err(err) = lossless_context
                        .record_raw_message(&ChatMessage::user(&timestamped_content))
                    {
                        tracing::warn!(
                            channel = %msg.channel,
                            sender = %msg.sender,
                            "failed to persist blocked channel user turn before approval prompt: {err}"
                        );
                    }
                    append_sender_turn(
                        ctx.as_ref(),
                        &history_key,
                        ChatMessage::user(&timestamped_content),
                    );
                }
                dispatch_capability_recovery(
                    &plan,
                    &msg,
                    target_channel.as_ref(),
                    Some("llm_classifier"),
                )
                .await;
                if let Some(token) = early_typing_cancellation.as_ref() {
                    token.cancel();
                }
                if let (Some(channel), Some(ref draft_id)) =
                    (target_channel.as_ref(), &early_draft_message_id)
                {
                    let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
                }
                return;
            }
        }

        if let Some(plan) =
            infer_capability_recovery_plan(ctx.as_ref(), &msg, &excluded_tools_snapshot)
        {
            if matches!(plan.state, CapabilityState::NeedsApproval) {
                if let Err(err) =
                    lossless_context.record_raw_message(&ChatMessage::user(&timestamped_content))
                {
                    tracing::warn!(
                        channel = %msg.channel,
                        sender = %msg.sender,
                        "failed to persist blocked channel user turn before approval prompt: {err}"
                    );
                }
                append_sender_turn(
                    ctx.as_ref(),
                    &history_key,
                    ChatMessage::user(&timestamped_content),
                );
            }
            dispatch_capability_recovery(&plan, &msg, target_channel.as_ref(), None).await;
            if let Some(token) = early_typing_cancellation.as_ref() {
                token.cancel();
            }
            if let (Some(channel), Some(ref draft_id)) =
                (target_channel.as_ref(), &early_draft_message_id)
            {
                let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
            }
            return;
        }
    }

    let mut system_prompt = build_channel_system_prompt(
        ctx.system_prompt.as_str(),
        &msg.channel,
        &msg.reply_target,
        expose_internal_tool_details,
    );
    let visible_tool_names: Vec<&str> = visible_tool_names
        .iter()
        .map(std::string::String::as_str)
        .collect();
    if let Some(turn_hint) = build_current_turn_routing_hint(&msg.content, &visible_tool_names) {
        system_prompt.push_str("\n\n## Turn Routing Hint\n\n");
        system_prompt.push_str(&turn_hint);
        system_prompt.push('\n');
    }
    system_prompt.push_str(&build_runtime_tool_visibility_prompt(
        effective_tools_registry,
        &excluded_tools_snapshot,
        active_provider.supports_native_tools(),
    ));
    let canary_guard = crate::security::CanaryGuard::new(canary_enabled_for_turn);
    let (system_prompt, turn_canary_token) = canary_guard.inject_turn_token(&system_prompt);
    if !options.resume_existing_user_turn {
        if let Err(err) =
            lossless_context.record_raw_message(&ChatMessage::user(&timestamped_content))
        {
            tracing::warn!(
                channel = %msg.channel,
                sender = %msg.sender,
                "failed to persist channel user turn into lossless context: {err}"
            );
        }
    }
    let mut history = match lossless_context
        .rebuild_active_history(
            active_provider.as_ref(),
            route.model.as_str(),
            &system_prompt,
            MAX_CHANNEL_HISTORY,
        )
        .await
    {
        Ok(history) => history,
        Err(err) => {
            tracing::warn!(
                channel = %msg.channel,
                sender = %msg.sender,
                "failed to rebuild lossless channel history: {err}"
            );
            let mut fallback = vec![ChatMessage::system(system_prompt.clone())];
            fallback.extend(
                ctx.conversation_histories
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&history_key)
                    .cloned()
                    .unwrap_or_default(),
            );
            if !options.resume_existing_user_turn {
                fallback.push(ChatMessage::user(&timestamped_content));
            }
            fallback
        }
    };

    if !had_prior_history {
        let memory_context =
            build_memory_context(ctx.memory.as_ref(), &msg.content, ctx.min_relevance_score).await;
        if let Some(last_turn) = history.last_mut() {
            if last_turn.role == "user" && !memory_context.is_empty() {
                last_turn.content = format!("{memory_context}{timestamped_content}");
            }
        }
    }
    let use_streaming = target_channel
        .as_ref()
        .is_some_and(|ch| ch.supports_draft_updates());

    tracing::debug!(
        channel = %msg.channel,
        has_target_channel = target_channel.is_some(),
        use_streaming,
        supports_draft = target_channel.as_ref().map_or(false, |ch| ch.supports_draft_updates()),
        "Draft streaming decision"
    );

    let (delta_tx, delta_rx) = if use_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let draft_message_id = Arc::new(tokio::sync::Mutex::new(None::<String>));

    let draft_updater = if let (Some(mut rx), Some(channel_ref)) =
        (delta_rx, target_channel.as_ref())
    {
        let channel = Arc::clone(channel_ref);
        let reply_target = msg.reply_target.clone();
        let thread_ts = msg.thread_ts.clone();
        let draft_message_id = Arc::clone(&draft_message_id);
        let suppress_internal_progress = !expose_internal_tool_details;
        Some(tokio::spawn(async move {
            let mut accumulated = String::new();
            let mut internal_progress_lines = VecDeque::<String>::new();
            let mut last_sanitized_progress: Option<String> = None;
            let mut last_meaningful_progress: Option<String> = None;
            while let Some(delta) = rx.recv().await {
                if delta == crate::agent::loop_::DRAFT_CLEAR_SENTINEL {
                    accumulated.clear();
                    internal_progress_lines.clear();
                    last_sanitized_progress = None;
                    last_meaningful_progress = None;
                    continue;
                }
                let (is_internal_progress, visible_delta) = split_internal_progress_delta(&delta);
                if suppress_internal_progress && is_internal_progress {
                    let Some(summary) = summarize_internal_progress_delta(visible_delta) else {
                        continue;
                    };
                    let summary = contextualize_progress_heartbeat(
                        &summary,
                        last_meaningful_progress.as_deref(),
                    );
                    if last_sanitized_progress.as_deref() == Some(summary.as_str()) {
                        continue;
                    }
                    if !is_generic_progress_heartbeat(&summary) {
                        last_meaningful_progress = Some(summary.clone());
                    }
                    internal_progress_lines.push_back(summary.clone());
                    while internal_progress_lines.len() > INTERNAL_PROGRESS_MAX_LINES {
                        internal_progress_lines.pop_front();
                    }
                    accumulated = internal_progress_lines.iter().cloned().collect::<String>();
                    last_sanitized_progress = Some(summary);
                } else {
                    accumulated.push_str(visible_delta);
                    internal_progress_lines.clear();
                    last_sanitized_progress = None;
                }

                // Strip <think>…</think> blocks so reasoning models
                // (e.g. MiniMax M2.7) don't leak chain-of-thought to the user.
                // Also suppress everything after an unclosed <think> tag
                // (the closing tag hasn't streamed in yet).
                let visible = strip_think_blocks_streaming(&accumulated);
                if visible.is_empty() {
                    continue;
                }

                let current_draft_id = {
                    let guard = draft_message_id.lock().await;
                    guard.clone()
                };

                if let Some(draft_id) = current_draft_id {
                    match channel
                        .update_draft(&reply_target, &draft_id, &visible)
                        .await
                    {
                        Ok(Some(new_id)) => {
                            let mut guard = draft_message_id.lock().await;
                            *guard = Some(new_id);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::debug!("Draft update failed: {e}");
                        }
                    }
                    continue;
                }

                match channel
                    .send_draft(
                        &SendMessage::new(&visible, &reply_target).in_thread(thread_ts.clone()),
                    )
                    .await
                {
                    Ok(Some(new_id)) => {
                        let mut guard = draft_message_id.lock().await;
                        *guard = Some(new_id);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!("Failed to send draft on {}: {e}", channel.name());
                    }
                }
            }
        }))
    } else {
        None
    };

    // Stop the early typing indicator and cancel the early "Analyzing..."
    // draft message. The main tool loop will produce its own draft updates.
    if let Some(token) = early_typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = early_typing_task {
        let _ = handle.await;
    }
    if let (Some(channel), Some(ref draft_id)) = (target_channel.as_ref(), &early_draft_message_id)
    {
        let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
    }

    let typing_cancellation = target_channel.as_ref().map(|_| CancellationToken::new());
    let typing_task = match (target_channel.as_ref(), typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(spawn_scoped_typing_task(
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        )),
        _ => None,
    };
    let progress_heartbeat_cancellation = delta_tx.as_ref().map(|_| CancellationToken::new());
    let progress_heartbeat_task =
        match (delta_tx.as_ref(), progress_heartbeat_cancellation.as_ref()) {
            (Some(tx), Some(token)) => Some(spawn_progress_heartbeat_task(
                tx.clone(),
                token.clone(),
                started_at,
            )),
            _ => None,
        };

    // Record history length before tool loop so we can extract tool context after.
    let history_len_before_tools = history.len();

    enum LlmExecutionResult {
        Completed(Result<Result<String, anyhow::Error>, tokio::time::error::Elapsed>),
        Cancelled,
    }

    let timeout_budget_secs =
        channel_message_timeout_budget_secs(ctx.message_timeout_secs, ctx.max_tool_iterations);
    let (approval_prompt_tx, mut approval_prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::agent::loop_::NonCliApprovalPrompt>();
    let approval_prompt_task = if msg.channel == "cli" {
        None
    } else if let Some(channel_ref) = target_channel.as_ref() {
        let channel = Arc::clone(channel_ref);
        let reply_target = msg.reply_target.clone();
        let thread_ts = msg.thread_ts.clone();
        Some(tokio::spawn(async move {
            while let Some(prompt) = approval_prompt_rx.recv().await {
                if let Err(err) = channel
                    .send_approval_prompt(
                        &reply_target,
                        &prompt.request_id,
                        &prompt.title,
                        &prompt.details,
                        thread_ts.clone(),
                    )
                    .await
                {
                    tracing::warn!(
                        channel = %channel.name(),
                        request_id = %prompt.request_id,
                        "Failed to send approval prompt: {err}"
                    );
                }
            }
        }))
    } else {
        None
    };
    let non_cli_approval_context = if msg.channel == "cli" || target_channel.is_none() {
        None
    } else {
        Some(NonCliApprovalContext {
            sender: msg.sender.clone(),
            reply_target: msg.reply_target.clone(),
            message_id: msg.id.clone(),
            content: msg.content.clone(),
            timestamp: msg.timestamp,
            thread_ts: msg.thread_ts.clone(),
            prompt_tx: approval_prompt_tx.clone(),
        })
    };
    let runtime_context = ToolChannelRuntimeContext {
        channel: msg.channel.clone(),
        reply_target: msg.reply_target.clone(),
        thread_ts: msg.thread_ts.clone(),
        sender: msg.sender.clone(),
        message_id: msg.id.clone(),
    };
    let llm_result = tokio::select! {
        () = cancellation_token.cancelled() => LlmExecutionResult::Cancelled,
        result = tokio::time::timeout(
            Duration::from_secs(timeout_budget_secs),
            with_channel_runtime_context(
                runtime_context,
                run_tool_call_loop_with_non_cli_approval_context(
                    active_provider.as_ref(),
                    &mut history,
                    effective_tools_registry,
                    ctx.observer.as_ref(),
                    route.provider.as_str(),
                    route.model.as_str(),
                    runtime_defaults.temperature,
                    true,
                    Some(ctx.approval_manager.as_ref()),
                    msg.channel.as_str(),
                    non_cli_approval_context,
                    &ctx.multimodal,
                    ctx.max_tool_iterations,
                    Some(cancellation_token.clone()),
                    delta_tx,
                    ctx.hooks.as_deref(),
                    &excluded_tools_snapshot,
                ),
            ),
        ) => LlmExecutionResult::Completed(result),
    };

    drop(approval_prompt_tx);
    if let Some(handle) = approval_prompt_task {
        log_worker_join_result(handle.await);
    }

    if let Some(token) = progress_heartbeat_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = progress_heartbeat_task {
        log_worker_join_result(handle.await);
    }

    if let Some(handle) = draft_updater {
        let _ = handle.await;
    }

    let draft_message_id = draft_message_id.lock().await.clone();

    if let Some(token) = typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = typing_task {
        log_worker_join_result(handle.await);
    }

    let reaction_done_emoji = match &llm_result {
        LlmExecutionResult::Completed(Ok(Ok(_))) => "\u{2705}", // check mark
        _ => "\u{26A0}\u{FE0F}",                                // warning
    };

    match llm_result {
        LlmExecutionResult::Cancelled => {
            tracing::info!(
                channel = %msg.channel,
                sender = %msg.sender,
                "Cancelled in-flight channel request due to newer message"
            );
            runtime_trace::record_event(
                "channel_message_cancelled",
                Some(msg.channel.as_str()),
                Some(route.provider.as_str()),
                Some(route.model.as_str()),
                None,
                Some(false),
                Some("cancelled due to newer inbound message"),
                serde_json::json!({
                    "sender": msg.sender,
                    "elapsed_ms": started_at.elapsed().as_millis(),
                }),
            );
            if let (Some(channel), Some(draft_id)) =
                (target_channel.as_ref(), draft_message_id.as_deref())
            {
                if let Err(err) = channel.cancel_draft(&msg.reply_target, draft_id).await {
                    tracing::debug!("Failed to cancel draft on {}: {err}", channel.name());
                }
            }
        }
        LlmExecutionResult::Completed(Ok(Ok(response))) => {
            // -- Hook: on_message_sending (modifying) --
            let mut outbound_response = response;
            if canary_guard
                .response_contains_canary(&outbound_response, turn_canary_token.as_deref())
            {
                runtime_trace::record_event(
                    "channel_message_blocked_canary_guard",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("blocked response containing per-turn canary token"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "message_id": msg.id,
                    }),
                );
                outbound_response = "I blocked that response because it attempted to reveal protected internal context.".to_string();
            }
            if let Some(hooks) = &ctx.hooks {
                match hooks
                    .run_on_message_sending(
                        msg.channel.clone(),
                        msg.reply_target.clone(),
                        outbound_response.clone(),
                    )
                    .await
                {
                    crate::hooks::HookResult::Cancel(reason) => {
                        tracing::info!(%reason, "outgoing message suppressed by hook");
                        return;
                    }
                    crate::hooks::HookResult::Continue((
                        hook_channel,
                        hook_recipient,
                        mut modified_content,
                    )) => {
                        if hook_channel != msg.channel || hook_recipient != msg.reply_target {
                            tracing::warn!(
                                from_channel = %msg.channel,
                                from_recipient = %msg.reply_target,
                                to_channel = %hook_channel,
                                to_recipient = %hook_recipient,
                                "on_message_sending attempted to rewrite channel routing; only content mutation is applied"
                            );
                        }

                        let modified_len = modified_content.chars().count();
                        if modified_len > CHANNEL_HOOK_MAX_OUTBOUND_CHARS {
                            tracing::warn!(
                                limit = CHANNEL_HOOK_MAX_OUTBOUND_CHARS,
                                attempted = modified_len,
                                "hook-modified outbound content exceeded limit; truncating"
                            );
                            modified_content = truncate_with_ellipsis(
                                &modified_content,
                                CHANNEL_HOOK_MAX_OUTBOUND_CHARS,
                            );
                        }

                        if modified_content != outbound_response {
                            tracing::info!(
                                channel = %msg.channel,
                                sender = %msg.sender,
                                before_len = outbound_response.chars().count(),
                                after_len = modified_content.chars().count(),
                                "outgoing message content modified by hook"
                            );
                        }

                        outbound_response = modified_content;
                    }
                }
            }

            let sanitized_response =
                sanitize_channel_response(&outbound_response, ctx.tools_registry.as_ref());
            let delivered_response = if sanitized_response.is_empty()
                && !outbound_response.trim().is_empty()
            {
                "I encountered malformed tool-call output and could not produce a safe reply. Please try again.".to_string()
            } else {
                sanitized_response
            };
            let suppress_noop_response = is_agent_noop_sentinel(&delivered_response);
            let history_delta = if suppress_noop_response {
                filter_noop_assistant_turns(history[history_len_before_tools..].to_vec())
            } else {
                history[history_len_before_tools..].to_vec()
            };
            if let Err(err) = lossless_context.record_raw_messages(&history_delta) {
                tracing::warn!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    "failed to persist post-tool channel history: {err}"
                );
            }
            if suppress_noop_response {
                tracing::debug!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    response = %truncate_with_ellipsis(&delivered_response, 64),
                    "Suppressing noop sentinel response in channel flow"
                );
                runtime_trace::record_event(
                    "channel_message_outbound_suppressed_noop",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(true),
                    Some("suppressed noop sentinel"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "response": scrub_credentials(&delivered_response),
                    }),
                );
                if let (Some(channel), Some(draft_id)) =
                    (target_channel.as_ref(), draft_message_id.as_deref())
                {
                    let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
                }
            } else {
                runtime_trace::record_event(
                    "channel_message_outbound",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(true),
                    None,
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "response": scrub_credentials(&delivered_response),
                    }),
                );

                // Extract condensed tool-use context from the history messages
                // added during run_tool_call_loop, so the LLM retains awareness
                // of what it did on subsequent turns.
                let tool_summary = extract_tool_context_summary(&history, history_len_before_tools);
                let history_response = if tool_summary.is_empty() || msg.channel == "telegram" {
                    delivered_response.clone()
                } else {
                    format!("{tool_summary}\n{delivered_response}")
                };

                history.push(ChatMessage::assistant(&history_response));
                println!(
                    "  \u{1F916} Reply ({}ms): {}",
                    started_at.elapsed().as_millis(),
                    truncate_with_ellipsis(&delivered_response, 80)
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        if let Err(e) = channel
                            .finalize_draft(&msg.reply_target, draft_id, &delivered_response)
                            .await
                        {
                            tracing::warn!("Failed to finalize draft: {e}; sending as new message");
                            let _ = channel
                                .send(
                                    &SendMessage::new(&delivered_response, &msg.reply_target)
                                        .in_thread(msg.thread_ts.clone()),
                                )
                                .await;
                        }
                    } else if let Err(e) = channel
                        .send(
                            &SendMessage::new(delivered_response, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await
                    {
                        eprintln!("  \u{274C} Failed to reply on {}: {e}", channel.name());
                    }
                }
            }
            match lossless_context
                .rebuild_active_history(
                    active_provider.as_ref(),
                    route.model.as_str(),
                    &system_prompt,
                    MAX_CHANNEL_HISTORY,
                )
                .await
            {
                Ok(active_history) => {
                    set_sender_history(
                        ctx.as_ref(),
                        &history_key,
                        filter_noop_assistant_turns(active_history.into_iter().skip(1).collect()),
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        channel = %msg.channel,
                        sender = %msg.sender,
                        "failed to refresh channel lossless history cache: {err}"
                    );
                    set_sender_history(
                        ctx.as_ref(),
                        &history_key,
                        normalize_cached_channel_turns(filter_noop_assistant_turns(
                            history.into_iter().skip(1).collect(),
                        )),
                    );
                }
            }
        }
        LlmExecutionResult::Completed(Ok(Err(e))) => {
            if let Err(err) =
                lossless_context.record_raw_messages(&history[history_len_before_tools..])
            {
                tracing::warn!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    "failed to persist partial channel history after error: {err}"
                );
            }
            if crate::agent::loop_::is_tool_loop_cancelled(&e) || cancellation_token.is_cancelled()
            {
                tracing::info!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    "Cancelled in-flight channel request due to newer message"
                );
                runtime_trace::record_event(
                    "channel_message_cancelled",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("cancelled during tool-call loop"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                    }),
                );
                if let (Some(channel), Some(draft_id)) =
                    (target_channel.as_ref(), draft_message_id.as_deref())
                {
                    if let Err(err) = channel.cancel_draft(&msg.reply_target, draft_id).await {
                        tracing::debug!("Failed to cancel draft on {}: {err}", channel.name());
                    }
                }
            } else if let Some(pending) = is_non_cli_approval_pending(&e) {
                runtime_trace::record_event(
                    "channel_message_approval_pending",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("paused awaiting non-cli approval"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "request_id": pending.request_id,
                        "tool_name": pending.tool_name,
                    }),
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel.cancel_draft(&msg.reply_target, draft_id).await;
                    }
                }
            } else if is_context_window_overflow_error(&e) {
                let compacted = match lossless_context
                    .rebuild_active_history(
                        active_provider.as_ref(),
                        route.model.as_str(),
                        &system_prompt,
                        MAX_CHANNEL_HISTORY,
                    )
                    .await
                {
                    Ok(active_history) => {
                        set_sender_history(
                            ctx.as_ref(),
                            &history_key,
                            active_history.into_iter().skip(1).collect(),
                        );
                        true
                    }
                    Err(err) => {
                        tracing::warn!(
                            channel = %msg.channel,
                            sender = %msg.sender,
                            "failed to rebuild channel history after overflow: {err}"
                        );
                        compact_sender_history(ctx.as_ref(), &history_key)
                    }
                };
                let error_text = "\u{26A0}\u{FE0F} Context window exceeded for this conversation. Older turns were compacted into lossless summaries and the latest context was preserved. Please resend your last message.";
                eprintln!(
                    "  \u{26A0}\u{FE0F} Context window exceeded after {}ms; sender history compacted={}",
                    started_at.elapsed().as_millis(),
                    compacted
                );
                runtime_trace::record_event(
                    "channel_message_error",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("context window exceeded"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "history_compacted": compacted,
                    }),
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(&msg.reply_target, draft_id, error_text)
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(error_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }
            } else if is_tool_iteration_limit_error(&e) {
                let limit = ctx.max_tool_iterations.max(1);
                let pause_text = format!(
                    "\u{26A0}\u{FE0F} Reached tool-iteration limit ({limit}) for this turn. Context and progress were preserved. Reply \"continue\" to resume, or increase `agent.max_tool_iterations`."
                );
                runtime_trace::record_event(
                    "channel_message_error",
                    Some(msg.channel.as_str()),
                    Some(route.provider.as_str()),
                    Some(route.model.as_str()),
                    None,
                    Some(false),
                    Some("tool iteration limit reached"),
                    serde_json::json!({
                        "sender": msg.sender,
                        "elapsed_ms": started_at.elapsed().as_millis(),
                        "max_tool_iterations": limit,
                    }),
                );
                append_sender_turn(
                    ctx.as_ref(),
                    &history_key,
                    ChatMessage::assistant(
                        "[Task paused at tool-iteration limit \u{2014} context preserved. Ask to continue.]",
                    ),
                );
                let _ = lossless_context.record_raw_message(&ChatMessage::assistant(
                    "[Task paused at tool-iteration limit \u{2014} context preserved. Ask to continue.]",
                ));
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(&msg.reply_target, draft_id, &pause_text)
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(pause_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }
            } else {
                eprintln!(
                    "  \u{274C} LLM error after {}ms: {e}",
                    started_at.elapsed().as_millis()
                );
                if let Some(local_response) = local_capability_response.as_ref() {
                    runtime_trace::record_event(
                        "channel_message_local_capability_response",
                        Some(msg.channel.as_str()),
                        Some(route.provider.as_str()),
                        Some(route.model.as_str()),
                        None,
                        Some(true),
                        Some("answered capability question locally after provider failure"),
                        serde_json::json!({
                            "sender": msg.sender,
                            "elapsed_ms": started_at.elapsed().as_millis(),
                            "message_id": msg.id,
                        }),
                    );

                    history.push(ChatMessage::assistant(local_response));
                    let _ = lossless_context
                        .record_raw_message(&ChatMessage::assistant(local_response));
                    if let Ok(active_history) = lossless_context
                        .rebuild_active_history(
                            active_provider.as_ref(),
                            route.model.as_str(),
                            &system_prompt,
                            MAX_CHANNEL_HISTORY,
                        )
                        .await
                    {
                        set_sender_history(
                            ctx.as_ref(),
                            &history_key,
                            active_history.into_iter().skip(1).collect(),
                        );
                    } else {
                        set_sender_history(
                            ctx.as_ref(),
                            &history_key,
                            normalize_cached_channel_turns(history.into_iter().skip(1).collect()),
                        );
                    }
                    if let Some(channel) = target_channel.as_ref() {
                        if let Some(ref draft_id) = draft_message_id {
                            let _ = channel
                                .finalize_draft(&msg.reply_target, draft_id, local_response)
                                .await;
                        } else {
                            let _ = channel
                                .send(
                                    &SendMessage::new(local_response, &msg.reply_target)
                                        .in_thread(msg.thread_ts.clone()),
                                )
                                .await;
                        }
                    }
                } else {
                    let safe_error = providers::sanitize_api_error(&e.to_string());
                    let user_visible_error =
                        format_user_visible_llm_error(msg.channel.as_str(), &e);
                    runtime_trace::record_event(
                        "channel_message_error",
                        Some(msg.channel.as_str()),
                        Some(route.provider.as_str()),
                        Some(route.model.as_str()),
                        None,
                        Some(false),
                        Some(&safe_error),
                        serde_json::json!({
                            "sender": msg.sender,
                            "elapsed_ms": started_at.elapsed().as_millis(),
                        }),
                    );
                    let should_rollback_user_turn = e
                        .downcast_ref::<providers::ProviderCapabilityError>()
                        .is_some_and(|capability| {
                            capability.capability.eq_ignore_ascii_case("vision")
                        });
                    let rolled_back = should_rollback_user_turn
                        && rollback_orphan_user_turn(
                            ctx.as_ref(),
                            &history_key,
                            &timestamped_content,
                        );
                    let _ = if should_rollback_user_turn {
                        lossless_context.rollback_latest_raw_message("user", &timestamped_content)
                    } else {
                        Ok(false)
                    };

                    if !rolled_back {
                        // Close the orphan user turn so subsequent messages don't
                        // inherit this failed request as unfinished context.
                        append_sender_turn(
                            ctx.as_ref(),
                            &history_key,
                            ChatMessage::assistant(
                                "[Task failed \u{2014} not continuing this request]",
                            ),
                        );
                        let _ = lossless_context.record_raw_message(&ChatMessage::assistant(
                            "[Task failed \u{2014} not continuing this request]",
                        ));
                    }
                    if let Ok(active_history) = lossless_context
                        .rebuild_active_history(
                            active_provider.as_ref(),
                            route.model.as_str(),
                            &system_prompt,
                            MAX_CHANNEL_HISTORY,
                        )
                        .await
                    {
                        set_sender_history(
                            ctx.as_ref(),
                            &history_key,
                            active_history.into_iter().skip(1).collect(),
                        );
                    }
                    if let Some(channel) = target_channel.as_ref() {
                        if let Some(ref draft_id) = draft_message_id {
                            let _ = channel
                                .finalize_draft(&msg.reply_target, draft_id, &user_visible_error)
                                .await;
                        } else {
                            let _ = channel
                                .send(
                                    &SendMessage::new(&user_visible_error, &msg.reply_target)
                                        .in_thread(msg.thread_ts.clone()),
                                )
                                .await;
                        }
                    }
                }
            }
        }
        LlmExecutionResult::Completed(Err(_)) => {
            let timeout_msg = format!(
                "LLM response timed out after {}s (base={}s, max_tool_iterations={})",
                timeout_budget_secs, ctx.message_timeout_secs, ctx.max_tool_iterations
            );
            runtime_trace::record_event(
                "channel_message_timeout",
                Some(msg.channel.as_str()),
                Some(route.provider.as_str()),
                Some(route.model.as_str()),
                None,
                Some(false),
                Some(&timeout_msg),
                serde_json::json!({
                    "sender": msg.sender,
                    "elapsed_ms": started_at.elapsed().as_millis(),
                }),
            );
            eprintln!(
                "  \u{274C} {} (elapsed: {}ms)",
                timeout_msg,
                started_at.elapsed().as_millis()
            );
            // Close the orphan user turn so subsequent messages don't
            // inherit this timed-out request as unfinished context.
            append_sender_turn(
                ctx.as_ref(),
                &history_key,
                ChatMessage::assistant("[Task timed out \u{2014} not continuing this request]"),
            );
            let _ = lossless_context.record_raw_message(&ChatMessage::assistant(
                "[Task timed out \u{2014} not continuing this request]",
            ));
            if let Ok(active_history) = lossless_context
                .rebuild_active_history(
                    active_provider.as_ref(),
                    route.model.as_str(),
                    &system_prompt,
                    MAX_CHANNEL_HISTORY,
                )
                .await
            {
                set_sender_history(
                    ctx.as_ref(),
                    &history_key,
                    active_history.into_iter().skip(1).collect(),
                );
            }
            if let Some(channel) = target_channel.as_ref() {
                let error_text =
                    "\u{26A0}\u{FE0F} Request timed out while waiting for the model. Please try again.";
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(&msg.reply_target, draft_id, error_text)
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(error_text, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
        }
    }

    // Swap eyes -> check (or warning on error) to signal processing is complete
    if let Some(channel) = target_channel.as_ref() {
        let _ = channel
            .remove_reaction(&msg.reply_target, &msg.id, "\u{1F440}")
            .await;
        let _ = channel
            .add_reaction(&msg.reply_target, &msg.id, reaction_done_emoji)
            .await;
    }
}

/// Strip `<think>…</think>` blocks from streaming accumulated text.
///
/// Handles three cases:
/// 1. Complete `<think>…</think>` blocks — removed entirely.
/// 2. Unclosed `<think>` at the end — everything from the opening tag onward
///    is suppressed (the closing tag hasn't streamed in yet).
/// 3. No think tags — returned as-is.
fn strip_think_blocks_streaming(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                // Unclosed <think> — suppress the rest (still streaming).
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::should_run_llm_capability_classifier;

    #[test]
    fn llm_capability_classifier_skips_normal_work_turns() {
        assert!(!should_run_llm_capability_classifier(
            false,
            "telegram",
            true,
            "What is the BTC price now?"
        ));
        assert!(!should_run_llm_capability_classifier(
            false,
            "telegram",
            true,
            "How many lines of code does this repo have? https://github.com/topway-ai/topclaw"
        ));
    }

    #[test]
    fn llm_capability_classifier_runs_for_capability_questions() {
        assert!(should_run_llm_capability_classifier(
            false,
            "telegram",
            true,
            "why can't you use that desktop skill?"
        ));
        assert!(!should_run_llm_capability_classifier(
            false,
            "cli",
            true,
            "why can't you use that desktop skill?"
        ));
    }
}
