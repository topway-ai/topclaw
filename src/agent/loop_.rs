use crate::agent::wiring;
use crate::approval::{ApprovalManager, ApprovalRequest, ApprovalResponse};
use crate::channels::APPROVAL_ALL_TOOLS_ONCE_TOKEN;
use crate::config::Config;
use crate::memory::MemoryCategory;
use crate::multimodal;
use crate::observability::{runtime_trace, Observer, ObserverEvent};
#[allow(unused_imports)]
use crate::providers::ChatRequest;
#[cfg(test)]
use crate::providers::ToolCall;
use crate::providers::{self, ChatMessage, Provider, ProviderCapabilityError};
#[cfg(test)]
use crate::tools;
use crate::tools::Tool;
use crate::util::truncate_with_ellipsis;
use anyhow::Result;
use regex::Regex;
use rustyline::error::ReadlineError;
use std::collections::{BTreeSet, HashSet};
use std::fmt::Write;
use std::io::Write as _;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

mod context;
mod execution;
mod guardrails;
mod history;
mod history_builders;
pub(crate) mod lossless;
mod parsing;
mod provider_io;
mod tool_helpers;
mod utilities;

use context::build_context;
use execution::{
    blocked_non_cli_approval_plan_reason, execute_tools_parallel, execute_tools_sequential,
    should_execute_tools_in_parallel, ToolExecutionOutcome,
};
use guardrails::{
    build_tool_unavailable_retry_prompt, looks_like_tool_unavailability_claim,
    looks_like_unverified_action_completion_without_tool_call,
};
use history::trim_history;
use history_builders::{
    build_native_assistant_history, build_native_assistant_history_from_parsed_calls,
};
use lossless::LosslessContext;
#[allow(unused_imports)]
use parsing::{
    default_param_for_tool, detect_tool_call_parse_issue, extract_json_values, map_tool_name_alias,
    parse_arguments_value, parse_glm_shortened_body, parse_glm_style_tool_calls,
    parse_perl_style_tool_calls, parse_structured_tool_calls, parse_tool_call_value,
    parse_tool_calls, parse_tool_calls_from_json_value, tool_call_signature, ParsedToolCall,
};
use provider_io::{call_provider_chat, consume_provider_streaming_response};
use tool_helpers::{
    build_non_cli_approval_plan_prompt, collect_planned_shell_commands,
    maybe_inject_cron_add_delivery, qualifies_for_non_cli_investigation_batch,
    truncate_tool_args_for_progress,
};
use utilities::autosave_memory_key;

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
const STREAM_CHUNK_MIN_CHARS: usize = 80;
/// Rolling window size for detecting streamed tool-call payload markers.
const STREAM_TOOL_MARKER_WINDOW_CHARS: usize = 512;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 100;
const MAX_CONSECUTIVE_WEB_TOOL_FAILURES: usize = 2;

/// Minimum user-message length (in chars) for auto-save to memory.
/// Matches the channel-side constant in `channels/mod.rs`.
const AUTOSAVE_MIN_MESSAGE_CHARS: usize = 20;

static SENSITIVE_KV_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(token|api[_-]?key|password|secret|user[_-]?key|bearer|credential)(["']?\s*[:=]\s*)(?:"([^"]{8,})"|'([^']{8,})'|([a-zA-Z0-9_\-\.]{8,}))"#).unwrap()
});

/// Scrub credentials from tool output to prevent accidental exfiltration.
/// Replaces known credential patterns with a redacted placeholder while preserving
/// a small prefix for context.
pub(crate) fn scrub_credentials(input: &str) -> String {
    SENSITIVE_KV_REGEX
        .replace_all(input, |caps: &regex::Captures| {
            let key = &caps[1];
            let delimiter = caps.get(2).map(|m| m.as_str()).unwrap_or(": ");
            let val = caps
                .get(3)
                .or(caps.get(4))
                .or(caps.get(5))
                .map(|m| m.as_str())
                .unwrap_or("");
            let quote = if caps.get(3).is_some() {
                "\""
            } else if caps.get(4).is_some() {
                "'"
            } else {
                ""
            };

            // Preserve first 4 chars for context, then redact.
            let prefix = if val.len() > 4 { &val[..4] } else { "" };

            format!("{key}{delimiter}{quote}{prefix}*[REDACTED]{quote}")
        })
        .to_string()
}

/// Default trigger for auto-compaction when non-system message count exceeds this threshold.
/// Prefer passing the config-driven value via `run_tool_call_loop`; this constant is only
/// used when callers omit the parameter.
/// Sentinel value sent through on_delta to signal the draft updater to clear accumulated text.
/// Used before streaming the final answer so progress lines are replaced by the clean response.
pub(crate) const DRAFT_CLEAR_SENTINEL: &str = "\x00CLEAR\x00";
/// Sentinel prefix for internal progress deltas (thinking/tool execution trace).
/// Channel layers can suppress these messages by default and only expose them
/// when the user explicitly asks for command/tool execution details.
pub(crate) const DRAFT_PROGRESS_SENTINEL: &str = "\x00PROGRESS\x00";

tokio::task_local! {
    static TOOL_LOOP_REPLY_TARGET: Option<String>;
}

const MISSING_TOOL_CALL_RETRY_PROMPT: &str = "Internal correction: your last reply implied a follow-up action or claimed action completion, but no valid tool call was emitted. If a tool is needed, emit it now using the required <tool_call>...</tool_call> format. If no tool is needed, provide the complete final answer now and do not defer action.";
#[derive(Debug, Clone)]
pub(crate) struct NonCliApprovalPrompt {
    pub request_id: String,
    pub title: String,
    pub details: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NonCliApprovalContext {
    pub sender: String,
    pub reply_target: String,
    pub message_id: String,
    pub content: String,
    pub timestamp: u64,
    pub thread_ts: Option<String>,
    pub prompt_tx: tokio::sync::mpsc::UnboundedSender<NonCliApprovalPrompt>,
}

tokio::task_local! {
    static TOOL_LOOP_NON_CLI_APPROVAL_CONTEXT: Option<NonCliApprovalContext>;
}

#[derive(Debug)]
pub(crate) struct ToolLoopCancelled;

impl std::fmt::Display for ToolLoopCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tool loop cancelled")
    }
}

impl std::error::Error for ToolLoopCancelled {}

pub(crate) fn is_tool_loop_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|source| source.is::<ToolLoopCancelled>())
}

#[derive(Debug, Clone)]
pub(crate) struct NonCliApprovalPending {
    pub request_id: String,
    pub tool_name: String,
}

impl std::fmt::Display for NonCliApprovalPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "non-cli approval pending for tool `{}` (request `{}`)",
            self.tool_name, self.request_id
        )
    }
}

impl std::error::Error for NonCliApprovalPending {}

pub(crate) fn is_non_cli_approval_pending(err: &anyhow::Error) -> Option<&NonCliApprovalPending> {
    err.chain()
        .find_map(|source| source.downcast_ref::<NonCliApprovalPending>())
}

pub(crate) fn is_tool_iteration_limit_error(err: &anyhow::Error) -> bool {
    err.chain().any(|source| {
        source
            .to_string()
            .contains("Agent exceeded maximum tool iterations")
    })
}

fn is_web_lookup_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "web_search" | "web_search_tool" | "web_fetch" | "http_request"
    )
}

fn repeated_web_tool_failure_response(failure_count: usize, last_error: Option<&str>) -> String {
    let mut response = format!(
        "I couldn't retrieve the current web data because the web tools failed {failure_count} times in this turn. I stopped instead of continuing to retry the same failing path."
    );
    if let Some(error) = last_error.map(str::trim).filter(|error| !error.is_empty()) {
        let _ = write!(
            response,
            "\nLast tool error: {}",
            truncate_with_ellipsis(error, 240)
        );
    }
    response.push_str("\nPlease retry later or check the web search/fetch configuration.");
    response
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_turn(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::config::MultimodalConfig,
    max_tool_iterations: usize,
) -> Result<String> {
    run_tool_call_loop(
        provider,
        history,
        tools_registry,
        observer,
        provider_name,
        model,
        temperature,
        silent,
        None,
        "channel",
        multimodal_config,
        max_tool_iterations,
        None,
        None,
        None,
        &[],
    )
    .await
}

/// Run the tool loop with optional non-CLI approval context scoped to this task.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop_with_non_cli_approval_context(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    approval: Option<&ApprovalManager>,
    channel_name: &str,
    non_cli_approval_context: Option<NonCliApprovalContext>,
    multimodal_config: &crate::config::MultimodalConfig,
    max_tool_iterations: usize,
    cancellation_token: Option<CancellationToken>,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    hooks: Option<&crate::hooks::HookRunner>,
    excluded_tools: &[String],
) -> Result<String> {
    let reply_target = non_cli_approval_context
        .as_ref()
        .map(|ctx| ctx.reply_target.clone());

    TOOL_LOOP_NON_CLI_APPROVAL_CONTEXT
        .scope(
            non_cli_approval_context,
            TOOL_LOOP_REPLY_TARGET.scope(
                reply_target,
                run_tool_call_loop(
                    provider,
                    history,
                    tools_registry,
                    observer,
                    provider_name,
                    model,
                    temperature,
                    silent,
                    approval,
                    channel_name,
                    multimodal_config,
                    max_tool_iterations,
                    cancellation_token,
                    on_delta,
                    hooks,
                    excluded_tools,
                ),
            ),
        )
        .await
}

// ── Agent Tool-Call Loop ──────────────────────────────────────────────────
// Core agentic iteration: send conversation to the LLM, parse any tool
// calls from the response, execute them, append results to history, and
// repeat until the LLM produces a final text-only answer.
//
// Loop invariant: at the start of each iteration, `history` contains the
// full conversation so far (system prompt + user messages + prior tool
// results). The loop exits when:
//   • the LLM returns no tool calls (final answer), or
//   • max_iterations is reached (runaway safety), or
//   • the cancellation token fires (external abort).

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    observer: &dyn Observer,
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    approval: Option<&ApprovalManager>,
    channel_name: &str,
    multimodal_config: &crate::config::MultimodalConfig,
    max_tool_iterations: usize,
    cancellation_token: Option<CancellationToken>,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    hooks: Option<&crate::hooks::HookRunner>,
    excluded_tools: &[String],
) -> Result<String> {
    let non_cli_approval_context = TOOL_LOOP_NON_CLI_APPROVAL_CONTEXT
        .try_with(Clone::clone)
        .ok()
        .flatten();
    let channel_reply_target = TOOL_LOOP_REPLY_TARGET
        .try_with(Clone::clone)
        .ok()
        .flatten()
        .or_else(|| {
            non_cli_approval_context
                .as_ref()
                .map(|ctx| ctx.reply_target.clone())
        });

    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    let excluded_set = crate::channels::runtime_helpers::exclusion_set(excluded_tools);
    let tool_specs: Vec<crate::tools::ToolSpec> = tools_registry
        .iter()
        .filter(|tool| !excluded_set.contains(&tool.name().to_ascii_lowercase()))
        .map(|tool| tool.spec())
        .collect();
    let use_native_tools = provider.supports_native_tools() && !tool_specs.is_empty();
    let turn_id = Uuid::new_v4().to_string();
    let mut seen_tool_signatures: HashSet<(String, String)> = HashSet::new();
    let mut missing_tool_call_retry_used = false;
    let mut missing_tool_call_retry_prompt: Option<String> = None;
    let mut consecutive_web_tool_failures = 0usize;
    let mut last_web_tool_error: Option<String> = None;
    let approved_turn_grant = approval.and_then(|mgr| {
        if channel_name == "cli" {
            None
        } else {
            mgr.consume_non_cli_turn_grant()
        }
    });
    let bypass_non_cli_approval_for_turn = approved_turn_grant.is_some();
    if bypass_non_cli_approval_for_turn {
        runtime_trace::record_event(
            "approval_bypass_one_time_all_tools_consumed",
            Some(channel_name),
            Some(provider_name),
            Some(model),
            Some(&turn_id),
            Some(true),
            Some("consumed one-time non-cli allow-all approval token"),
            serde_json::json!({}),
        );
    }

    for iteration in 0..max_iterations {
        if cancellation_token
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(ToolLoopCancelled.into());
        }

        if let Some(retry_prompt) = missing_tool_call_retry_prompt.take() {
            history.push(ChatMessage::user(retry_prompt));
        }

        let image_marker_count = multimodal::count_image_markers(history);
        if image_marker_count > 0 && !provider.supports_vision() {
            return Err(ProviderCapabilityError {
                provider: provider_name.to_string(),
                capability: "vision".to_string(),
                message: format!(
                    "received {image_marker_count} image marker(s), but this provider does not support vision input"
                ),
            }
            .into());
        }

        let prepared_messages =
            multimodal::prepare_messages_for_provider(history, multimodal_config).await?;

        // ── Progress: LLM thinking ────────────────────────────
        if let Some(ref tx) = on_delta {
            let phase = if iteration == 0 {
                "\u{1f914} Thinking...\n".to_string()
            } else {
                format!("\u{1f914} Thinking (round {})...\n", iteration + 1)
            };
            let _ = tx.send(format!("{DRAFT_PROGRESS_SENTINEL}{phase}")).await;
        }

        observer.record_event(&ObserverEvent::LlmRequest {
            provider: provider_name.to_string(),
            model: model.to_string(),
            messages_count: history.len(),
        });
        runtime_trace::record_event(
            "llm_request",
            Some(channel_name),
            Some(provider_name),
            Some(model),
            Some(&turn_id),
            None,
            None,
            serde_json::json!({
                "iteration": iteration + 1,
                "messages_count": history.len(),
            }),
        );

        let llm_started_at = Instant::now();

        // Fire void hook before LLM call
        if let Some(hooks) = hooks {
            hooks.fire_llm_input(history, model).await;
        }

        // Unified path via Provider::chat so provider-specific native tool logic
        // (OpenAI/Anthropic/OpenRouter/compatible adapters) is honored.
        let request_tools = if use_native_tools {
            Some(tool_specs.as_slice())
        } else {
            None
        };
        let should_consume_provider_stream = on_delta.is_some()
            && provider.supports_streaming()
            && (request_tools.is_none() || provider.supports_streaming_tool_events());
        let mut streamed_live_deltas = false;

        let chat_result = if should_consume_provider_stream {
            match consume_provider_streaming_response(
                provider,
                &prepared_messages.messages,
                request_tools,
                model,
                temperature,
                cancellation_token.as_ref(),
                on_delta.as_ref(),
            )
            .await
            {
                Ok(streamed) => {
                    streamed_live_deltas = streamed.forwarded_live_deltas;
                    Ok(crate::providers::ChatResponse {
                        text: Some(streamed.response_text),
                        tool_calls: streamed.tool_calls,
                        usage: None,
                        reasoning_content: None,
                    })
                }
                Err(stream_err) => {
                    tracing::warn!(
                        provider = provider_name,
                        model = model,
                        iteration = iteration + 1,
                        "provider streaming failed, falling back to non-streaming chat: {stream_err}"
                    );
                    runtime_trace::record_event(
                        "llm_stream_fallback",
                        Some(channel_name),
                        Some(provider_name),
                        Some(model),
                        Some(&turn_id),
                        Some(false),
                        Some("provider stream failed; fallback to non-streaming chat"),
                        serde_json::json!({
                            "iteration": iteration + 1,
                            "error": scrub_credentials(&stream_err.to_string()),
                        }),
                    );
                    if let Some(ref tx) = on_delta {
                        let _ = tx.send(DRAFT_CLEAR_SENTINEL.to_string()).await;
                    }
                    call_provider_chat(
                        provider,
                        &prepared_messages.messages,
                        request_tools,
                        model,
                        temperature,
                        cancellation_token.as_ref(),
                    )
                    .await
                }
            }
        } else {
            call_provider_chat(
                provider,
                &prepared_messages.messages,
                request_tools,
                model,
                temperature,
                cancellation_token.as_ref(),
            )
            .await
        };

        let (
            response_text,
            parsed_text,
            tool_calls,
            assistant_history_content,
            native_tool_calls,
            parse_issue_detected,
            response_streamed_live,
        ) = match chat_result {
            Ok(resp) => {
                let (resp_input_tokens, resp_output_tokens) = resp
                    .usage
                    .as_ref()
                    .map(|u| (u.input_tokens, u.output_tokens))
                    .unwrap_or((None, None));

                observer.record_event(&ObserverEvent::LlmResponse {
                    provider: provider_name.to_string(),
                    model: model.to_string(),
                    duration: llm_started_at.elapsed(),
                    success: true,
                    error_message: None,
                    input_tokens: resp_input_tokens,
                    output_tokens: resp_output_tokens,
                });

                let response_text = resp.text_or_empty().to_string();
                // First try native structured tool calls (OpenAI-format).
                // Fall back to text-based parsing (XML tags, markdown blocks,
                // GLM format) only if the provider returned no native calls —
                // this ensures we support both native and prompt-guided models.
                let mut calls = parse_structured_tool_calls(&resp.tool_calls);
                let mut parsed_text = String::new();

                if calls.is_empty() {
                    let (fallback_text, fallback_calls) = parse_tool_calls(&response_text);
                    if !fallback_text.is_empty() {
                        parsed_text = fallback_text;
                    }
                    calls = fallback_calls;
                }

                let parse_issue = detect_tool_call_parse_issue(&response_text, &calls);
                if let Some(parse_issue) = parse_issue.as_ref() {
                    runtime_trace::record_event(
                        "tool_call_parse_issue",
                        Some(channel_name),
                        Some(provider_name),
                        Some(model),
                        Some(&turn_id),
                        Some(false),
                        Some(parse_issue),
                        serde_json::json!({
                            "iteration": iteration + 1,
                            "response_excerpt": truncate_with_ellipsis(
                                &scrub_credentials(&response_text),
                                600
                            ),
                        }),
                    );
                }

                runtime_trace::record_event(
                    "llm_response",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(true),
                    None,
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "duration_ms": llm_started_at.elapsed().as_millis(),
                        "input_tokens": resp_input_tokens,
                        "output_tokens": resp_output_tokens,
                        "raw_response": scrub_credentials(&response_text),
                        "native_tool_calls": resp.tool_calls.len(),
                        "parsed_tool_calls": calls.len(),
                    }),
                );

                // Preserve native tool call IDs in assistant history so role=tool
                // follow-up messages can reference the exact call id.
                let reasoning_content = resp.reasoning_content.clone();
                let assistant_history_content = if resp.tool_calls.is_empty() {
                    if use_native_tools {
                        build_native_assistant_history_from_parsed_calls(
                            &response_text,
                            &calls,
                            reasoning_content.as_deref(),
                        )
                        .unwrap_or_else(|| response_text.clone())
                    } else {
                        response_text.clone()
                    }
                } else {
                    build_native_assistant_history(
                        &response_text,
                        &resp.tool_calls,
                        reasoning_content.as_deref(),
                    )
                };

                let native_calls = resp.tool_calls;
                (
                    response_text,
                    parsed_text,
                    calls,
                    assistant_history_content,
                    native_calls,
                    parse_issue.is_some(),
                    streamed_live_deltas,
                )
            }
            Err(e) => {
                let safe_error = crate::providers::sanitize_api_error(&e.to_string());
                observer.record_event(&ObserverEvent::LlmResponse {
                    provider: provider_name.to_string(),
                    model: model.to_string(),
                    duration: llm_started_at.elapsed(),
                    success: false,
                    error_message: Some(safe_error.clone()),
                    input_tokens: None,
                    output_tokens: None,
                });
                runtime_trace::record_event(
                    "llm_response",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(&safe_error),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "duration_ms": llm_started_at.elapsed().as_millis(),
                    }),
                );
                return Err(e);
            }
        };

        let display_text = if parsed_text.is_empty() {
            response_text.clone()
        } else {
            parsed_text
        };

        // ── Progress: LLM responded ─────────────────────────────
        if let Some(ref tx) = on_delta {
            let llm_secs = llm_started_at.elapsed().as_secs();
            if !tool_calls.is_empty() {
                let _ = tx
                    .send(format!(
                        "{DRAFT_PROGRESS_SENTINEL}\u{1f4ac} Got {} tool call(s) ({llm_secs}s)\n",
                        tool_calls.len()
                    ))
                    .await;
            }
        }

        if tool_calls.is_empty() {
            let completion_claim_signal =
                looks_like_unverified_action_completion_without_tool_call(&display_text);
            let tool_unavailable_signal =
                looks_like_tool_unavailability_claim(&display_text, &tool_specs);
            let missing_tool_call_signal =
                parse_issue_detected || completion_claim_signal || tool_unavailable_signal;
            let missing_tool_call_followthrough = !missing_tool_call_retry_used
                && iteration + 1 < max_iterations
                && !tool_specs.is_empty()
                && missing_tool_call_signal;

            if missing_tool_call_followthrough {
                missing_tool_call_retry_used = true;
                missing_tool_call_retry_prompt = Some(if tool_unavailable_signal {
                    build_tool_unavailable_retry_prompt(&tool_specs)
                } else {
                    MISSING_TOOL_CALL_RETRY_PROMPT.to_string()
                });
                let retry_reason = if parse_issue_detected {
                    "parse_issue_detected"
                } else if tool_unavailable_signal {
                    "tool_unavailable_claim_detected"
                } else {
                    "completion_claim_text_detected"
                };
                runtime_trace::record_event(
                    "tool_call_followthrough_retry",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(retry_reason),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "response_excerpt": truncate_with_ellipsis(
                            &scrub_credentials(&display_text),
                            240
                        ),
                    }),
                );
                if let Some(ref tx) = on_delta {
                    let _ = tx
                        .send(format!(
                            "{DRAFT_PROGRESS_SENTINEL}\u{21bb} Retrying: response implied action without a verifiable tool call\n"
                        ))
                        .await;
                }
                continue;
            }

            if missing_tool_call_signal && missing_tool_call_retry_used {
                runtime_trace::record_event(
                    "tool_call_followthrough_failed",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some("model repeated deferred action without tool call"),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "response_excerpt": truncate_with_ellipsis(
                            &scrub_credentials(&display_text),
                            240
                        ),
                    }),
                );
                if tool_unavailable_signal && !parse_issue_detected && !completion_claim_signal {
                    tracing::warn!(
                        "Model still claims missing tools after corrective retry; returning text response."
                    );
                } else {
                    anyhow::bail!("Model repeatedly deferred action without emitting a tool call");
                }
            }

            runtime_trace::record_event(
                "turn_final_response",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                Some(true),
                None,
                serde_json::json!({
                    "iteration": iteration + 1,
                    "text": scrub_credentials(&display_text),
                }),
            );
            // No tool calls — this is the final response.
            // If a streaming sender is provided, relay the text in small chunks
            // so the channel can progressively update the draft message.
            if let Some(ref tx) = on_delta {
                let should_emit_post_hoc_chunks =
                    !response_streamed_live || display_text != response_text;
                if !should_emit_post_hoc_chunks {
                    history.push(ChatMessage::assistant(response_text.clone()));
                    return Ok(display_text);
                }
                // Clear accumulated progress lines before streaming the final answer.
                let _ = tx.send(DRAFT_CLEAR_SENTINEL.to_string()).await;
                // Split on whitespace boundaries, accumulating chunks of at least
                // STREAM_CHUNK_MIN_CHARS characters for progressive draft updates.
                let mut chunk = String::new();
                for word in display_text.split_inclusive(char::is_whitespace) {
                    if cancellation_token
                        .as_ref()
                        .is_some_and(CancellationToken::is_cancelled)
                    {
                        return Err(ToolLoopCancelled.into());
                    }
                    chunk.push_str(word);
                    if chunk.len() >= STREAM_CHUNK_MIN_CHARS
                        && tx.send(std::mem::take(&mut chunk)).await.is_err()
                    {
                        break; // receiver dropped
                    }
                }
                if !chunk.is_empty() {
                    let _ = tx.send(chunk).await;
                }
            }
            history.push(ChatMessage::assistant(response_text.clone()));
            return Ok(display_text);
        }

        // Print any text the LLM produced alongside tool calls (unless silent)
        if !silent && !display_text.is_empty() {
            print!("{display_text}");
            let _ = std::io::stdout().flush();
        }

        // Execute tool calls and build results. `individual_results` tracks per-call output so
        // native-mode history can emit one role=tool message per tool call with the correct ID.
        //
        // When multiple tool calls are present and interactive CLI approval is not needed, run
        // tool executions concurrently for lower wall-clock latency.
        let mut tool_results = String::new();
        let mut individual_results: Vec<(Option<String>, String)> = Vec::new();
        let mut ordered_results: Vec<Option<(String, Option<String>, ToolExecutionOutcome)>> =
            (0..tool_calls.len()).map(|_| None).collect();
        let allow_parallel_execution = should_execute_tools_in_parallel(
            &tool_calls,
            approval,
            bypass_non_cli_approval_for_turn,
        );
        let blocked_non_cli_plan_reason = if !bypass_non_cli_approval_for_turn
            && channel_name != "cli"
            && non_cli_approval_context.is_some()
        {
            blocked_non_cli_approval_plan_reason(&tool_calls, tools_registry)
        } else {
            None
        };
        let mut executable_indices: Vec<usize> = Vec::new();
        let mut executable_calls: Vec<ParsedToolCall> = Vec::new();

        for (idx, call) in tool_calls.iter().enumerate() {
            // ── Hook: before_tool_call (modifying) ──────────
            let mut tool_name = call.name.clone();
            let mut tool_args = call.arguments.clone();
            if let Some(hooks) = hooks {
                match hooks
                    .run_before_tool_call(tool_name.clone(), tool_args.clone())
                    .await
                {
                    crate::hooks::HookResult::Cancel(reason) => {
                        tracing::info!(tool = %call.name, %reason, "tool call cancelled by hook");
                        let cancelled = format!("Cancelled by hook: {reason}");
                        runtime_trace::record_event(
                            "tool_call_result",
                            Some(channel_name),
                            Some(provider_name),
                            Some(model),
                            Some(&turn_id),
                            Some(false),
                            Some(&cancelled),
                            serde_json::json!({
                                "iteration": iteration + 1,
                                "tool": call.name,
                                "arguments": scrub_credentials(&tool_args.to_string()),
                            }),
                        );
                        ordered_results[idx] = Some((
                            call.name.clone(),
                            call.tool_call_id.clone(),
                            ToolExecutionOutcome {
                                output: cancelled,
                                success: false,
                                error_reason: Some(scrub_credentials(&reason)),
                                duration: Duration::ZERO,
                            },
                        ));
                        continue;
                    }
                    crate::hooks::HookResult::Continue((name, args)) => {
                        tool_name = name;
                        tool_args = args;
                    }
                }
            }

            maybe_inject_cron_add_delivery(
                &tool_name,
                &mut tool_args,
                channel_name,
                channel_reply_target.as_deref(),
            );

            if excluded_set.contains(&tool_name.to_ascii_lowercase()) {
                let blocked = format!("Tool '{tool_name}' is not available in this channel.");
                runtime_trace::record_event(
                    "tool_call_result",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(&blocked),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "tool": tool_name.clone(),
                        "arguments": scrub_credentials(&tool_args.to_string()),
                        "blocked_by_channel_policy": true,
                    }),
                );
                ordered_results[idx] = Some((
                    tool_name.clone(),
                    call.tool_call_id.clone(),
                    ToolExecutionOutcome {
                        output: blocked.clone(),
                        success: false,
                        error_reason: Some(blocked),
                        duration: Duration::ZERO,
                    },
                ));
                continue;
            }

            // ── Approval hook ────────────────────────────────
            if let Some(mgr) = approval {
                if bypass_non_cli_approval_for_turn {
                    mgr.record_decision(
                        &tool_name,
                        &tool_args,
                        ApprovalResponse::Yes,
                        channel_name,
                    );
                } else {
                    let batched_non_cli_investigation = channel_name != "cli"
                        && non_cli_approval_context.is_some()
                        && qualifies_for_non_cli_investigation_batch(&tool_name, &tool_args);

                    if batched_non_cli_investigation {
                        mgr.record_decision(
                            &tool_name,
                            &tool_args,
                            ApprovalResponse::Yes,
                            channel_name,
                        );
                    } else if mgr.needs_approval(&tool_name) {
                        if let Some(blocked_reason) = blocked_non_cli_plan_reason.as_ref() {
                            runtime_trace::record_event(
                                "tool_call_result",
                                Some(channel_name),
                                Some(provider_name),
                                Some(model),
                                Some(&turn_id),
                                Some(false),
                                Some(blocked_reason),
                                serde_json::json!({
                                    "iteration": iteration + 1,
                                    "tool": tool_name.clone(),
                                    "arguments": scrub_credentials(&tool_args.to_string()),
                                    "approval_prompt_suppressed": true,
                                    "blocked_execution_plan": true,
                                }),
                            );
                            ordered_results[idx] = Some((
                                tool_name.clone(),
                                call.tool_call_id.clone(),
                                ToolExecutionOutcome {
                                    output: blocked_reason.clone(),
                                    success: false,
                                    error_reason: Some(blocked_reason.clone()),
                                    duration: Duration::ZERO,
                                },
                            ));
                            continue;
                        }
                        let request = ApprovalRequest {
                            tool_name: tool_name.clone(),
                            arguments: tool_args.clone(),
                        };
                        let approval_reason = if batched_non_cli_investigation {
                            Some(
                                "interactive approval required for low-risk investigation tools in this supervised non-cli turn"
                                    .to_string(),
                            )
                        } else {
                            Some(
                                "human confirmation required for the current supervised execution plan"
                                    .to_string(),
                            )
                        };

                        let decision = if channel_name == "cli" {
                            mgr.prompt_cli(&request)
                        } else if let Some(ctx) = non_cli_approval_context.as_ref() {
                            let (prompt_title, prompt_details) =
                                build_non_cli_approval_plan_prompt(&tool_calls);
                            let pending = mgr.create_non_cli_pending_request(
                                APPROVAL_ALL_TOOLS_ONCE_TOKEN,
                                &ctx.sender,
                                channel_name,
                                &ctx.reply_target,
                                Some(crate::approval::PendingNonCliResumeRequest {
                                    message_id: ctx.message_id.clone(),
                                    content: ctx.content.clone(),
                                    timestamp: ctx.timestamp,
                                    thread_ts: ctx.thread_ts.clone(),
                                }),
                                approval_reason,
                                collect_planned_shell_commands(&tool_calls),
                            );

                            let _ = ctx.prompt_tx.send(NonCliApprovalPrompt {
                                request_id: pending.request_id.clone(),
                                title: prompt_title,
                                details: prompt_details,
                            });

                            return Err(NonCliApprovalPending {
                                request_id: pending.request_id,
                                tool_name: "current execution plan".to_string(),
                            }
                            .into());
                        } else {
                            ApprovalResponse::No
                        };

                        mgr.record_decision(&tool_name, &tool_args, decision, channel_name);

                        if decision == ApprovalResponse::No {
                            let denied = "Denied by user.".to_string();
                            runtime_trace::record_event(
                                "tool_call_result",
                                Some(channel_name),
                                Some(provider_name),
                                Some(model),
                                Some(&turn_id),
                                Some(false),
                                Some(&denied),
                                serde_json::json!({
                                    "iteration": iteration + 1,
                                    "tool": tool_name.clone(),
                                    "arguments": scrub_credentials(&tool_args.to_string()),
                                }),
                            );
                            ordered_results[idx] = Some((
                                tool_name.clone(),
                                call.tool_call_id.clone(),
                                ToolExecutionOutcome {
                                    output: denied.clone(),
                                    success: false,
                                    error_reason: Some(denied),
                                    duration: Duration::ZERO,
                                },
                            ));
                            continue;
                        }
                    }
                }
            }

            let signature = tool_call_signature(&tool_name, &tool_args);
            if !seen_tool_signatures.insert(signature) {
                let duplicate = format!(
                    "Skipped duplicate tool call '{tool_name}' with identical arguments in this turn."
                );
                runtime_trace::record_event(
                    "tool_call_result",
                    Some(channel_name),
                    Some(provider_name),
                    Some(model),
                    Some(&turn_id),
                    Some(false),
                    Some(&duplicate),
                    serde_json::json!({
                        "iteration": iteration + 1,
                        "tool": tool_name.clone(),
                        "arguments": scrub_credentials(&tool_args.to_string()),
                        "deduplicated": true,
                    }),
                );
                ordered_results[idx] = Some((
                    tool_name.clone(),
                    call.tool_call_id.clone(),
                    ToolExecutionOutcome {
                        output: duplicate.clone(),
                        success: false,
                        error_reason: Some(duplicate),
                        duration: Duration::ZERO,
                    },
                ));
                continue;
            }

            runtime_trace::record_event(
                "tool_call_start",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                None,
                None,
                serde_json::json!({
                    "iteration": iteration + 1,
                    "tool": tool_name.clone(),
                    "arguments": scrub_credentials(&tool_args.to_string()),
                }),
            );

            // ── Progress: tool start ────────────────────────────
            if let Some(ref tx) = on_delta {
                let hint = truncate_tool_args_for_progress(&tool_name, &tool_args, 60);
                let progress = if hint.is_empty() {
                    format!("\u{23f3} {}\n", tool_name)
                } else {
                    format!("\u{23f3} {}: {hint}\n", tool_name)
                };
                tracing::debug!(tool = %tool_name, "Sending progress start to draft");
                let _ = tx
                    .send(format!("{DRAFT_PROGRESS_SENTINEL}{progress}"))
                    .await;
            }

            executable_indices.push(idx);
            executable_calls.push(ParsedToolCall {
                name: tool_name,
                arguments: tool_args,
                tool_call_id: call.tool_call_id.clone(),
            });
        }

        let executed_outcomes = if allow_parallel_execution && executable_calls.len() > 1 {
            execute_tools_parallel(
                &executable_calls,
                tools_registry,
                observer,
                cancellation_token.as_ref(),
                approved_turn_grant.as_ref(),
            )
            .await?
        } else {
            execute_tools_sequential(
                &executable_calls,
                tools_registry,
                observer,
                cancellation_token.as_ref(),
                approved_turn_grant.as_ref(),
            )
            .await?
        };

        let web_only_iteration = !executable_calls.is_empty()
            && executable_calls
                .iter()
                .all(|call| is_web_lookup_tool(&call.name));
        let failed_web_tools_this_iteration = executable_calls
            .iter()
            .zip(executed_outcomes.iter())
            .filter(|(call, outcome)| is_web_lookup_tool(&call.name) && !outcome.success)
            .count();
        let any_tool_succeeded = executed_outcomes.iter().any(|outcome| outcome.success);
        if web_only_iteration && failed_web_tools_this_iteration > 0 {
            consecutive_web_tool_failures += failed_web_tools_this_iteration;
            last_web_tool_error = executable_calls
                .iter()
                .zip(executed_outcomes.iter())
                .rev()
                .find_map(|(call, outcome)| {
                    if is_web_lookup_tool(&call.name) && !outcome.success {
                        outcome
                            .error_reason
                            .clone()
                            .or_else(|| Some(outcome.output.clone()))
                    } else {
                        None
                    }
                });
        } else if any_tool_succeeded {
            consecutive_web_tool_failures = 0;
            last_web_tool_error = None;
        }
        let should_stop_repeated_web_failures =
            consecutive_web_tool_failures >= MAX_CONSECUTIVE_WEB_TOOL_FAILURES;

        for ((idx, call), outcome) in executable_indices
            .iter()
            .zip(executable_calls.iter())
            .zip(executed_outcomes.into_iter())
        {
            runtime_trace::record_event(
                "tool_call_result",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                Some(outcome.success),
                outcome.error_reason.as_deref(),
                serde_json::json!({
                    "iteration": iteration + 1,
                    "tool": call.name.clone(),
                    "duration_ms": outcome.duration.as_millis(),
                    "output": scrub_credentials(&outcome.output),
                }),
            );

            // ── Hook: after_tool_call (void) ─────────────────
            if let Some(hooks) = hooks {
                let tool_result_obj = crate::tools::ToolResult {
                    success: outcome.success,
                    output: outcome.output.clone(),
                    error: None,
                };
                hooks
                    .fire_after_tool_call(&call.name, &tool_result_obj, outcome.duration)
                    .await;
            }

            // ── Progress: tool completion ───────────────────────
            if let Some(ref tx) = on_delta {
                let secs = outcome.duration.as_secs();
                let icon = if outcome.success {
                    "\u{2705}"
                } else {
                    "\u{274c}"
                };
                tracing::debug!(tool = %call.name, secs, "Sending progress complete to draft");
                let _ = tx
                    .send(format!(
                        "{DRAFT_PROGRESS_SENTINEL}{icon} {} ({secs}s)\n",
                        call.name
                    ))
                    .await;
            }

            ordered_results[*idx] = Some((call.name.clone(), call.tool_call_id.clone(), outcome));
        }

        for (tool_name, tool_call_id, outcome) in ordered_results.into_iter().flatten() {
            individual_results.push((tool_call_id, outcome.output.clone()));
            let _ = writeln!(
                tool_results,
                "<tool_result name=\"{}\">\n{}\n</tool_result>",
                tool_name, outcome.output
            );
        }

        // Add assistant message with tool calls + tool results to history.
        // Native mode: use JSON-structured messages so convert_messages() can
        // reconstruct proper OpenAI-format tool_calls and tool result messages.
        // Prompt mode: use XML-based text format as before.
        history.push(ChatMessage::assistant(assistant_history_content));
        if native_tool_calls.is_empty() {
            let all_results_have_ids = use_native_tools
                && !individual_results.is_empty()
                && individual_results
                    .iter()
                    .all(|(tool_call_id, _)| tool_call_id.is_some());
            if all_results_have_ids {
                for (tool_call_id, result) in &individual_results {
                    let tool_msg = serde_json::json!({
                        "tool_call_id": tool_call_id,
                        "content": result,
                    });
                    history.push(ChatMessage::tool(tool_msg.to_string()));
                }
            } else {
                history.push(ChatMessage::user(format!("[Tool results]\n{tool_results}")));
            }
        } else {
            for (native_call, (_, result)) in
                native_tool_calls.iter().zip(individual_results.iter())
            {
                let tool_msg = serde_json::json!({
                    "tool_call_id": native_call.id,
                    "content": result,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }

        if should_stop_repeated_web_failures {
            let response = repeated_web_tool_failure_response(
                consecutive_web_tool_failures,
                last_web_tool_error.as_deref(),
            );
            runtime_trace::record_event(
                "tool_loop_stopped_repeated_web_failures",
                Some(channel_name),
                Some(provider_name),
                Some(model),
                Some(&turn_id),
                Some(false),
                Some("repeated web tool failures"),
                serde_json::json!({
                    "failure_count": consecutive_web_tool_failures,
                    "last_error": last_web_tool_error,
                }),
            );
            return Ok(response);
        }
    }

    runtime_trace::record_event(
        "tool_loop_exhausted",
        Some(channel_name),
        Some(provider_name),
        Some(model),
        Some(&turn_id),
        Some(false),
        Some("agent exceeded maximum tool iterations"),
        serde_json::json!({
            "max_iterations": max_iterations,
        }),
    );
    anyhow::bail!("Agent exceeded maximum tool iterations ({max_iterations})")
}

/// Build the tool instruction block for the system prompt from concrete tool
/// specs so the LLM knows how to invoke tools.
pub(crate) fn build_tool_instructions(tools_registry: &[Box<dyn Tool>]) -> String {
    let specs: Vec<crate::tools::ToolSpec> =
        tools_registry.iter().map(|tool| tool.spec()).collect();
    build_tool_instructions_from_specs(&specs)
}

/// Build the tool instruction block for the system prompt from concrete tool
/// specs so the LLM knows how to invoke tools.
pub(crate) fn build_tool_instructions_from_specs(tool_specs: &[crate::tools::ToolSpec]) -> String {
    let mut instructions = String::new();
    instructions.push_str("\n## Tool Use Protocol\n\n");
    instructions.push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
    instructions.push_str("```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n");
    instructions.push_str(
        "CRITICAL: Output actual <tool_call> tags—never describe steps or give examples.\n\n",
    );
    instructions.push_str(
        "When a tool is needed, emit a real call (not prose), for example:\n\
<tool_call>\n\
{\"name\":\"tool_name\",\"arguments\":{}}\n\
</tool_call>\n\n",
    );
    instructions.push_str("You may use multiple tool calls in a single response. ");
    instructions.push_str("After tool execution, results appear in <tool_result> tags. ");
    instructions
        .push_str("Continue reasoning with the results until you can give a final answer.\n\n");
    instructions.push_str("### Available Tools\n\n");

    for tool in tool_specs {
        let _ = writeln!(
            instructions,
            "**{}**: {}\nParameters: `{}`\n",
            tool.name, tool.description, tool.parameters
        );
    }

    instructions
}

/// Build shell-policy instructions for the system prompt so the model is aware
/// of command-level execution constraints before it emits tool calls.
pub(crate) fn build_shell_policy_instructions(autonomy: &crate::config::AutonomyConfig) -> String {
    let mut instructions = String::new();
    instructions.push_str("\n## Shell Policy\n\n");
    instructions
        .push_str("When using the `shell` tool, follow these runtime constraints exactly.\n\n");

    let autonomy_label = match autonomy.level {
        crate::security::AutonomyLevel::ReadOnly => "read_only",
        crate::security::AutonomyLevel::Supervised => "supervised",
        crate::security::AutonomyLevel::Full => "full",
    };
    let _ = writeln!(instructions, "- Autonomy level: `{autonomy_label}`");

    if autonomy.level == crate::security::AutonomyLevel::ReadOnly {
        instructions.push_str(
            "- Shell execution is disabled in `read_only` mode. Do not emit shell tool calls.\n",
        );
        return instructions;
    }

    let normalized: BTreeSet<String> = autonomy
        .allowed_commands
        .iter()
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    if normalized.contains("*") {
        instructions.push_str(
            "- Allowed commands: wildcard `*` is configured (any command name/path may be allowlisted).\n",
        );
    } else if normalized.is_empty() {
        instructions
            .push_str("- Allowed commands: none configured. Any shell command will be rejected.\n");
    } else {
        const MAX_DISPLAY_COMMANDS: usize = 64;
        let shown: Vec<String> = normalized
            .iter()
            .take(MAX_DISPLAY_COMMANDS)
            .map(|cmd| format!("`{cmd}`"))
            .collect();
        let hidden = normalized.len().saturating_sub(MAX_DISPLAY_COMMANDS);
        let _ = write!(instructions, "- Allowed commands: {}", shown.join(", "));
        if hidden > 0 {
            let _ = write!(instructions, " (+{hidden} more)");
        }
        instructions.push('\n');
    }

    if autonomy.level == crate::security::AutonomyLevel::Supervised
        && autonomy.require_approval_for_medium_risk
    {
        instructions.push_str(
            "- Medium-risk shell commands require explicit approval in `supervised` mode.\n",
        );
    }
    if autonomy.block_high_risk_commands {
        instructions.push_str(
            "- High-risk shell commands are blocked even when command names are allowed.\n",
        );
    }
    instructions.push_str(
        "- If a requested command is outside policy, choose allowed alternatives and explain the limitation.\n",
    );
    instructions.push_str(
        "- Call the shell tool directly when you need to run a command. The runtime handles approval when required — do not present a text-based preflight or ask the user to confirm before calling the tool.\n",
    );

    instructions
}

fn build_runtime_tool_availability_notice(tools_registry: &[Box<dyn Tool>]) -> String {
    const MAX_LISTED_TOOLS: usize = 40;
    let names = tools_registry
        .iter()
        .map(|tool| tool.name())
        .take(MAX_LISTED_TOOLS)
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "\n## Runtime Tool Availability (Authoritative)\n\n\
         Use only these runtime-available tools for this turn.\n\
         Tools: {names}\n\
         Do not claim tools are unavailable when they are listed here.\n\
         If the user asks about your abilities, answer from this tool list plus loaded skills and prompt context.\n\
         Distinguish clearly between actions available now, actions that still require approval, and workflows that remain operator-controlled.\n\
         Self-improvement is not automatic by default; candidate preparation, validation, and promotion remain operator-controlled unless the runtime explicitly says otherwise.\n"
    )
}

// ── CLI Entrypoint ───────────────────────────────────────────────────────
// Wires up all subsystems (observer, runtime, security, memory, tools,
// provider, hardware RAG, peripherals) and enters either single-shot or
// interactive REPL mode. The interactive loop manages history compaction
// and hard trimming to keep the context window bounded.

#[allow(clippy::too_many_lines)]
pub async fn run(
    config: Config,
    message: Option<String>,
    provider_override: Option<String>,
    model_override: Option<String>,
    temperature: f64,
    peripheral_overrides: Vec<String>,
    interactive: bool,
) -> Result<String> {
    // ── Wire up agnostic subsystems ──────────────────────────────
    let observer = wiring::build_observer(&config);
    let wiring::ExecutionSupport {
        memory: mem,
        tools: tools_registry,
        ..
    } = wiring::build_execution_support(&config, &[])?;
    tracing::info!(backend = mem.name(), "Memory initialized");

    // ── Peripherals (merge peripheral tools into registry) ─
    if !peripheral_overrides.is_empty() {
        tracing::info!(
            peripherals = ?peripheral_overrides,
            "Peripheral overrides from CLI (config boards take precedence)"
        );
    }

    // ── Resolve provider ─────────────────────────────────────────
    let provider_name = provider_override
        .as_deref()
        .or(config.default_provider.as_deref())
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME);

    let model_name = model_override
        .as_deref()
        .or(config.default_model.as_deref())
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_MODEL);

    let provider_runtime_options = providers::ProviderRuntimeOptions::from_config(&config);

    let provider: Box<dyn Provider> = providers::create_routed_provider_with_options(
        provider_name,
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &config.model_routes,
        model_name,
        &provider_runtime_options,
    )?;

    observer.record_event(&ObserverEvent::AgentStart {
        provider: provider_name.to_string(),
        model: model_name.to_string(),
    });

    // ── Build system prompt from workspace MD files ──
    let skills = wiring::load_skills(&config);
    let mut tool_descs: Vec<(&str, &str)> = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory_store",
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.",
        ),
        (
            "memory_recall",
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.",
        ),
        (
            "memory_forget",
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.",
        ),
    ];
    tool_descs.push((
        "cron_add",
        "Create a cron job. Supports schedule kinds: cron, at, every; and job types: shell or agent.",
    ));
    tool_descs.push((
        "cron_list",
        "List all cron jobs with schedule, status, and metadata.",
    ));
    tool_descs.push(("cron_remove", "Remove a cron job by job_id."));
    tool_descs.push((
        "cron_update",
        "Patch a cron job (schedule, enabled, command/prompt, model, delivery, session_target).",
    ));
    tool_descs.push((
        "cron_run",
        "Force-run a cron job immediately and record a run history entry.",
    ));
    tool_descs.push(("cron_runs", "Show recent run history for a cron job."));
    tool_descs.push((
        "screenshot",
        "Capture a screenshot of the current screen. Returns file path and base64-encoded PNG. Use when: visual verification, UI inspection, debugging displays.",
    ));
    tool_descs.push((
        "image_info",
        "Read image file metadata (format, dimensions, size) and optionally base64-encode it. Use when: inspecting images, preparing visual data for analysis.",
    ));
    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in system browser (allowlist-only, no scraping)",
        ));
    }
    if config.channels_config.discord.is_some() {
        tool_descs.push((
            "discord_history_fetch",
            "Fetch Discord message history on demand for current conversation context or explicit channel_id.",
        ));
    }
    tool_descs.push((
            "model_routing_config",
        "Configure default model, scenario routing, and delegate agents. Use for natural-language requests like: 'set conversation to kimi and coding to gpt-5.3-codex'.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a sub-task to a specialized agent. Use when: task needs different model/capability, or to parallelize work.",
        ));
    }
    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let native_tools = provider.supports_native_tools();
    let mut system_prompt = crate::channels::build_system_prompt_with_mode(
        &config.workspace_dir,
        model_name,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
        native_tools,
        config.skills.prompt_injection_mode,
    );

    // Append structured tool-use instructions with schemas (only for non-native providers)
    if !native_tools {
        system_prompt.push_str(&build_tool_instructions(&tools_registry));
    }
    system_prompt.push_str(&build_shell_policy_instructions(&config.autonomy));
    system_prompt.push_str(&build_runtime_tool_availability_notice(&tools_registry));

    // ── Approval manager (supervised mode) ───────────────────────
    let approval_manager = if interactive {
        Some(ApprovalManager::from_config(&config.autonomy))
    } else {
        None
    };
    let channel_name = if interactive { "cli" } else { "daemon" };

    // ── Execute ──────────────────────────────────────────────────
    let start = Instant::now();

    let mut final_output = String::new();

    if let Some(msg) = message {
        // Auto-save user message to memory (skip short/trivial messages)
        if config.memory.auto_save && msg.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS {
            let user_key = autosave_memory_key("user_msg");
            let _ = mem
                .store(&user_key, &msg, MemoryCategory::Conversation, None)
                .await;
        }

        // Inject memory + hardware RAG context into user message
        let mem_context =
            build_context(mem.as_ref(), &msg, config.memory.min_relevance_score).await;
        let context = mem_context;
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
        let enriched = if context.is_empty() {
            format!("[{now}] {msg}")
        } else {
            format!("{context}[{now}] {msg}")
        };

        let mut history = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&enriched),
        ];

        let response = run_tool_call_loop(
            provider.as_ref(),
            &mut history,
            &tools_registry,
            observer.as_ref(),
            provider_name,
            model_name,
            temperature,
            false,
            approval_manager.as_ref(),
            channel_name,
            &config.multimodal,
            config.agent.max_tool_iterations,
            None,
            None,
            None,
            &[],
        )
        .await?;
        final_output = response.clone();
        println!("{response}");
        observer.record_event(&ObserverEvent::TurnComplete);
    } else {
        println!("🦀 TopClaw Interactive Mode");
        println!("Type /help for commands.\n");
        let cli = crate::channels::CliChannel::new();

        // Persistent conversation history across turns
        let mut history = vec![ChatMessage::system(&system_prompt)];
        let mut lossless_context = LosslessContext::new(&config.workspace_dir, &system_prompt)?;
        // Reusable readline editor for UTF-8 input support
        let mut rl = rustyline::DefaultEditor::new()?;

        loop {
            let input = match rl.readline("> ") {
                Ok(line) => line,
                Err(ReadlineError::Interrupted | ReadlineError::Eof) => {
                    break;
                }
                Err(e) => {
                    eprintln!("\nError reading input: {e}\n");
                    break;
                }
            };

            let user_input = input.trim().to_string();
            if user_input.is_empty() {
                continue;
            }
            rl.add_history_entry(&input)?;
            match user_input.as_str() {
                "/quit" | "/exit" => break,
                "/help" => {
                    println!("Available commands:");
                    println!("  /help        Show this help message");
                    println!("  /clear /new  Clear conversation history");
                    println!("  /quit /exit  Exit interactive mode\n");
                    continue;
                }
                "/clear" | "/new" => {
                    println!(
                        "This will clear the current conversation and delete all session memory."
                    );
                    println!("Core memories (long-term facts/preferences) will be preserved.");
                    let confirm = rl.readline("Continue? [y/N] ").unwrap_or_default();

                    if !matches!(confirm.trim().to_lowercase().as_str(), "y" | "yes") {
                        println!("Cancelled.\n");
                        continue;
                    }

                    // Ensure prior prompts are not navigable after reset.
                    rl.clear_history()?;
                    history.clear();
                    history.push(ChatMessage::system(&system_prompt));
                    lossless_context.reset(&system_prompt)?;
                    // Clear conversation and daily memory
                    let mut cleared = 0;
                    for category in [MemoryCategory::Conversation, MemoryCategory::Daily] {
                        let entries = mem.list(Some(&category), None).await.unwrap_or_default();
                        for entry in entries {
                            if mem.forget(&entry.key).await.unwrap_or(false) {
                                cleared += 1;
                            }
                        }
                    }
                    if cleared > 0 {
                        println!("Conversation cleared ({cleared} memory entries removed).\n");
                    } else {
                        println!("Conversation cleared.\n");
                    }
                    continue;
                }
                _ => {}
            }

            // Auto-save conversation turns (skip short/trivial messages)
            if config.memory.auto_save && user_input.chars().count() >= AUTOSAVE_MIN_MESSAGE_CHARS {
                let user_key = autosave_memory_key("user_msg");
                let _ = mem
                    .store(&user_key, &user_input, MemoryCategory::Conversation, None)
                    .await;
            }

            // Inject memory context into user message
            let mem_context =
                build_context(mem.as_ref(), &user_input, config.memory.min_relevance_score).await;
            let context = mem_context;
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
            let enriched = if context.is_empty() {
                format!("[{now}] {user_input}")
            } else {
                format!("{context}[{now}] {user_input}")
            };

            let pre_turn_len = history.len();
            history.push(ChatMessage::user(&enriched));
            lossless_context.record_raw_message(&ChatMessage::user(&enriched))?;

            let response = match run_tool_call_loop(
                provider.as_ref(),
                &mut history,
                &tools_registry,
                observer.as_ref(),
                provider_name,
                model_name,
                temperature,
                false,
                approval_manager.as_ref(),
                channel_name,
                &config.multimodal,
                config.agent.max_tool_iterations,
                None,
                None,
                None,
                &[],
            )
            .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    lossless_context.record_raw_messages(&history[pre_turn_len + 1..])?;
                    if is_tool_iteration_limit_error(&e) {
                        let pause_notice = format!(
                            "⚠️ Reached tool-iteration limit ({}). Context and progress are preserved. \
                            Reply \"continue\" to resume, or increase `agent.max_tool_iterations` in config.",
                            config.agent.max_tool_iterations.max(DEFAULT_MAX_TOOL_ITERATIONS)
                        );
                        history.push(ChatMessage::assistant(&pause_notice));
                        lossless_context
                            .record_raw_message(&ChatMessage::assistant(&pause_notice))?;
                        eprintln!("\n{pause_notice}\n");
                        continue;
                    }
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            };
            final_output = response.clone();
            if let Err(e) = crate::channels::Channel::send(
                &cli,
                &crate::channels::traits::SendMessage::new(format!("\n{response}\n"), "user"),
            )
            .await
            {
                eprintln!("\nError sending CLI response: {e}\n");
            }
            observer.record_event(&ObserverEvent::TurnComplete);

            lossless_context.record_raw_messages(&history[pre_turn_len + 1..])?;
            history = lossless_context
                .rebuild_active_history(
                    provider.as_ref(),
                    model_name,
                    &system_prompt,
                    config.agent.max_history_messages,
                )
                .await?;
            println!("🧠 Lossless context refresh complete");

            // Hard cap as a safety net.
            trim_history(&mut history, config.agent.max_history_messages);
        }
    }

    let duration = start.elapsed();
    observer.record_event(&ObserverEvent::AgentEnd {
        provider: provider_name.to_string(),
        model: model_name.to_string(),
        duration,
        tokens_used: None,
        cost_usd: None,
    });

    Ok(final_output)
}

/// Process a single message through the full agent (with tools, peripherals, memory).
/// Used by channels (Telegram, Discord, etc.) to enable hardware and tool use.
pub async fn process_message(config: Config, message: &str) -> Result<String> {
    let observer = wiring::build_observer(&config);
    let wiring::ExecutionSupport {
        memory: mem,
        tools: tools_registry,
        ..
    } = wiring::build_execution_support(&config, &[])?;

    let provider_name = config
        .default_provider
        .as_deref()
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME);
    let model_name = config
        .default_model
        .clone()
        .unwrap_or_else(|| crate::providers::DEFAULT_PROVIDER_MODEL.into());
    let provider_runtime_options = providers::ProviderRuntimeOptions::from_config(&config);
    let provider: Box<dyn Provider> = providers::create_routed_provider_with_options(
        provider_name,
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &config.model_routes,
        &model_name,
        &provider_runtime_options,
    )?;

    let skills = wiring::load_skills(&config);
    let mut tool_descs: Vec<(&str, &str)> = vec![
        ("shell", "Execute terminal commands."),
        ("file_read", "Read file contents."),
        ("file_write", "Write file contents."),
        ("memory_store", "Save to memory."),
        ("memory_recall", "Search memory."),
        ("memory_forget", "Delete a memory entry."),
        (
            "model_routing_config",
            "Configure default model, scenario routing, and delegate agents.",
        ),
        ("screenshot", "Capture a screenshot."),
        ("image_info", "Read image metadata."),
    ];
    if config.browser.enabled {
        tool_descs.push(("browser_open", "Open approved URLs in browser."));
    }
    if config.channels_config.discord.is_some() {
        tool_descs.push((
            "discord_history_fetch",
            "Fetch Discord message history on demand for current conversation context or explicit channel_id.",
        ));
    }
    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let native_tools = provider.supports_native_tools();
    let mut system_prompt = crate::channels::build_system_prompt_with_mode(
        &config.workspace_dir,
        &model_name,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
        native_tools,
        config.skills.prompt_injection_mode,
    );
    if !native_tools {
        system_prompt.push_str(&build_tool_instructions(&tools_registry));
    }
    system_prompt.push_str(&build_shell_policy_instructions(&config.autonomy));
    system_prompt.push_str(&build_runtime_tool_availability_notice(&tools_registry));

    let mem_context = build_context(mem.as_ref(), message, config.memory.min_relevance_score).await;
    let context = mem_context;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let enriched = if context.is_empty() {
        format!("[{now}] {message}")
    } else {
        format!("{context}[{now}] {message}")
    };

    let mut history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&enriched),
    ];

    agent_turn(
        provider.as_ref(),
        &mut history,
        &tools_registry,
        observer.as_ref(),
        provider_name,
        &model_name,
        config.default_temperature,
        true,
        &config.multimodal,
        config.agent.max_tool_iterations,
    )
    .await
}

#[cfg(test)]
#[path = "loop_/loop_tests.rs"]
mod tests;
