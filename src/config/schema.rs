use crate::config::autonomy::is_valid_env_var_name;
use crate::config::parse_skills_prompt_injection_mode;
use crate::config::proxy::{
    normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list, parse_proxy_scope,
};
use crate::config::set_runtime_proxy_config;
use crate::config::traits::ChannelConfig;
use crate::config::AgentConfig;
use crate::config::AuditConfig;
use crate::config::AutonomyConfig;
use crate::config::BrowserConfig;
use crate::config::CoordinationConfig;
use crate::config::CostConfig;
use crate::config::CronConfig;
use crate::config::DelegateAgentConfig;
use crate::config::EmbeddingRouteConfig;
use crate::config::EstopConfig;
use crate::config::GatewayConfig;
use crate::config::HeartbeatConfig;
use crate::config::HooksConfig;
use crate::config::HttpRequestConfig;
use crate::config::IdentityConfig;
use crate::config::ModelProviderConfig;
use crate::config::ModelRouteConfig;
use crate::config::MultimodalConfig;
use crate::config::ObservabilityConfig;
use crate::config::OtpConfig;
use crate::config::ProviderConfig;
use crate::config::ProxyConfig;
use crate::config::ProxyScope;
use crate::config::QueryClassificationConfig;
use crate::config::ReliabilityConfig;
use crate::config::ResearchPhaseConfig;
use crate::config::ResourceLimitsConfig;
use crate::config::RuntimeConfig;
use crate::config::SandboxConfig;
use crate::config::SchedulerConfig;
use crate::config::SecretsConfig;
use crate::config::SkillsConfig;
use crate::config::StorageProviderSection;
use crate::config::TranscriptionConfig;
use crate::config::TunnelConfig;
use crate::config::WebFetchConfig;
use crate::config::WebSearchConfig;
use crate::config::WorkspacesConfig;
use crate::providers::is_zai_alias;
use crate::security::DomainMatcher;
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[path = "schema_runtime_dirs.rs"]
mod runtime_dirs;
#[path = "schema_channels.rs"]
mod schema_channels;
#[path = "schema_memory.rs"]
mod schema_memory;
#[path = "schema_provider_profiles.rs"]
mod schema_provider_profiles;
#[path = "schema_secrets.rs"]
mod schema_secrets;
#[path = "schema_security.rs"]
mod schema_security;
#[path = "schema_telegram_allowed_users.rs"]
mod schema_telegram_allowed_users;
#[cfg(unix)]
use tokio::fs::File;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use self::schema_channels::{
    clone_group_reply_allowed_sender_ids, default_channel_message_timeout_secs,
    default_draft_update_interval_ms, default_telegram_stream_mode, resolve_group_reply_mode,
};
use self::schema_memory::{
    default_archive_after_days, default_cache_size, default_chunk_size,
    default_conversation_retention_days, default_embedding_dims, default_embedding_model,
    default_embedding_provider, default_hygiene_enabled, default_keyword_weight,
    default_min_relevance_score, default_purge_after_days, default_response_cache_max,
    default_response_cache_ttl, default_vector_weight,
};
use self::schema_security::{
    default_semantic_guard_collection, default_semantic_guard_threshold,
    default_syscall_anomaly_alert_cooldown_secs, default_syscall_anomaly_baseline_syscalls,
    default_syscall_anomaly_log_path, default_syscall_anomaly_max_alerts_per_minute,
    default_syscall_anomaly_max_denied_events_per_minute,
    default_syscall_anomaly_max_total_events_per_minute,
};
use runtime_dirs::{default_config_and_workspace_dirs, resolve_runtime_config_dirs};
pub use runtime_dirs::default_config_dir;
pub(crate) use runtime_dirs::{
    persist_active_workspace_config_dir, resolve_config_dir_for_workspace,
    resolve_runtime_dirs_for_onboarding,
};
#[cfg(test)]
use runtime_dirs::{ActiveWorkspaceState, ConfigResolutionSource, ACTIVE_WORKSPACE_STATE_FILE};
use schema_telegram_allowed_users::resolve_telegram_allowed_users_env_refs;

pub(crate) const fn default_true() -> bool {
    true
}

// ── Top-level config ──────────────────────────────────────────────

/// Protocol mode for `custom:` OpenAI-compatible providers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderApiMode {
    /// Default behavior: `/chat/completions` first, optional `/responses`
    /// fallback when supported.
    OpenAiChatCompletions,
    /// Responses-first behavior: call `/responses` directly.
    OpenAiResponses,
}

impl ProviderApiMode {
    pub fn as_compatible_mode(self) -> crate::providers::compatible::CompatibleApiMode {
        match self {
            Self::OpenAiChatCompletions => {
                crate::providers::compatible::CompatibleApiMode::OpenAiChatCompletions
            }
            Self::OpenAiResponses => {
                crate::providers::compatible::CompatibleApiMode::OpenAiResponses
            }
        }
    }
}

/// Top-level TopClaw configuration, loaded from `config.toml`.
///
/// Resolution order: `TOPCLAW_WORKSPACE` env → `active_workspace.toml` marker → `~/.topclaw/config.toml`.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    /// Workspace directory - computed from home, not serialized
    #[serde(skip)]
    pub workspace_dir: PathBuf,
    /// Path to config.toml - computed from home, not serialized
    #[serde(skip)]
    pub config_path: PathBuf,
    /// API key for the selected provider. Overridden by `TOPCLAW_API_KEY`.
    pub api_key: Option<String>,
    /// Base URL override for provider API (e.g. "http://10.0.0.1:11434" for remote Ollama)
    pub api_url: Option<String>,
    /// Default provider ID or alias (e.g. `"openai-codex"`, `"openrouter"`, `"ollama"`). Default: `"openai-codex"`.
    pub default_provider: Option<String>,
    /// Optional API protocol mode for `custom:` providers.
    #[serde(default)]
    pub provider_api: Option<ProviderApiMode>,
    /// Default model routed through the selected provider (e.g. `"gpt-5.4"` or `"openai/gpt-5.2"`).
    pub default_model: Option<String>,
    /// Optional named provider profiles keyed by id (Codex app-server compatible layout).
    #[serde(default)]
    pub model_providers: HashMap<String, ModelProviderConfig>,
    /// Provider-specific behavior overrides (`[provider]`).
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Default model temperature (0.0–2.0). Default: `0.7`.
    pub default_temperature: f64,

    /// Observability backend configuration (`[observability]`).
    #[serde(default)]
    pub observability: ObservabilityConfig,

    /// Autonomy and security policy configuration (`[autonomy]`).
    #[serde(default)]
    pub autonomy: AutonomyConfig,

    /// Security subsystem configuration (`[security]`).
    #[serde(default)]
    pub security: SecurityConfig,

    /// Runtime adapter configuration (`[runtime]`). Controls native vs Docker execution.
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Research phase configuration (`[research]`). Proactive information gathering.
    #[serde(default)]
    pub research: ResearchPhaseConfig,

    /// Reliability settings: retries, fallback providers, backoff (`[reliability]`).
    #[serde(default)]
    pub reliability: ReliabilityConfig,

    /// Scheduler configuration for periodic task execution (`[scheduler]`).
    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// Agent orchestration settings (`[agent]`).
    #[serde(default)]
    pub agent: AgentConfig,

    /// Multi-workspace routing and registry settings (`[workspaces]`).
    #[serde(default)]
    pub workspaces: WorkspacesConfig,

    /// Skills loading and community repository behavior (`[skills]`).
    #[serde(default)]
    pub skills: SkillsConfig,

    /// Model routing rules — route `hint:<name>` to specific provider+model combos.
    #[serde(default)]
    pub model_routes: Vec<ModelRouteConfig>,

    /// Embedding routing rules — route `hint:<name>` to specific provider+model combos.
    #[serde(default)]
    pub embedding_routes: Vec<EmbeddingRouteConfig>,

    /// Automatic query classification — maps user messages to model hints.
    #[serde(default)]
    pub query_classification: QueryClassificationConfig,

    /// Heartbeat configuration for periodic health pings (`[heartbeat]`).
    #[serde(default)]
    pub heartbeat: HeartbeatConfig,

    /// Cron job configuration (`[cron]`).
    #[serde(default)]
    pub cron: CronConfig,

    /// Channel configurations: Telegram, Discord, etc. (`[channels_config]`).
    #[serde(default)]
    pub channels_config: ChannelsConfig,

    /// Memory backend configuration: sqlite, markdown, embeddings (`[memory]`).
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Persistent storage provider configuration (`[storage]`).
    #[serde(default)]
    pub storage: StorageConfig,

    /// Tunnel configuration for exposing the gateway publicly (`[tunnel]`).
    #[serde(default)]
    pub tunnel: TunnelConfig,

    /// Gateway server configuration: host, port, pairing, rate limits (`[gateway]`).
    #[serde(default)]
    pub gateway: GatewayConfig,

    /// Secrets encryption configuration (`[secrets]`).
    #[serde(default)]
    pub secrets: SecretsConfig,

    /// Browser automation configuration (`[browser]`).
    #[serde(default)]
    pub browser: BrowserConfig,

    /// HTTP request tool configuration (`[http_request]`).
    #[serde(default)]
    pub http_request: HttpRequestConfig,

    /// Multimodal (image) handling configuration (`[multimodal]`).
    #[serde(default)]
    pub multimodal: MultimodalConfig,

    /// Web fetch tool configuration (`[web_fetch]`).
    #[serde(default)]
    pub web_fetch: WebFetchConfig,

    /// Web search tool configuration (`[web_search]`).
    #[serde(default)]
    pub web_search: WebSearchConfig,

    /// Proxy configuration for outbound HTTP/HTTPS/SOCKS5 traffic (`[proxy]`).
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Identity format configuration (`[identity]`).
    #[serde(default)]
    pub identity: IdentityConfig,

    /// Cost tracking and budget enforcement configuration (`[cost]`).
    #[serde(default)]
    pub cost: CostConfig,

    /// Delegate agent configurations for multi-agent workflows.
    #[serde(default)]
    pub agents: HashMap<String, DelegateAgentConfig>,

    /// Delegate coordination runtime configuration (`[coordination]`).
    #[serde(default)]
    pub coordination: CoordinationConfig,

    /// Hooks configuration (lifecycle hooks and built-in hook toggles).
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Voice transcription configuration (Whisper API via Groq).
    #[serde(default)]
    pub transcription: TranscriptionConfig,

    /// Vision support override for the active provider/model.
    /// - `None` (default): use provider's built-in default
    /// - `Some(true)`: force vision support on (e.g. Ollama running llava)
    /// - `Some(false)`: force vision support off
    #[serde(default)]
    pub model_support_vision: Option<bool>,
}

// ── Delegate Agents ──────────────────────────────────────────────

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let model_provider_ids: Vec<&str> =
            self.model_providers.keys().map(String::as_str).collect();
        let delegate_agent_ids: Vec<&str> = self.agents.keys().map(String::as_str).collect();
        let enabled_channel_count = [
            self.channels_config.telegram.is_some(),
            self.channels_config.discord.is_some(),
            self.channels_config.webhook.is_some(),
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();

        f.debug_struct("Config")
            .field("workspace_dir", &self.workspace_dir)
            .field("config_path", &self.config_path)
            .field("api_key_configured", &self.api_key.is_some())
            .field("api_url_configured", &self.api_url.is_some())
            .field("default_provider", &self.default_provider)
            .field("provider_api", &self.provider_api)
            .field("default_model", &self.default_model)
            .field("model_providers", &model_provider_ids)
            .field("default_temperature", &self.default_temperature)
            .field("model_routes_count", &self.model_routes.len())
            .field("embedding_routes_count", &self.embedding_routes.len())
            .field("delegate_agents", &delegate_agent_ids)
            .field("cli_channel_enabled", &self.channels_config.cli)
            .field("enabled_channels_count", &enabled_channel_count)
            .field("sensitive_sections", &"***REDACTED***")
            .finish_non_exhaustive()
    }
}

// ── Transcription ────────────────────────────────────────────────

// ── Cost tracking and budget enforcement ───────────────────────────

// ── Composio (managed tool surface) ─────────────────────────────

// ── Secrets (encrypted credential store) ────────────────────────

// ── Browser (friendly-service browsing only) ───────────────────

// ── HTTP request tool ───────────────────────────────────────────

// ── Web fetch ────────────────────────────────────────────────────

/// Web fetch tool configuration (`[web_fetch]` section).
///
fn parse_proxy_enabled(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
// ── Memory ───────────────────────────────────────────────────

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: Storage & Memory Configs
// ═══════════════════════════════════════════════════════════════════════════

/// Persistent storage configuration (`[storage]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageConfig {
    /// Storage provider settings.
    #[serde(default)]
    pub provider: StorageProviderSection,
}

/// Memory backend configuration (`[memory]` section).
///
/// Controls conversation memory storage, embeddings, hybrid search, response caching,
/// and memory snapshot/hydration.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct MemoryConfig {
    /// "sqlite" | "markdown" | "none" (`none` = explicit no-op memory)
    pub backend: String,
    /// Auto-save user-stated conversation input to memory (assistant output is excluded)
    pub auto_save: bool,
    /// Run memory/session hygiene (archiving + retention cleanup)
    #[serde(default = "default_hygiene_enabled")]
    pub hygiene_enabled: bool,
    /// Archive daily/session files older than this many days
    #[serde(default = "default_archive_after_days")]
    pub archive_after_days: u32,
    /// Purge archived files older than this many days
    #[serde(default = "default_purge_after_days")]
    pub purge_after_days: u32,
    /// For sqlite backend: prune conversation rows older than this many days
    #[serde(default = "default_conversation_retention_days")]
    pub conversation_retention_days: u32,
    /// Embedding provider: "none" | "openai" | "custom:URL"
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    /// Embedding model name (e.g. "text-embedding-3-small")
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Embedding vector dimensions
    #[serde(default = "default_embedding_dims")]
    pub embedding_dimensions: usize,
    /// Weight for vector similarity in hybrid search (0.0–1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// Weight for keyword BM25 in hybrid search (0.0–1.0)
    #[serde(default = "default_keyword_weight")]
    pub keyword_weight: f64,
    /// Minimum hybrid score (0.0–1.0) for a memory to be included in context.
    /// Memories scoring below this threshold are dropped to prevent irrelevant
    /// context from bleeding into conversations. Default: 0.4
    #[serde(default = "default_min_relevance_score")]
    pub min_relevance_score: f64,
    /// Max embedding cache entries before LRU eviction
    #[serde(default = "default_cache_size")]
    pub embedding_cache_size: usize,
    /// Max tokens per chunk for document splitting
    #[serde(default = "default_chunk_size")]
    pub chunk_max_tokens: usize,

    // ── Response Cache (saves tokens on repeated prompts) ──────
    /// Enable LLM response caching to avoid paying for duplicate prompts
    #[serde(default)]
    pub response_cache_enabled: bool,
    /// TTL in minutes for cached responses (default: 60)
    #[serde(default = "default_response_cache_ttl")]
    pub response_cache_ttl_minutes: u32,
    /// Max number of cached responses before LRU eviction (default: 5000)
    #[serde(default = "default_response_cache_max")]
    pub response_cache_max_entries: usize,

    // ── Memory Snapshot (soul backup to Markdown) ─────────────
    /// Enable periodic export of core memories to MEMORY_SNAPSHOT.md
    #[serde(default)]
    pub snapshot_enabled: bool,
    /// Run snapshot during hygiene passes (heartbeat-driven)
    #[serde(default)]
    pub snapshot_on_hygiene: bool,
    /// Auto-hydrate from MEMORY_SNAPSHOT.md when brain.db is missing
    #[serde(default = "default_true")]
    pub auto_hydrate: bool,

    // ── SQLite backend options ─────────────────────────────────
    /// For sqlite backend: max seconds to wait when opening the DB (e.g. file locked).
    /// None = wait indefinitely (default). Recommended max: 300.
    #[serde(default)]
    pub sqlite_open_timeout_secs: Option<u64>,


}

// ── Hooks ────────────────────────────────────────────────────────

// ── Autonomy / Security ──────────────────────────────────────────

// ── Research Phase ───────────────────────────────────────────────

// ── Reliability / supervision ────────────────────────────────────

// ── Scheduler ────────────────────────────────────────────────────

// ── Model routing ────────────────────────────────────────────────

// ── Embedding routing ───────────────────────────────────────────

// Route an embedding hint to a specific provider + model.
//
// ```toml
// [[embedding_routes]]
// hint = "semantic"
// provider = "openai"
// model = "text-embedding-3-small"
// dimensions = 1536
//
// [memory]
// embedding_model = "hint:semantic"
// ```
// ── Query Classification ─────────────────────────────────────────

// ── Heartbeat ────────────────────────────────────────────────────

// ── Channels ─────────────────────────────────────────────────────

/// Top-level channel configurations (`[channels_config]` section).
///
/// Each channel sub-section (e.g. `telegram`, `discord`) is optional;
/// setting it to `Some(...)` enables that channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelsConfig {
    /// Enable the CLI interactive channel. Default: `true`.
    pub cli: bool,
    /// Telegram bot channel configuration.
    pub telegram: Option<TelegramConfig>,
    /// Discord bot channel configuration.
    pub discord: Option<DiscordConfig>,
    /// Webhook channel configuration.
    pub webhook: Option<WebhookConfig>,
    /// Base timeout in seconds for processing a single channel message (LLM + tools).
    /// Runtime uses this as a per-turn budget that scales with tool-loop depth
    /// (up to 4x, capped) so one slow/retried model call does not consume the
    /// entire conversation budget.
    /// Default: 300s for on-device LLMs (Ollama) which are slower than cloud APIs.
    #[serde(default = "default_channel_message_timeout_secs")]
    pub message_timeout_secs: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: Channel Configs
// ═══════════════════════════════════════════════════════════════════════════

/// Streaming mode for channels that support progressive message updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    /// No streaming -- send the complete response as a single message (default).
    #[default]
    Off,
    /// Update a draft message with every flush interval.
    Partial,
}

/// Group-chat reply trigger mode for channels that support mention gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupReplyMode {
    /// Reply only when the bot is explicitly @-mentioned in group chats.
    MentionOnly,
    /// Reply to every message in group chats.
    AllMessages,
}

impl GroupReplyMode {
    #[must_use]
    pub fn requires_mention(self) -> bool {
        matches!(self, Self::MentionOnly)
    }
}

/// Advanced group-chat trigger controls.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct GroupReplyConfig {
    /// Optional explicit trigger mode.
    #[serde(default)]
    pub mode: Option<GroupReplyMode>,
    /// Sender IDs that always trigger group replies.
    ///
    /// These IDs bypass mention gating in group chats, but do not bypass the
    /// channel-level inbound allowlist (`allowed_users` / equivalents).
    #[serde(default)]
    pub allowed_sender_ids: Vec<String>,
}

/// Telegram bot channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TelegramConfig {
    /// Telegram Bot API token (from @BotFather).
    pub bot_token: String,
    /// Allowed Telegram user IDs or usernames. Empty = deny all.
    pub allowed_users: Vec<String>,
    /// Streaming mode for progressive response delivery via native drafts or message edits.
    #[serde(default = "default_telegram_stream_mode")]
    pub stream_mode: StreamMode,
    /// Minimum interval (ms) between draft message edits to avoid rate limits.
    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// When true, a newer Telegram message from the same sender in the same chat
    /// cancels the in-flight request and starts a fresh response with preserved history.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
    /// Optional custom base URL for Telegram-compatible APIs.
    /// Defaults to "https://api.telegram.org" when omitted.
    /// Example for Bale messenger: "https://tapi.bale.ai"
    #[serde(default)]
    pub base_url: Option<String>,
}

impl ChannelConfig for TelegramConfig {
    fn name() -> &'static str {
        "Telegram"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

impl TelegramConfig {
    #[must_use]
    pub fn effective_group_reply_mode(&self) -> GroupReplyMode {
        resolve_group_reply_mode(self.group_reply.as_ref(), GroupReplyMode::AllMessages)
    }

    #[must_use]
    pub fn group_reply_allowed_sender_ids(&self) -> Vec<String> {
        clone_group_reply_allowed_sender_ids(self.group_reply.as_ref())
    }
}

/// Discord bot channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordConfig {
    /// Discord bot token (from Discord Developer Portal).
    pub bot_token: String,
    /// Optional guild (server) ID to restrict the bot to a single guild.
    pub guild_id: Option<String>,
    /// Allowed Discord user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true, process messages from other bots (not just humans).
    /// The bot still ignores its own messages to prevent feedback loops.
    #[serde(default)]
    pub listen_to_bots: bool,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
}

impl ChannelConfig for DiscordConfig {
    fn name() -> &'static str {
        "Discord"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

impl DiscordConfig {
    #[must_use]
    pub fn effective_group_reply_mode(&self) -> GroupReplyMode {
        resolve_group_reply_mode(self.group_reply.as_ref(), GroupReplyMode::AllMessages)
    }

    #[must_use]
    pub fn group_reply_allowed_sender_ids(&self) -> Vec<String> {
        clone_group_reply_allowed_sender_ids(self.group_reply.as_ref())
    }
}

/// Webhook channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebhookConfig {
    /// Port to listen on for incoming webhooks.
    pub port: u16,
    /// Optional shared secret for webhook signature verification.
    pub secret: Option<String>,
}

impl ChannelConfig for WebhookConfig {
    fn name() -> &'static str {
        "Webhook"
    }
    fn desc() -> &'static str {
        "HTTP endpoint"
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: Security Config
// ═══════════════════════════════════════════════════════════════════════════

// ── Security Config ─────────────────────────────────────────────────

/// Security configuration for sandboxing, resource limits, and audit logging
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecurityConfig {
    /// Sandbox configuration
    #[serde(default)]
    pub sandbox: SandboxConfig,

    /// Resource limits
    #[serde(default)]
    pub resources: ResourceLimitsConfig,

    /// Audit logging configuration
    #[serde(default)]
    pub audit: AuditConfig,

    /// OTP gating configuration for sensitive actions/domains.
    #[serde(default)]
    pub otp: OtpConfig,

    /// Emergency-stop state machine configuration.
    #[serde(default)]
    pub estop: EstopConfig,

    /// Syscall anomaly detection profile for daemon shell/process execution.
    #[serde(default)]
    pub syscall_anomaly: SyscallAnomalyConfig,

    /// Enable per-turn canary token injection to detect context exfiltration.
    #[serde(default = "default_true")]
    pub canary_tokens: bool,

    /// Enable semantic prompt-injection guard backed by vector similarity.
    #[serde(default)]
    pub semantic_guard: bool,

    /// Collection name used by semantic guard in the vector store.
    #[serde(default = "default_semantic_guard_collection")]
    pub semantic_guard_collection: String,

    /// Similarity threshold (0.0-1.0) used to block semantic prompt-injection matches.
    #[serde(default = "default_semantic_guard_threshold")]
    pub semantic_guard_threshold: f64,
}

/// Syscall anomaly detection configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SyscallAnomalyConfig {
    /// Enable syscall anomaly detection.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Treat denied syscall lines as anomalies even when syscall is in baseline.
    #[serde(default)]
    pub strict_mode: bool,

    /// Emit anomaly alerts when a syscall appears outside the expected baseline.
    #[serde(default = "default_true")]
    pub alert_on_unknown_syscall: bool,

    /// Allowed denied-syscall events per rolling minute before triggering an alert.
    #[serde(default = "default_syscall_anomaly_max_denied_events_per_minute")]
    pub max_denied_events_per_minute: u32,

    /// Allowed total syscall telemetry events per rolling minute before triggering an alert.
    #[serde(default = "default_syscall_anomaly_max_total_events_per_minute")]
    pub max_total_events_per_minute: u32,

    /// Maximum anomaly alerts emitted per rolling minute (global guardrail).
    #[serde(default = "default_syscall_anomaly_max_alerts_per_minute")]
    pub max_alerts_per_minute: u32,

    /// Cooldown between identical anomaly alerts (seconds).
    #[serde(default = "default_syscall_anomaly_alert_cooldown_secs")]
    pub alert_cooldown_secs: u64,

    /// Path to syscall anomaly log file (relative to ~/.topclaw unless absolute).
    #[serde(default = "default_syscall_anomaly_log_path")]
    pub log_path: String,

    /// Expected syscall baseline. Unknown syscall names trigger anomaly when enabled.
    #[serde(default = "default_syscall_anomaly_baseline_syscalls")]
    pub baseline_syscalls: Vec<String>,
}

// ── Config impl ──────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        let topclaw_dir = default_config_dir()
            .unwrap_or_else(|_| PathBuf::from(".topclaw"));

        Self {
            workspace_dir: topclaw_dir.join("workspace"),
            config_path: topclaw_dir.join("config.toml"),
            api_key: None,
            api_url: None,
            default_provider: Some(crate::providers::DEFAULT_PROVIDER_NAME.to_string()),
            provider_api: None,
            default_model: Some(crate::providers::DEFAULT_PROVIDER_MODEL.to_string()),
            model_providers: HashMap::new(),
            provider: ProviderConfig::default(),
            default_temperature: 0.7,
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            security: SecurityConfig::default(),
            runtime: RuntimeConfig::default(),
            research: ResearchPhaseConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            agent: AgentConfig::default(),
            workspaces: WorkspacesConfig::default(),
            skills: SkillsConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),

            identity: IdentityConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            cost: CostConfig::default(),
            agents: HashMap::new(),
            coordination: CoordinationConfig::default(),
            hooks: HooksConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            transcription: TranscriptionConfig::default(),
            model_support_vision: None,
        }
    }
}

fn config_dir_creation_error(path: &Path) -> String {
    format!(
        "Failed to create config directory: {}. If running as an OpenRC service, \
         ensure this path is writable by user 'topclaw'.",
        path.display()
    )
}



impl Config {
    pub async fn load_or_init() -> Result<Self> {
        let (default_topclaw_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;

        let (topclaw_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs(&default_topclaw_dir, &default_workspace_dir).await?;

        let config_path = topclaw_dir.join("config.toml");

        fs::create_dir_all(&topclaw_dir)
            .await
            .with_context(|| config_dir_creation_error(&topclaw_dir))?;
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create workspace directory")?;

        if config_path.exists() {
            // Warn if config file is world-readable (may contain API keys)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(&config_path).await {
                    if meta.permissions().mode() & 0o004 != 0 {
                        tracing::warn!(
                            "Config file {:?} is world-readable (mode {:o}). \
                             Consider restricting with: chmod 600 {:?}",
                            config_path,
                            meta.permissions().mode() & 0o777,
                            config_path,
                        );
                    }
                }
            }

            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;
            let raw_toml: toml::Value =
                toml::from_str(&contents).context("Failed to parse config file")?;
            let normalized_contents =
                toml::to_string(&raw_toml).context("Failed to normalize config file")?;

            // Track ignored/unknown config keys to warn users about silent misconfigurations
            // (e.g., using [providers.ollama] which doesn't exist instead of top-level api_url)
            let mut ignored_paths: Vec<String> = Vec::new();
            let mut config: Config = serde_ignored::deserialize(
                toml::de::Deserializer::parse(&normalized_contents)
                    .context("Failed to parse config file")?,
                |path| {
                    ignored_paths.push(path.to_string());
                },
            )
            .context("Failed to deserialize config file")?;

            // Warn about each unknown config key
            for path in ignored_paths {
                tracing::warn!(
                    "Unknown config key ignored: \"{}\". Check config.toml for typos or deprecated options.",
                    path
                );
            }
            // Set computed paths that are skipped during serialization
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            schema_secrets::decrypt_config_secrets(&topclaw_dir, &mut config)?;
            resolve_telegram_allowed_users_env_refs(&mut config.channels_config)?;

            config.apply_env_overrides();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = false,
                "Config loaded"
            );
            Ok(config)
        } else {
            let mut config = Config::default();
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            config.save().await?;

            // Restrict permissions on newly created config file (may contain API keys)
            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        }
    }

    fn normalize_reasoning_level_override(raw: Option<&str>, source: &str) -> Option<String> {
        let value = raw?.trim();
        if value.is_empty() {
            return None;
        }
        let normalized = value.to_ascii_lowercase().replace(['-', '_'], "");
        match normalized.as_str() {
            "minimal" | "low" | "medium" | "high" | "xhigh" => Some(normalized),
            _ => {
                tracing::warn!(
                    reasoning_level = %value,
                    source,
                    "Ignoring invalid reasoning level override"
                );
                None
            }
        }
    }

    /// Resolve provider reasoning level from the canonical provider config.
    pub fn effective_provider_reasoning_level(&self) -> Option<String> {
        Self::normalize_reasoning_level_override(
            self.provider.reasoning_level.as_deref(),
            "provider.reasoning_level",
        )
    }

    /// Validate configuration values that would cause runtime failures.
    ///
    /// Called after TOML deserialization and env-override application to catch
    /// obviously invalid values early instead of failing at arbitrary runtime points.
    pub fn validate(&self) -> Result<()> {
        // Gateway
        if self.gateway.host.trim().is_empty() {
            anyhow::bail!("gateway.host must not be empty");
        }

        // Reliability
        let configured_fallbacks = self
            .reliability
            .fallback_providers
            .iter()
            .map(|provider| provider.trim())
            .filter(|provider| !provider.is_empty())
            .collect::<std::collections::HashSet<_>>();
        for (entry, api_key) in &self.reliability.fallback_api_keys {
            let normalized_entry = entry.trim();
            if normalized_entry.is_empty() {
                anyhow::bail!("reliability.fallback_api_keys contains an empty key");
            }
            if api_key.trim().is_empty() {
                anyhow::bail!("reliability.fallback_api_keys.{normalized_entry} must not be empty");
            }
            if !configured_fallbacks.contains(normalized_entry) {
                anyhow::bail!(
                    "reliability.fallback_api_keys.{normalized_entry} has no matching entry in reliability.fallback_providers"
                );
            }
        }

        // Autonomy
        if self.autonomy.max_actions_per_hour == 0 {
            anyhow::bail!("autonomy.max_actions_per_hour must be greater than 0");
        }
        for (i, env_name) in self.autonomy.shell_env_passthrough.iter().enumerate() {
            if !is_valid_env_var_name(env_name) {
                anyhow::bail!(
                    "autonomy.shell_env_passthrough[{i}] is invalid ({env_name}); expected [A-Za-z_][A-Za-z0-9_]*"
                );
            }
        }
        let mut seen_non_cli_excluded = std::collections::HashSet::new();
        for (i, tool_name) in self.autonomy.non_cli_excluded_tools.iter().enumerate() {
            let normalized = tool_name.trim();
            if normalized.is_empty() {
                anyhow::bail!("autonomy.non_cli_excluded_tools[{i}] must not be empty");
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "autonomy.non_cli_excluded_tools[{i}] contains invalid characters: {normalized}"
                );
            }
            if !seen_non_cli_excluded.insert(normalized.to_string()) {
                anyhow::bail!(
                    "autonomy.non_cli_excluded_tools contains duplicate entry: {normalized}"
                );
            }
        }

        // Security OTP / estop
        if self.security.otp.token_ttl_secs == 0 {
            anyhow::bail!("security.otp.token_ttl_secs must be greater than 0");
        }
        if self.security.otp.cache_valid_secs == 0 {
            anyhow::bail!("security.otp.cache_valid_secs must be greater than 0");
        }
        if self.security.otp.cache_valid_secs < self.security.otp.token_ttl_secs {
            anyhow::bail!(
                "security.otp.cache_valid_secs must be greater than or equal to security.otp.token_ttl_secs"
            );
        }
        for (i, action) in self.security.otp.gated_actions.iter().enumerate() {
            let normalized = action.trim();
            if normalized.is_empty() {
                anyhow::bail!("security.otp.gated_actions[{i}] must not be empty");
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "security.otp.gated_actions[{i}] contains invalid characters: {normalized}"
                );
            }
        }
        DomainMatcher::new(
            &self.security.otp.gated_domains,
            &self.security.otp.gated_domain_categories,
        )
        .with_context(|| {
            "Invalid security.otp.gated_domains or security.otp.gated_domain_categories"
        })?;
        if self.security.estop.state_file.trim().is_empty() {
            anyhow::bail!("security.estop.state_file must not be empty");
        }
        if self.security.syscall_anomaly.max_denied_events_per_minute == 0 {
            anyhow::bail!(
                "security.syscall_anomaly.max_denied_events_per_minute must be greater than 0"
            );
        }
        if self.security.syscall_anomaly.max_total_events_per_minute == 0 {
            anyhow::bail!(
                "security.syscall_anomaly.max_total_events_per_minute must be greater than 0"
            );
        }
        if self.security.syscall_anomaly.max_denied_events_per_minute
            > self.security.syscall_anomaly.max_total_events_per_minute
        {
            anyhow::bail!(
                "security.syscall_anomaly.max_denied_events_per_minute must be less than or equal to security.syscall_anomaly.max_total_events_per_minute"
            );
        }
        if self.security.syscall_anomaly.max_alerts_per_minute == 0 {
            anyhow::bail!("security.syscall_anomaly.max_alerts_per_minute must be greater than 0");
        }
        if self.security.syscall_anomaly.alert_cooldown_secs == 0 {
            anyhow::bail!("security.syscall_anomaly.alert_cooldown_secs must be greater than 0");
        }
        if self.security.syscall_anomaly.log_path.trim().is_empty() {
            anyhow::bail!("security.syscall_anomaly.log_path must not be empty");
        }
        for (i, syscall_name) in self
            .security
            .syscall_anomaly
            .baseline_syscalls
            .iter()
            .enumerate()
        {
            let normalized = syscall_name.trim();
            if normalized.is_empty() {
                anyhow::bail!("security.syscall_anomaly.baseline_syscalls[{i}] must not be empty");
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '#')
            {
                anyhow::bail!(
                    "security.syscall_anomaly.baseline_syscalls[{i}] contains invalid characters: {normalized}"
                );
            }
        }
        if self.security.semantic_guard_collection.trim().is_empty() {
            anyhow::bail!("security.semantic_guard_collection must not be empty");
        }
        if !(0.0..=1.0).contains(&self.security.semantic_guard_threshold) {
            anyhow::bail!("security.semantic_guard_threshold must be between 0.0 and 1.0");
        }

        // Scheduler
        if self.scheduler.max_concurrent == 0 {
            anyhow::bail!("scheduler.max_concurrent must be greater than 0");
        }
        if self.scheduler.max_tasks == 0 {
            anyhow::bail!("scheduler.max_tasks must be greater than 0");
        }

        // Model routes
        for (i, route) in self.model_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                anyhow::bail!("model_routes[{i}].hint must not be empty");
            }
            if route.provider.trim().is_empty() {
                anyhow::bail!("model_routes[{i}].provider must not be empty");
            }
            if route.model.trim().is_empty() {
                anyhow::bail!("model_routes[{i}].model must not be empty");
            }
            if route.max_tokens == Some(0) {
                anyhow::bail!("model_routes[{i}].max_tokens must be greater than 0");
            }
        }

        if self.provider_api.is_some()
            && !self
                .default_provider
                .as_deref()
                .is_some_and(|provider| provider.starts_with("custom:"))
        {
            anyhow::bail!(
                "provider_api is only valid when default_provider uses the custom:<url> format"
            );
        }

        // Embedding routes
        for (i, route) in self.embedding_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                anyhow::bail!("embedding_routes[{i}].hint must not be empty");
            }
            if route.provider.trim().is_empty() {
                anyhow::bail!("embedding_routes[{i}].provider must not be empty");
            }
            if route.model.trim().is_empty() {
                anyhow::bail!("embedding_routes[{i}].model must not be empty");
            }
        }

        schema_provider_profiles::validate_model_provider_profiles(self)?;

        // Proxy (delegate to existing validation)
        self.proxy.validate()?;

        // Delegate coordination runtime safety bounds.
        if self.coordination.enabled && self.coordination.lead_agent.trim().is_empty() {
            anyhow::bail!("coordination.lead_agent must not be empty when coordination is enabled");
        }
        if self.coordination.max_inbox_messages_per_agent == 0 {
            anyhow::bail!("coordination.max_inbox_messages_per_agent must be greater than 0");
        }
        if self.coordination.max_dead_letters == 0 {
            anyhow::bail!("coordination.max_dead_letters must be greater than 0");
        }
        if self.coordination.max_context_entries == 0 {
            anyhow::bail!("coordination.max_context_entries must be greater than 0");
        }
        if self.coordination.max_seen_message_ids == 0 {
            anyhow::bail!("coordination.max_seen_message_ids must be greater than 0");
        }

        Ok(())
    }

    /// Apply environment variable overrides to config
    pub fn apply_env_overrides(&mut self) {
        // API Key: TOPCLAW_API_KEY
        if let Ok(key) = std::env::var("TOPCLAW_API_KEY") {
            if !key.is_empty() {
                self.api_key = Some(key);
            }
        }

        // API Key: ZAI_API_KEY overrides when provider is a Z.AI variant.
        if self.default_provider.as_deref().is_some_and(is_zai_alias) {
            if let Ok(key) = std::env::var("ZAI_API_KEY") {
                if !key.is_empty() {
                    self.api_key = Some(key);
                }
            }
        }

        // Provider override: TOPCLAW_PROVIDER
        if let Ok(provider) = std::env::var("TOPCLAW_PROVIDER") {
            if !provider.is_empty() {
                self.default_provider = Some(provider);
            }
        }

        // Model override: TOPCLAW_MODEL
        if let Ok(model) = std::env::var("TOPCLAW_MODEL") {
            if !model.is_empty() {
                self.default_model = Some(model);
            }
        }

        // Apply named provider profile remapping (Codex app-server compatibility).
        self.apply_named_model_provider_profile();

        // Workspace directory: TOPCLAW_WORKSPACE
        if let Ok(workspace) = std::env::var("TOPCLAW_WORKSPACE") {
            if !workspace.is_empty() {
                let (_, workspace_dir) =
                    resolve_config_dir_for_workspace(&PathBuf::from(workspace));
                self.workspace_dir = workspace_dir;
            }
        }

        // Open-skills opt-in flag: TOPCLAW_OPEN_SKILLS_ENABLED
        if let Ok(flag) = std::env::var("TOPCLAW_OPEN_SKILLS_ENABLED") {
            if !flag.trim().is_empty() {
                match flag.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => self.skills.open_skills_enabled = true,
                    "0" | "false" | "no" | "off" => self.skills.open_skills_enabled = false,
                    _ => tracing::warn!(
                        "Ignoring invalid TOPCLAW_OPEN_SKILLS_ENABLED (valid: 1|0|true|false|yes|no|on|off)"
                    ),
                }
            }
        }

        // Open-skills directory override: TOPCLAW_OPEN_SKILLS_DIR
        if let Ok(path) = std::env::var("TOPCLAW_OPEN_SKILLS_DIR") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.skills.open_skills_dir = Some(trimmed.to_string());
            }
        }

        // Skills prompt mode override: TOPCLAW_SKILLS_PROMPT_MODE
        if let Ok(mode) = std::env::var("TOPCLAW_SKILLS_PROMPT_MODE") {
            if !mode.trim().is_empty() {
                if let Some(parsed) = parse_skills_prompt_injection_mode(&mode) {
                    self.skills.prompt_injection_mode = parsed;
                } else {
                    tracing::warn!(
                        "Ignoring invalid TOPCLAW_SKILLS_PROMPT_MODE (valid: full|compact)"
                    );
                }
            }
        }

        // Gateway port: TOPCLAW_GATEWAY_PORT or PORT
        if let Ok(port_str) =
            std::env::var("TOPCLAW_GATEWAY_PORT").or_else(|_| std::env::var("PORT"))
        {
            if let Ok(port) = port_str.parse::<u16>() {
                self.gateway.port = port;
            }
        }

        // Gateway host: TOPCLAW_GATEWAY_HOST or HOST
        if let Ok(host) = std::env::var("TOPCLAW_GATEWAY_HOST").or_else(|_| std::env::var("HOST")) {
            if !host.is_empty() {
                self.gateway.host = host;
            }
        }

        // Allow public bind: TOPCLAW_ALLOW_PUBLIC_BIND
        if let Ok(val) = std::env::var("TOPCLAW_ALLOW_PUBLIC_BIND") {
            self.gateway.allow_public_bind = val == "1" || val.eq_ignore_ascii_case("true");
        }

        // Temperature: TOPCLAW_TEMPERATURE
        if let Ok(temp_str) = std::env::var("TOPCLAW_TEMPERATURE") {
            if let Ok(temp) = temp_str.parse::<f64>() {
                if (0.0..=2.0).contains(&temp) {
                    self.default_temperature = temp;
                }
            }
        }

        // Reasoning override: TOPCLAW_REASONING_ENABLED
        if let Ok(flag) = std::env::var("TOPCLAW_REASONING_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        // Vision support override: TOPCLAW_MODEL_SUPPORT_VISION
        if let Ok(flag) = std::env::var("TOPCLAW_MODEL_SUPPORT_VISION") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.model_support_vision = Some(true),
                "0" | "false" | "no" | "off" => self.model_support_vision = Some(false),
                _ => {}
            }
        }

        // Web search enabled: TOPCLAW_WEB_SEARCH_ENABLED
        if let Ok(enabled) = std::env::var("TOPCLAW_WEB_SEARCH_ENABLED") {
            self.web_search.enabled = enabled == "1" || enabled.eq_ignore_ascii_case("true");
        }

        // Web search provider: TOPCLAW_WEB_SEARCH_PROVIDER
        if let Ok(provider) = std::env::var("TOPCLAW_WEB_SEARCH_PROVIDER") {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.web_search.provider = provider.to_string();
            }
        }

        // Brave API key: TOPCLAW_BRAVE_API_KEY
        if let Ok(api_key) = std::env::var("TOPCLAW_BRAVE_API_KEY") {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.brave_api_key = Some(api_key.to_string());
            }
        }

        // Web search max results: TOPCLAW_WEB_SEARCH_MAX_RESULTS
        if let Ok(max_results) = std::env::var("TOPCLAW_WEB_SEARCH_MAX_RESULTS") {
            if let Ok(max_results) = max_results.parse::<usize>() {
                if (1..=10).contains(&max_results) {
                    self.web_search.max_results = max_results;
                }
            }
        }

        // Web search timeout: TOPCLAW_WEB_SEARCH_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("TOPCLAW_WEB_SEARCH_TIMEOUT_SECS") {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.web_search.timeout_secs = timeout_secs;
                }
            }
        }

        // Storage provider key (optional backend override): TOPCLAW_STORAGE_PROVIDER
        if let Ok(provider) = std::env::var("TOPCLAW_STORAGE_PROVIDER") {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.storage.provider.config.provider = provider.to_string();
            }
        }

        // Storage connection URL (for remote backends): TOPCLAW_STORAGE_DB_URL
        if let Ok(db_url) = std::env::var("TOPCLAW_STORAGE_DB_URL") {
            let db_url = db_url.trim();
            if !db_url.is_empty() {
                self.storage.provider.config.db_url = Some(db_url.to_string());
            }
        }

        // Storage connect timeout: TOPCLAW_STORAGE_CONNECT_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("TOPCLAW_STORAGE_CONNECT_TIMEOUT_SECS") {
            if let Ok(timeout_secs) = timeout_secs.parse::<u64>() {
                if timeout_secs > 0 {
                    self.storage.provider.config.connect_timeout_secs = Some(timeout_secs);
                }
            }
        }
        // Proxy enabled flag: TOPCLAW_PROXY_ENABLED
        let explicit_proxy_enabled = std::env::var("TOPCLAW_PROXY_ENABLED")
            .ok()
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        // Proxy URLs: TOPCLAW_* wins, then generic *PROXY vars.
        let mut proxy_url_overridden = false;
        if let Ok(proxy_url) =
            std::env::var("TOPCLAW_HTTP_PROXY").or_else(|_| std::env::var("HTTP_PROXY"))
        {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("TOPCLAW_HTTPS_PROXY").or_else(|_| std::env::var("HTTPS_PROXY"))
        {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("TOPCLAW_ALL_PROXY").or_else(|_| std::env::var("ALL_PROXY"))
        {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(no_proxy) =
            std::env::var("TOPCLAW_NO_PROXY").or_else(|_| std::env::var("NO_PROXY"))
        {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        // Proxy scope and service selectors.
        if let Ok(scope_raw) = std::env::var("TOPCLAW_PROXY_SCOPE") {
            if let Some(scope) = parse_proxy_scope(&scope_raw) {
                self.proxy.scope = scope;
            } else {
                tracing::warn!(
                    scope = %scope_raw,
                    "Ignoring invalid TOPCLAW_PROXY_SCOPE (valid: environment|topclaw|services)"
                );
            }
        }

        if let Ok(services_raw) = std::env::var("TOPCLAW_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());
    }

    pub async fn save(&self) -> Result<()> {
        // Encrypt secrets before serialization
        let mut config_to_save = self.clone();
        let topclaw_dir = self
            .config_path
            .parent()
            .context("Config path must have a parent directory")?;
        schema_secrets::encrypt_config_secrets(topclaw_dir, &mut config_to_save)?;

        let toml_str =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        let parent_dir = self
            .config_path
            .parent()
            .context("Config path must have a parent directory")?;

        fs::create_dir_all(parent_dir).await.with_context(|| {
            format!(
                "Failed to create config directory: {}",
                parent_dir.display()
            )
        })?;

        let file_name = self
            .config_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("config.toml");
        let temp_path = parent_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
        let backup_path = parent_dir.join(format!("{file_name}.bak"));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to create temporary config file: {}",
                    temp_path.display()
                )
            })?;
        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            fs::set_permissions(&temp_path, Permissions::from_mode(0o600))
                .await
                .with_context(|| {
                    format!(
                        "Failed to set secure permissions on temporary config file: {}",
                        temp_path.display()
                    )
                })?;
        }
        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to fsync temporary config file")?;
        drop(temp_file);

        let had_existing_config = self.config_path.exists();
        if had_existing_config {
            fs::copy(&self.config_path, &backup_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create config backup before atomic replace: {}",
                        backup_path.display()
                    )
                })?;
        }

        if let Err(e) = fs::rename(&temp_path, &self.config_path).await {
            let _ = fs::remove_file(&temp_path).await;
            if had_existing_config && backup_path.exists() {
                fs::copy(&backup_path, &self.config_path)
                    .await
                    .context("Failed to restore config backup")?;
            }
            anyhow::bail!("Failed to atomically replace config file: {e}");
        }

        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            fs::set_permissions(&self.config_path, Permissions::from_mode(0o600))
                .await
                .with_context(|| {
                    format!(
                        "Failed to enforce secure permissions on config file: {}",
                        self.config_path.display()
                    )
                })?;
        }

        sync_directory(parent_dir).await?;

        if had_existing_config {
            let _ = fs::remove_file(&backup_path).await;
        }

        Ok(())
    }
}

async fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let dir = File::open(path)
            .await
            .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
        dir.sync_all()
            .await
            .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
