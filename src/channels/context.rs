//! Channel runtime context and shared types.

use crate::approval::ApprovalManager;
use crate::memory::Memory;
use crate::observability::Observer;
use crate::providers::{self, Provider};
use crate::tools::Tool;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use super::traits;

/// Per-sender conversation history for channel messages.
pub(super) type ConversationHistoryMap =
    Arc<Mutex<HashMap<String, Vec<crate::providers::ChatMessage>>>>;
/// Provider cache mapping provider names to provider instances.
pub(super) type ProviderCacheMap = Arc<Mutex<HashMap<String, Arc<dyn Provider>>>>;
/// Route selection overrides per sender.
pub(super) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

/// Maximum history messages to keep per sender.
pub(super) const MAX_CHANNEL_HISTORY: usize = 50;
/// Minimum user-message length (in chars) for auto-save to memory.
/// Messages shorter than this (e.g. "ok", "thanks") are not stored,
/// reducing noise in memory recall.
pub(super) const AUTOSAVE_MIN_MESSAGE_CHARS: usize = 20;

/// Maximum characters per injected workspace file.
pub(crate) const BOOTSTRAP_MAX_CHARS: usize = 20_000;

pub(super) const DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS: u64 = 2;
pub(super) const DEFAULT_CHANNEL_MAX_BACKOFF_SECS: u64 = 60;
pub(super) const MIN_CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 30;
/// Default timeout for processing a single channel message (LLM + tools).
/// Used as fallback when not configured in channels_config.message_timeout_secs.
#[cfg(test)]
pub(super) const CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 300;
/// Cap timeout scaling so large max_tool_iterations values do not create unbounded waits.
pub(super) const CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP: u64 = 4;
pub(super) const CHANNEL_PARALLELISM_PER_CHANNEL: usize = 4;
pub(super) const CHANNEL_MIN_IN_FLIGHT_MESSAGES: usize = 8;
pub(super) const CHANNEL_MAX_IN_FLIGHT_MESSAGES: usize = 64;
pub(super) const CHANNEL_TYPING_REFRESH_INTERVAL_SECS: u64 = 4;
pub(super) const CHANNEL_PROGRESS_HEARTBEAT_SECS: u64 = 15;
pub(super) const CHANNEL_HEALTH_HEARTBEAT_SECS: u64 = 30;
pub(super) const MODEL_CACHE_FILE: &str = "models_cache.json";
pub(super) const MODEL_CACHE_PREVIEW_LIMIT: usize = 10;
pub(super) const MEMORY_CONTEXT_MAX_ENTRIES: usize = 4;
pub(super) const MEMORY_CONTEXT_ENTRY_MAX_CHARS: usize = 800;
pub(super) const MEMORY_CONTEXT_MAX_CHARS: usize = 4_000;
pub(super) const CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES: usize = 12;
pub(super) const CHANNEL_HISTORY_COMPACT_CONTENT_CHARS: usize = 600;
/// Guardrail for hook-modified outbound channel content.
pub(super) const CHANNEL_HOOK_MAX_OUTBOUND_CHARS: usize = 20_000;

pub(super) const SYSTEMD_STATUS_ARGS: [&str; 3] = ["--user", "is-active", "topclaw.service"];
pub(super) const SYSTEMD_RESTART_ARGS: [&str; 3] = ["--user", "restart", "topclaw.service"];
pub(super) const OPENRC_STATUS_ARGS: [&str; 2] = ["topclaw", "status"];
pub(super) const OPENRC_RESTART_ARGS: [&str; 2] = ["topclaw", "restart"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChannelRouteSelection {
    pub(super) provider: String,
    pub(super) model: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct ModelCacheState {
    pub(super) entries: Vec<ModelCacheEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct ModelCacheEntry {
    pub(super) provider: String,
    pub(super) models: Vec<String>,
}

#[derive(Clone)]
pub(super) struct ChannelRuntimeContext {
    pub(super) channels_by_name: Arc<HashMap<String, Arc<dyn super::traits::Channel>>>,
    pub(super) provider: Arc<dyn Provider>,
    pub(super) default_provider: Arc<String>,
    pub(super) memory: Arc<dyn Memory>,
    pub(super) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    pub(super) observer: Arc<dyn Observer>,
    pub(super) system_prompt: Arc<String>,
    pub(super) model: Arc<String>,
    pub(super) temperature: f64,
    pub(super) auto_save_memory: bool,
    pub(super) max_tool_iterations: usize,
    pub(super) min_relevance_score: f64,
    pub(super) conversation_histories: ConversationHistoryMap,
    pub(super) provider_cache: ProviderCacheMap,
    pub(super) route_overrides: RouteSelectionMap,
    pub(super) api_key: Option<String>,
    pub(super) api_url: Option<String>,
    pub(super) reliability: Arc<crate::config::ReliabilityConfig>,
    pub(super) provider_runtime_options: providers::ProviderRuntimeOptions,
    pub(super) workspace_dir: Arc<PathBuf>,
    pub(super) message_timeout_secs: u64,
    pub(super) interrupt_on_new_message: bool,
    pub(super) multimodal: crate::config::MultimodalConfig,
    pub(super) hooks: Option<Arc<crate::hooks::HookRunner>>,
    pub(super) non_cli_excluded_tools: Arc<Mutex<Vec<String>>>,
    pub(super) query_classification: crate::config::QueryClassificationConfig,
    pub(super) model_routes: Vec<crate::config::ModelRouteConfig>,
    pub(super) approval_manager: Arc<ApprovalManager>,
}

#[derive(Clone)]
pub(super) struct InFlightSenderTaskState {
    pub(super) task_id: u64,
    pub(super) cancellation: CancellationToken,
    pub(super) completion: Arc<InFlightTaskCompletion>,
}

pub(super) struct InFlightTaskCompletion {
    done: AtomicBool,
    notify: tokio::sync::Notify,
}

impl InFlightTaskCompletion {
    pub(super) fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }

    pub(super) fn mark_done(&self) {
        self.done.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub(super) async fn wait(&self) {
        if self.done.load(Ordering::Acquire) {
            return;
        }
        self.notify.notified().await;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProcessChannelMessageOptions {
    pub(super) resume_existing_user_turn: bool,
}

pub(super) fn effective_channel_message_timeout_secs(configured: u64) -> u64 {
    configured.max(MIN_CHANNEL_MESSAGE_TIMEOUT_SECS)
}

pub(super) fn channel_message_timeout_budget_secs(
    message_timeout_secs: u64,
    max_tool_iterations: usize,
) -> u64 {
    let iterations = max_tool_iterations.max(1) as u64;
    let scale = iterations.min(CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP);
    message_timeout_secs.saturating_mul(scale)
}

pub(super) fn conversation_memory_key(msg: &traits::ChannelMessage) -> String {
    // Include thread_ts for per-topic memory isolation in forum groups
    match &msg.thread_ts {
        Some(tid) => format!("{}_{}_{}_{}", msg.channel, tid, msg.sender, msg.id),
        None => format!("{}_{}_{}", msg.channel, msg.sender, msg.id),
    }
}

pub(super) fn conversation_history_key(msg: &traits::ChannelMessage) -> String {
    // Include thread_ts for per-topic session isolation in forum groups
    match &msg.thread_ts {
        Some(tid) => format!("{}_{}_{}", msg.channel, tid, msg.sender),
        None => format!("{}_{}", msg.channel, msg.sender),
    }
}

pub(super) fn interruption_scope_key(msg: &traits::ChannelMessage) -> String {
    format!("{}_{}_{}", msg.channel, msg.reply_target, msg.sender)
}

pub(super) fn log_worker_join_result(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::error!("Channel message worker crashed: {error}");
    }
}
