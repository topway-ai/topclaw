//! Core message processing pipeline for channel messages.
//!
//! Contains the main `process_channel_message` function that orchestrates
//! receiving a message, running the LLM tool-call loop, and delivering
//! the response back to the channel.

use crate::agent::loop_::{
    is_non_cli_approval_pending, lossless::LosslessContext,
    run_tool_call_loop_with_non_cli_approval_context, scrub_credentials, NonCliApprovalContext,
};
use crate::config::Config;
use crate::observability::runtime_trace;
use crate::providers::{self, ChatMessage};
use crate::tools::channel_runtime_context::{
    with_channel_runtime_context, ChannelRuntimeContext as ToolChannelRuntimeContext,
};
use crate::util::truncate_with_ellipsis;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use super::capability_recovery::{
    infer_capability_recovery_plan, should_expose_internal_tool_details,
    try_llm_capability_recovery_plan, CapabilityRecoveryPlan, CapabilityState,
};
use super::command_handler::handle_runtime_command_if_needed;
use super::context::*;
use super::dispatch::{spawn_progress_heartbeat_task, spawn_scoped_typing_task};
use super::helpers::*;
use super::prompt::build_channel_system_prompt;
use super::route_state::{
    append_sender_turn, compact_sender_history, get_route_selection, rollback_orphan_user_turn,
    set_sender_history,
};
use super::runtime_config::{
    maybe_apply_runtime_config_update, runtime_config_path, runtime_defaults_snapshot,
};
use super::runtime_helpers::{
    build_runtime_tool_visibility_prompt, snapshot_non_cli_excluded_tools,
};
use super::sanitize::sanitize_channel_response;
use super::traits::{self, SendMessage};

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
        let _ = channel
            .send(
                &SendMessage::new(&plan.message, &msg.reply_target)
                    .in_thread(msg.thread_ts.clone()),
            )
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
                            cfg.security.semantic_guard_collection,
                            cfg.security.semantic_guard_threshold,
                            cfg.memory,
                            cfg.api_key,
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

        if let Some((
            canary_enabled,
            semantic_enabled,
            semantic_collection,
            semantic_threshold,
            memory_cfg,
            api_key,
        )) = semantic_cfg
        {
            canary_enabled_for_turn = canary_enabled;
            if semantic_enabled {
                let semantic_guard = crate::security::SemanticGuard::from_config(
                    &memory_cfg,
                    semantic_enabled,
                    semantic_collection.as_str(),
                    semantic_threshold,
                    api_key.as_deref(),
                );
                if let Some(detection) = semantic_guard.detect(&msg.content).await {
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
    let runtime_defaults = runtime_defaults_snapshot(ctx.as_ref());
    let active_provider = match get_or_create_provider(ctx.as_ref(), &route.provider).await {
        Ok(provider) => provider,
        Err(err) => {
            let safe_err = providers::sanitize_api_error(&err.to_string());
            let message = format!(
                "\u{26A0}\u{FE0F} Failed to initialize provider `{}`. Please run `/models` to choose another provider.\nDetails: {safe_err}",
                route.provider
            );
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

    // Inject per-message timestamp so the LLM always knows the current time,
    // even in multi-turn conversations where the system prompt may be stale.
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let timestamped_content = format!("[{now}] {}", msg.content);

    let expose_internal_tool_details =
        msg.channel == "cli" || should_expose_internal_tool_details(&msg.content);
    let excluded_tools_snapshot = if msg.channel == "cli" {
        Vec::new()
    } else {
        snapshot_non_cli_excluded_tools(ctx.as_ref())
    };

    if msg.channel != "cli" {
        if let Some(plan) =
            infer_capability_recovery_plan(ctx.as_ref(), &msg, &excluded_tools_snapshot)
        {
            dispatch_capability_recovery(&plan, &msg, target_channel.as_ref(), None).await;
            return;
        }

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
            dispatch_capability_recovery(
                &plan,
                &msg,
                target_channel.as_ref(),
                Some("llm_classifier"),
            )
            .await;
            return;
        }
    }

    let mut system_prompt = build_channel_system_prompt(
        ctx.system_prompt.as_str(),
        &msg.channel,
        &msg.reply_target,
        expose_internal_tool_details,
    );
    system_prompt.push_str(&build_runtime_tool_visibility_prompt(
        ctx.tools_registry.as_ref(),
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
            let mut last_sanitized_progress: Option<String> = None;
            let mut last_meaningful_progress: Option<String> = None;
            while let Some(delta) = rx.recv().await {
                if delta == crate::agent::loop_::DRAFT_CLEAR_SENTINEL {
                    accumulated.clear();
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
                    accumulated.push_str(&summary);
                    last_sanitized_progress = Some(summary);
                } else {
                    accumulated.push_str(visible_delta);
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

    // React with eyes to acknowledge the incoming message
    if let Some(channel) = target_channel.as_ref() {
        if let Err(e) = channel
            .add_reaction(&msg.reply_target, &msg.id, "\u{1F440}")
            .await
        {
            tracing::debug!("Failed to add reaction: {e}");
        }
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
                        &prompt.display_tool_name,
                        &prompt.arguments,
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
                    ctx.tools_registry.as_ref(),
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
                let safe_error = providers::sanitize_api_error(&e.to_string());
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
                    .is_some_and(|capability| capability.capability.eq_ignore_ascii_case("vision"));
                let rolled_back = should_rollback_user_turn
                    && rollback_orphan_user_turn(ctx.as_ref(), &history_key, &timestamped_content);
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
                            .finalize_draft(
                                &msg.reply_target,
                                draft_id,
                                &format!("\u{26A0}\u{FE0F} Error: {e}"),
                            )
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(
                                    format!("\u{26A0}\u{FE0F} Error: {e}"),
                                    &msg.reply_target,
                                )
                                .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
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
