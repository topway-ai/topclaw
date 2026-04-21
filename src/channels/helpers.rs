//! Shared helper functions for channel runtime.

use super::capability_detection::{
    looks_like_audio_capability_question, looks_like_computer_use_availability_question,
    looks_like_current_model_question, looks_like_loaded_skills_question,
    looks_like_rate_limit_question, looks_like_skill_workflow_advisory_question,
    looks_like_tool_inventory_question, looks_like_tools_and_skills_question,
};
use crate::memory;
use crate::providers::{self, ChatMessage, Provider};
use anyhow::Context;
use std::path::Path;
use std::sync::Arc;

use super::context::*;

pub(super) fn split_internal_progress_delta(delta: &str) -> (bool, &str) {
    if let Some(rest) = delta.strip_prefix(crate::agent::loop_::DRAFT_PROGRESS_SENTINEL) {
        (true, rest)
    } else {
        (false, delta)
    }
}

pub(super) fn summarize_internal_progress_delta(delta: &str) -> Option<String> {
    let trimmed = delta.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("⏳ Still working") || trimmed.starts_with("🤔 Still thinking") {
        Some(format!("{trimmed}\n"))
    } else if trimmed == "🤔 Thinking..." {
        Some("Analyzing the request...\n".to_string())
    } else if trimmed
        .strip_prefix("🤔 Thinking (round ")
        .and_then(|rest| rest.strip_suffix(")..."))
        .is_some()
    {
        None
    } else if trimmed.contains("Thinking") {
        Some("Analyzing the request...\n".to_string())
    } else if trimmed.starts_with('↻') || trimmed.contains("Retrying:") {
        Some("Retrying the previous step...\n".to_string())
    } else if let Some(rest) = trimmed.strip_prefix("⏳ ") {
        let (tool_name, detail) = match rest.split_once(':') {
            Some((name, detail)) => (name.trim(), Some(detail.trim())),
            None => (rest.split_whitespace().next().unwrap_or("").trim(), None),
        };

        if tool_name.is_empty() || tool_name.eq_ignore_ascii_case("Still") {
            Some(format!("{trimmed}\n"))
        } else if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
            Some(format!("Using `{tool_name}`: {detail}\n"))
        } else {
            Some(format!("Using `{tool_name}`...\n"))
        }
    } else if trimmed.contains("tool call") {
        None
    } else if trimmed.starts_with('✅') || trimmed.starts_with('❌') {
        let status = if trimmed.starts_with('✅') {
            "Finished"
        } else {
            "Failed"
        };
        let rest = trimmed.trim_start_matches(['✅', '❌']).trim_start();
        let tool_name = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(':')
            .trim();
        if tool_name.is_empty() {
            Some(format!("{status} the current step.\n"))
        } else {
            Some(format!("{status} `{tool_name}`.\n"))
        }
    } else {
        None
    }
}

pub(super) fn is_generic_progress_heartbeat(summary: &str) -> bool {
    let trimmed = summary.trim();
    trimmed.starts_with("⏳ Still working") || trimmed.starts_with("🤔 Still thinking")
}

pub(super) fn contextualize_progress_heartbeat(
    summary: &str,
    last_meaningful_summary: Option<&str>,
) -> String {
    if !is_generic_progress_heartbeat(summary) {
        return summary.to_string();
    }

    let Some(previous) = last_meaningful_summary
        .map(str::trim)
        .filter(|value| !value.is_empty() && !is_generic_progress_heartbeat(value))
    else {
        return "⏳ Still working: Analyzing the request...\n".to_string();
    };

    format!(
        "⏳ Still working: {}\n",
        previous.trim_end_matches('.').trim_end_matches('\n')
    )
}

pub(super) fn normalize_cached_channel_turns(turns: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut normalized = Vec::with_capacity(turns.len());
    let mut expecting_user = true;

    for turn in turns {
        match (expecting_user, turn.role.as_str()) {
            (true, "user") => {
                normalized.push(turn);
                expecting_user = false;
            }
            (false, "assistant") => {
                normalized.push(turn);
                expecting_user = true;
            }
            // Interrupted channel turns can produce consecutive user messages
            // (no assistant persisted yet). Merge instead of dropping.
            (false, "user") | (true, "assistant") => {
                if let Some(last_turn) = normalized.last_mut() {
                    if !turn.content.is_empty() {
                        if !last_turn.content.is_empty() {
                            last_turn.content.push_str("\n\n");
                        }
                        last_turn.content.push_str(&turn.content);
                    }
                }
            }
            _ => {}
        }
    }

    normalized
}

pub(super) fn filter_noop_assistant_turns(turns: Vec<ChatMessage>) -> Vec<ChatMessage> {
    turns
        .into_iter()
        .filter(|turn| !(turn.role == "assistant" && is_agent_noop_sentinel(&turn.content)))
        .collect()
}

/// Classify a user message and return the appropriate route selection with logging.
/// Returns None if classification is disabled or no rules match.
pub(super) fn classify_message_route(
    ctx: &ChannelRuntimeContext,
    message: &str,
) -> Option<ChannelRouteSelection> {
    let decision =
        crate::agent::classifier::classify_with_decision(&ctx.query_classification, message)?;

    // Find the matching model route
    let route = ctx.model_routes.iter().find(|r| r.hint == decision.hint)?;

    tracing::info!(
        target: "query_classification",
        hint = %decision.hint,
        model = %route.model,
        rule_priority = decision.priority,
        message_length = message.len(),
        "Classified message route"
    );

    Some(ChannelRouteSelection {
        provider: route.provider.clone(),
        model: route.model.clone(),
    })
}

pub(super) fn should_skip_memory_context_entry(key: &str, content: &str) -> bool {
    if memory::is_assistant_autosave_key(key) {
        return true;
    }

    if key.trim().to_ascii_lowercase().ends_with("_history") {
        return true;
    }

    content.chars().count() > MEMORY_CONTEXT_MAX_CHARS
}

pub(super) fn is_context_window_overflow_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    [
        "exceeds the context window",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "too many tokens",
        "token limit exceeded",
        "prompt is too long",
        "input is too long",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

pub(super) fn is_tool_iteration_limit_error(err: &anyhow::Error) -> bool {
    crate::agent::loop_::is_tool_iteration_limit_error(err)
}

pub(super) fn format_user_visible_llm_error(channel_name: &str, err: &anyhow::Error) -> String {
    let safe_error = providers::sanitize_api_error(&err.to_string());
    let lower = safe_error.to_ascii_lowercase();
    let route_hint = if matches!(channel_name, "telegram" | "discord") {
        " or use `/models` to switch provider/model"
    } else {
        ""
    };

    if lower.contains("all providers/models failed") {
        if lower.contains("error decoding response body") {
            return format!(
                "⚠️ The configured provider returned an unreadable response after several retries. Please try again{route_hint}."
            );
        }

        return format!(
            "⚠️ The configured provider/model could not complete this request after several retries. Please try again{route_hint}."
        );
    }

    if lower.contains("model repeatedly deferred action without emitting a tool call") {
        return "⚠️ I got stuck deciding the next step and did not complete that request. Please try again or phrase it more directly.".to_string();
    }

    format!("⚠️ Error: {safe_error}")
}

fn extract_loaded_skill_names_from_system_prompt(system_prompt: &str) -> Vec<String> {
    let Some(section_start) = system_prompt.find("<available_skills>") else {
        return Vec::new();
    };
    let after_start = &system_prompt[section_start + "<available_skills>".len()..];
    let Some(section_end) = after_start.find("</available_skills>") else {
        return Vec::new();
    };
    let mut remaining = &after_start[..section_end];
    let mut names = Vec::new();

    while let Some(skill_start) = remaining.find("<skill>") {
        let after_skill_start = &remaining[skill_start + "<skill>".len()..];
        let Some(skill_end) = after_skill_start.find("</skill>") else {
            break;
        };
        let skill_block = &after_skill_start[..skill_end];
        if let Some(name_start) = skill_block.find("<name>") {
            let after_name_start = &skill_block[name_start + "<name>".len()..];
            if let Some(name_end) = after_name_start.find("</name>") {
                let name = after_name_start[..name_end].trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
        remaining = &after_skill_start[skill_end + "</skill>".len()..];
    }

    names.sort();
    names.dedup();
    names
}

pub(super) fn build_local_capability_response(
    content: &str,
    system_prompt: &str,
    provider_name: &str,
    model_name: &str,
    visible_tool_names: &[String],
) -> Option<String> {
    if looks_like_current_model_question(content) {
        return Some(format!(
            "I'm currently using provider `{provider_name}` with model `{model_name}`."
        ));
    }

    if looks_like_rate_limit_question(content) {
        return Some(format!(
            "The default action budget is {} actions per hour, configured by `autonomy.max_actions_per_hour`.",
            crate::config::AutonomyConfig::default().max_actions_per_hour
        ));
    }

    if looks_like_tools_and_skills_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let tool_section = if visible_tool_names.is_empty() {
            "Visible tools in this runtime turn: (none).".to_string()
        } else {
            format!(
                "Visible tools in this runtime turn ({}): {}.",
                visible_tool_names.len(),
                visible_tool_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let skill_section = if skill_names.is_empty() {
            "Loaded skills in the current prompt: (none).".to_string()
        } else {
            format!(
                "Loaded skills in the current prompt ({}): {}.",
                skill_names.len(),
                skill_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        return Some(format!(
            "{tool_section}\n{skill_section}\n`computer_use` is a runtime tool when compiled and allowed, not a marketplace skill."
        ));
    }

    if looks_like_tool_inventory_question(content) {
        if visible_tool_names.is_empty() {
            return Some(
                "Visible tools in this runtime turn: (none). I can still answer from chat context, but I should not claim tool access that is not actually loaded."
                    .to_string(),
            );
        }

        return Some(format!(
            "Visible tools in this runtime turn ({}): {}.",
            visible_tool_names.len(),
            visible_tool_names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if looks_like_computer_use_availability_question(content) {
        let has_computer_use = visible_tool_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("computer_use"));
        let availability = if has_computer_use {
            "`computer_use` is currently loaded as a runtime tool in this turn."
        } else {
            "`computer_use` is not currently loaded in this runtime turn."
        };

        return Some(format!(
            "{availability} `computer_use` is a tool, not a skill. A separate curated skill such as `desktop-computer-use` can document workflows around that tool, but it does not replace the compiled runtime tool itself."
        ));
    }

    if looks_like_loaded_skills_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        if skill_names.is_empty() {
            return Some(
                "I don’t have any advertised skills loaded in the current runtime prompt."
                    .to_string(),
            );
        }

        let visible = skill_names
            .iter()
            .take(8)
            .map(|name| format!("- `{name}`"))
            .collect::<Vec<_>>()
            .join("\n");
        let remaining = skill_names.len().saturating_sub(8);
        let suffix = if remaining > 0 {
            format!("\n- ... {remaining} more")
        } else {
            String::new()
        };

        return Some(format!(
            "I currently have {} advertised skills loaded:\n{}{suffix}\n\nAsk about any one by name, or just tell me the task and I’ll use what fits.",
            skill_names.len(),
            visible
        ));
    }

    if looks_like_audio_capability_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let skill_creator_suffix = if skill_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("skill-creator"))
        {
            " If you want a reusable workflow around local audio files, I can also use `skill-creator` to help scaffold one."
        } else {
            ""
        };

        let attachment_prefix = if content.to_ascii_lowercase().contains("[audio:") {
            "That looks like an attached audio file, not a plain-text document. "
        } else {
            "An m4a/audio file is not plain text. "
        };

        return Some(format!(
            "{attachment_prefix}I can’t read it the way I would a `.txt` or `.md` file. \
             If channel transcription is enabled, I can transcribe supported audio uploads \
             such as m4a, mp3, wav, flac, ogg, opus, and webm, then summarize or analyze the transcript. \
             If transcription is not enabled, I need a transcription tool or service first.{skill_creator_suffix}"
        ));
    }

    if looks_like_skill_workflow_advisory_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let has_skill_creator = skill_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("skill-creator"));
        let skill_creator_step = if has_skill_creator {
            "If no installed or curated local skill fits, I would use `skill-creator` next."
        } else {
            "If no installed or curated local skill fits, I would scaffold the new skill directly in the workspace."
        };

        return Some(format!(
            "I would check installed and curated local skills first, not `skills.sh` and not any desktop/browser tool. {skill_creator_step} \
             For a planning-only question like this, I do not need `skills.sh` as the first step."
        ));
    }

    None
}

pub(super) fn should_answer_local_capability_response_immediately(content: &str) -> bool {
    looks_like_current_model_question(content)
        || looks_like_tools_and_skills_question(content)
        || looks_like_tool_inventory_question(content)
        || looks_like_computer_use_availability_question(content)
        || looks_like_loaded_skills_question(content)
        || looks_like_audio_capability_question(content)
        || looks_like_skill_workflow_advisory_question(content)
}

pub(super) fn is_heartbeat_ok_sentinel(output: &str) -> bool {
    const HEARTBEAT_OK: &str = "HEARTBEAT_OK";
    output
        .trim_start()
        .get(..HEARTBEAT_OK.len())
        .map(|prefix| prefix.eq_ignore_ascii_case(HEARTBEAT_OK))
        .unwrap_or(false)
}

pub(super) fn is_agent_noop_sentinel(output: &str) -> bool {
    output.trim().eq_ignore_ascii_case("no_reply") || is_heartbeat_ok_sentinel(output)
}

pub(super) fn load_cached_model_preview(workspace_dir: &Path, provider_name: &str) -> Vec<String> {
    let cache_path = workspace_dir.join("state").join(MODEL_CACHE_FILE);
    let Ok(raw) = std::fs::read_to_string(cache_path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<ModelCacheState>(&raw) else {
        return Vec::new();
    };

    state
        .entries
        .into_iter()
        .find(|entry| entry.provider == provider_name)
        .map(|entry| {
            entry
                .models
                .into_iter()
                .take(MODEL_CACHE_PREVIEW_LIMIT)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) async fn get_or_create_provider(
    ctx: &ChannelRuntimeContext,
    provider_name: &str,
) -> anyhow::Result<Arc<dyn Provider>> {
    if let Some(existing) = ctx
        .provider_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(provider_name)
        .cloned()
    {
        return Ok(existing);
    }

    if provider_name == ctx.default_provider.as_str() {
        return Ok(Arc::clone(&ctx.provider));
    }

    let defaults = super::runtime_config::runtime_defaults_snapshot(ctx);
    let api_url = if provider_name == defaults.default_provider.as_str() {
        defaults.api_url.as_deref()
    } else {
        None
    };

    let provider = create_resilient_provider_nonblocking(
        provider_name,
        ctx.api_key.clone(),
        api_url.map(ToString::to_string),
        ctx.reliability.as_ref().clone(),
        ctx.provider_runtime_options.clone(),
    )
    .await?;
    let provider: Arc<dyn Provider> = Arc::from(provider);

    if let Err(err) = provider.warmup().await {
        tracing::warn!(provider = provider_name, "Provider warmup failed: {err}");
    }

    let mut cache = ctx.provider_cache.lock().unwrap_or_else(|e| e.into_inner());
    let cached = cache
        .entry(provider_name.to_string())
        .or_insert_with(|| Arc::clone(&provider));
    Ok(Arc::clone(cached))
}

pub(super) async fn create_resilient_provider_nonblocking(
    provider_name: &str,
    api_key: Option<String>,
    api_url: Option<String>,
    reliability: crate::config::ReliabilityConfig,
    provider_runtime_options: providers::ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    let provider_name = provider_name.to_string();
    tokio::task::spawn_blocking(move || {
        providers::create_resilient_provider_with_options(
            &provider_name,
            api_key.as_deref(),
            api_url.as_deref(),
            &reliability,
            &provider_runtime_options,
        )
    })
    .await
    .context("failed to join provider initialization task")?
}

pub(super) async fn create_routed_provider_nonblocking(
    provider_name: &str,
    api_key: Option<String>,
    api_url: Option<String>,
    reliability: crate::config::ReliabilityConfig,
    model_routes: Vec<crate::config::ModelRouteConfig>,
    default_model: String,
    provider_runtime_options: providers::ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    let provider_name = provider_name.to_string();
    tokio::task::spawn_blocking(move || {
        providers::create_routed_provider_with_options(
            &provider_name,
            api_key.as_deref(),
            api_url.as_deref(),
            &reliability,
            &model_routes,
            &default_model,
            &provider_runtime_options,
        )
    })
    .await
    .context("failed to join routed provider initialization task")?
}

pub(super) async fn build_memory_context(
    mem: &dyn memory::Memory,
    user_msg: &str,
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();

    if let Ok(entries) = mem.recall(user_msg, 5, None).await {
        let mut included = 0usize;
        let mut used_chars = 0usize;

        for entry in entries.iter().filter(|e| match e.score {
            Some(score) => score >= min_relevance_score,
            None => true, // keep entries without a score (e.g. non-vector backends)
        }) {
            if included >= MEMORY_CONTEXT_MAX_ENTRIES {
                break;
            }

            if should_skip_memory_context_entry(&entry.key, &entry.content) {
                continue;
            }

            let content = if entry.content.chars().count() > MEMORY_CONTEXT_ENTRY_MAX_CHARS {
                crate::util::truncate_with_ellipsis(&entry.content, MEMORY_CONTEXT_ENTRY_MAX_CHARS)
            } else {
                entry.content.clone()
            };

            let line = format!("- {}: {}\n", entry.key, content);
            let line_chars = line.chars().count();
            if used_chars + line_chars > MEMORY_CONTEXT_MAX_CHARS {
                break;
            }

            if included == 0 {
                context.push_str("[Memory context]\n");
            }

            context.push_str(&line);
            used_chars += line_chars;
            included += 1;
        }

        if included > 0 {
            context.push('\n');
        }
    }

    context
}

/// Extract a compact summary of tool interactions from history messages added
/// during `run_tool_call_loop`. Scans assistant messages for `<tool_call>` tags
/// or native tool-call JSON to collect tool names used.
/// Returns an empty string when no tools were invoked.
pub(super) fn extract_tool_context_summary(history: &[ChatMessage], start_index: usize) -> String {
    fn push_unique_tool_name(tool_names: &mut Vec<String>, name: &str) {
        let candidate = name.trim();
        if candidate.is_empty() {
            return;
        }
        if !tool_names.iter().any(|existing| existing == candidate) {
            tool_names.push(candidate.to_string());
        }
    }

    fn collect_tool_names_from_tool_call_tags(content: &str, tool_names: &mut Vec<String>) {
        const TAG_PAIRS: [(&str, &str); 4] = [
            ("<tool_call>", "</tool_call>"),
            ("<toolcall>", "</toolcall>"),
            ("<tool-call>", "</tool-call>"),
            ("<invoke>", "</invoke>"),
        ];

        for (open_tag, close_tag) in TAG_PAIRS {
            for segment in content.split(open_tag) {
                if let Some(json_end) = segment.find(close_tag) {
                    let json_str = segment[..json_end].trim();
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                            push_unique_tool_name(tool_names, name);
                        }
                    }
                }
            }
        }
    }

    fn collect_tool_names_from_native_json(content: &str, tool_names: &mut Vec<String>) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(calls) = val.get("tool_calls").and_then(|c| c.as_array()) {
                for call in calls {
                    let name = call
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .or_else(|| call.get("name").and_then(|n| n.as_str()));
                    if let Some(name) = name {
                        push_unique_tool_name(tool_names, name);
                    }
                }
            }
        }
    }

    fn collect_tool_names_from_tool_results(content: &str, tool_names: &mut Vec<String>) {
        let marker = "<tool_result name=\"";
        let mut remaining = content;
        while let Some(start) = remaining.find(marker) {
            let name_start = start + marker.len();
            let after_name_start = &remaining[name_start..];
            if let Some(name_end) = after_name_start.find('"') {
                let name = &after_name_start[..name_end];
                push_unique_tool_name(tool_names, name);
                remaining = &after_name_start[name_end + 1..];
            } else {
                break;
            }
        }
    }

    let mut tool_names: Vec<String> = Vec::new();

    for msg in history.iter().skip(start_index) {
        match msg.role.as_str() {
            "assistant" => {
                collect_tool_names_from_tool_call_tags(&msg.content, &mut tool_names);
                collect_tool_names_from_native_json(&msg.content, &mut tool_names);
            }
            "user" => {
                // Prompt-mode tool calls are always followed by [Tool results] entries
                // containing `<tool_result name="...">` tags with canonical tool names.
                collect_tool_names_from_tool_results(&msg.content, &mut tool_names);
            }
            _ => {}
        }
    }

    if tool_names.is_empty() {
        return String::new();
    }

    format!("[Used tools: {}]", tool_names.join(", "))
}

pub(super) fn compute_max_in_flight_messages(channel_count: usize) -> usize {
    channel_count
        .saturating_mul(CHANNEL_PARALLELISM_PER_CHANNEL)
        .clamp(
            CHANNEL_MIN_IN_FLIGHT_MESSAGES,
            CHANNEL_MAX_IN_FLIGHT_MESSAGES,
        )
}
