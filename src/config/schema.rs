use crate::config::autonomy::is_valid_env_var_name;
use crate::config::parse_skills_prompt_injection_mode;
use crate::config::proxy::{
    normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list, parse_proxy_scope,
};
use crate::config::set_runtime_proxy_config;
use crate::config::traits::ChannelConfig;
use crate::config::AgentConfig;
use crate::config::AgentsIpcConfig;
use crate::config::AuditConfig;
use crate::config::AutonomyConfig;
use crate::config::BridgeConfig;
use crate::config::BrowserConfig;
use crate::config::ComposioConfig;
use crate::config::CoordinationConfig;
use crate::config::CostConfig;
use crate::config::CronConfig;
use crate::config::DelegateAgentConfig;
use crate::config::EmbeddingRouteConfig;
use crate::config::EstopConfig;
use crate::config::GatewayConfig;
use crate::config::GoalLoopConfig;
use crate::config::HardwareConfig;
use crate::config::HeartbeatConfig;
use crate::config::HooksConfig;
use crate::config::HttpRequestConfig;
use crate::config::IdentityConfig;
use crate::config::ModelProviderConfig;
use crate::config::ModelRouteConfig;
use crate::config::MultimodalConfig;
use crate::config::ObservabilityConfig;
use crate::config::OtpConfig;
use crate::config::PeripheralsConfig;
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
use crate::config::SelfImprovementConfig;
use crate::config::SkillsConfig;
use crate::config::StorageProviderSection;
use crate::config::TranscriptionConfig;
use crate::config::TunnelConfig;
use crate::config::WebFetchConfig;
use crate::config::WebSearchConfig;
use crate::config::WorkspacesConfig;
use crate::providers::{is_glm_alias, is_zai_alias};
use crate::security::DomainMatcher;
use anyhow::{Context, Result};
use directories::UserDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use tokio::fs::File;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

const fn default_true() -> bool {
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
    /// Default provider ID or alias (e.g. `"openrouter"`, `"ollama"`, `"anthropic"`). Default: `"openrouter"`.
    pub default_provider: Option<String>,
    /// Optional API protocol mode for `custom:` providers.
    #[serde(default)]
    pub provider_api: Option<ProviderApiMode>,
    /// Default model routed through the selected provider (e.g. `"anthropic/claude-sonnet-4-6"`).
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

    /// Self-improvement automation configuration (`[self_improvement]`).
    #[serde(default)]
    pub self_improvement: SelfImprovementConfig,

    /// Goal loop configuration for autonomous long-term goal execution (`[goal_loop]`).
    #[serde(default)]
    pub goal_loop: GoalLoopConfig,

    /// Channel configurations: Telegram, Discord, Slack, etc. (`[channels_config]`).
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

    /// Composio managed OAuth tools integration (`[composio]`).
    #[serde(default)]
    pub composio: ComposioConfig,

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

    /// Identity format configuration: OpenClaw or AIEOS (`[identity]`).
    #[serde(default)]
    pub identity: IdentityConfig,

    /// Cost tracking and budget enforcement configuration (`[cost]`).
    #[serde(default)]
    pub cost: CostConfig,

    /// Peripheral board configuration for hardware integration (`[peripherals]`).
    #[serde(default)]
    pub peripherals: PeripheralsConfig,

    /// Delegate agent configurations for multi-agent workflows.
    #[serde(default)]
    pub agents: HashMap<String, DelegateAgentConfig>,

    /// Delegate coordination runtime configuration (`[coordination]`).
    #[serde(default)]
    pub coordination: CoordinationConfig,

    /// Hooks configuration (lifecycle hooks and built-in hook toggles).
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Hardware configuration (wizard-driven physical world setup).
    #[serde(default)]
    pub hardware: HardwareConfig,

    /// Voice transcription configuration (Whisper API via Groq).
    #[serde(default)]
    pub transcription: TranscriptionConfig,

    /// Inter-process agent communication (`[agents_ipc]`).
    #[serde(default)]
    pub agents_ipc: AgentsIpcConfig,

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
            self.channels_config.bridge.is_some(),
            self.channels_config.telegram.is_some(),
            self.channels_config.discord.is_some(),
            self.channels_config.slack.is_some(),
            self.channels_config.mattermost.is_some(),
            self.channels_config.webhook.is_some(),
            self.channels_config.imessage.is_some(),
            self.channels_config.matrix.is_some(),
            self.channels_config.signal.is_some(),
            self.channels_config.whatsapp.is_some(),
            self.channels_config.linq.is_some(),
            self.channels_config.wati.is_some(),
            self.channels_config.nextcloud_talk.is_some(),
            self.channels_config.email.is_some(),
            self.channels_config.irc.is_some(),
            self.channels_config.lark.is_some(),
            self.channels_config.feishu.is_some(),
            self.channels_config.dingtalk.is_some(),
            self.channels_config.qq.is_some(),
            self.channels_config.nostr.is_some(),
            self.channels_config.clawdtalk.is_some(),
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

// ── Identity (AIEOS / OpenClaw format) ──────────────────────────

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

/// Persistent storage configuration (`[storage]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageConfig {
    /// Storage provider settings (e.g. sqlite, postgres, mariadb).
    #[serde(default)]
    pub provider: StorageProviderSection,
}

/// Memory backend configuration (`[memory]` section).
///
/// Controls conversation memory storage, embeddings, hybrid search, response caching,
/// and memory snapshot/hydration.
/// Configuration for Qdrant vector database backend (`[memory.qdrant]`).
/// Used when `[memory].backend = "qdrant"`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QdrantConfig {
    /// Qdrant server URL (e.g. "http://localhost:6333").
    /// Falls back to `QDRANT_URL` env var if not set.
    #[serde(default)]
    pub url: Option<String>,
    /// Qdrant collection name for storing memories.
    /// Falls back to `QDRANT_COLLECTION` env var, or default "topclaw_memories".
    #[serde(default = "default_qdrant_collection")]
    pub collection: String,
    /// Optional API key for Qdrant Cloud or secured instances.
    /// Falls back to `QDRANT_API_KEY` env var if not set.
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_qdrant_collection() -> String {
    "topclaw_memories".into()
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: None,
            collection: default_qdrant_collection(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct MemoryConfig {
    /// "sqlite" | "lucid" | "postgres" | "mariadb" | "qdrant" | "markdown" | "none" (`none` = explicit no-op memory)
    ///
    /// `postgres` / `mariadb` require `[storage.provider.config]` with `db_url`.
    /// `qdrant` uses `[memory.qdrant]` config or `QDRANT_URL` env var.
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

    // ── Qdrant backend options ─────────────────────────────────
    /// Configuration for Qdrant vector database backend.
    /// Only used when `backend = "qdrant"`.
    #[serde(default)]
    pub qdrant: QdrantConfig,
}

fn default_embedding_provider() -> String {
    "none".into()
}
fn default_hygiene_enabled() -> bool {
    true
}
fn default_archive_after_days() -> u32 {
    7
}
fn default_purge_after_days() -> u32 {
    30
}
fn default_conversation_retention_days() -> u32 {
    30
}
fn default_embedding_model() -> String {
    "text-embedding-3-small".into()
}
fn default_embedding_dims() -> usize {
    1536
}
fn default_vector_weight() -> f64 {
    0.7
}
fn default_keyword_weight() -> f64 {
    0.3
}
fn default_min_relevance_score() -> f64 {
    0.4
}
fn default_cache_size() -> usize {
    10_000
}
fn default_chunk_size() -> usize {
    512
}
fn default_response_cache_ttl() -> u32 {
    60
}
fn default_response_cache_max() -> usize {
    5_000
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "sqlite".into(),
            auto_save: true,
            hygiene_enabled: default_hygiene_enabled(),
            archive_after_days: default_archive_after_days(),
            purge_after_days: default_purge_after_days(),
            conversation_retention_days: default_conversation_retention_days(),
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            vector_weight: default_vector_weight(),
            keyword_weight: default_keyword_weight(),
            min_relevance_score: default_min_relevance_score(),
            embedding_cache_size: default_cache_size(),
            chunk_max_tokens: default_chunk_size(),
            response_cache_enabled: false,
            response_cache_ttl_minutes: default_response_cache_ttl(),
            response_cache_max_entries: default_response_cache_max(),
            snapshot_enabled: false,
            snapshot_on_hygiene: false,
            auto_hydrate: true,
            sqlite_open_timeout_secs: None,
            qdrant: QdrantConfig::default(),
        }
    }
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

struct ConfigWrapper<T: ChannelConfig>(std::marker::PhantomData<T>);

impl<T: ChannelConfig> ConfigWrapper<T> {
    fn new(_: Option<&T>) -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ChannelConfig> crate::config::traits::ConfigHandle for ConfigWrapper<T> {
    fn name(&self) -> &'static str {
        T::name()
    }
    fn desc(&self) -> &'static str {
        T::desc()
    }
}

/// Top-level channel configurations (`[channels_config]` section).
///
/// Each channel sub-section (e.g. `telegram`, `discord`) is optional;
/// setting it to `Some(...)` enables that channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChannelsConfig {
    /// Enable the CLI interactive channel. Default: `true`.
    pub cli: bool,
    /// Local bridge websocket channel configuration.
    pub bridge: Option<BridgeConfig>,
    /// Telegram bot channel configuration.
    pub telegram: Option<TelegramConfig>,
    /// Discord bot channel configuration.
    pub discord: Option<DiscordConfig>,
    /// Slack bot channel configuration.
    pub slack: Option<SlackConfig>,
    /// Mattermost bot channel configuration.
    pub mattermost: Option<MattermostConfig>,
    /// Webhook channel configuration.
    pub webhook: Option<WebhookConfig>,
    /// iMessage channel configuration (macOS only).
    pub imessage: Option<IMessageConfig>,
    /// Matrix channel configuration.
    pub matrix: Option<MatrixConfig>,
    /// Signal channel configuration.
    pub signal: Option<SignalConfig>,
    /// WhatsApp channel configuration (Cloud API or Web mode).
    pub whatsapp: Option<WhatsAppConfig>,
    /// Linq Partner API channel configuration.
    pub linq: Option<LinqConfig>,
    /// WATI WhatsApp Business API channel configuration.
    pub wati: Option<WatiConfig>,
    /// Nextcloud Talk bot channel configuration.
    pub nextcloud_talk: Option<NextcloudTalkConfig>,
    /// Email channel configuration.
    pub email: Option<crate::channels::email_channel::EmailConfig>,
    /// IRC channel configuration.
    pub irc: Option<IrcConfig>,
    /// Lark channel configuration.
    pub lark: Option<LarkConfig>,
    /// Feishu channel configuration.
    pub feishu: Option<FeishuConfig>,
    /// DingTalk channel configuration.
    pub dingtalk: Option<DingTalkConfig>,
    /// QQ Official Bot channel configuration.
    pub qq: Option<QQConfig>,
    pub nostr: Option<NostrConfig>,
    /// ClawdTalk voice channel configuration.
    pub clawdtalk: Option<crate::channels::clawdtalk::ClawdTalkConfig>,
    /// Base timeout in seconds for processing a single channel message (LLM + tools).
    /// Runtime uses this as a per-turn budget that scales with tool-loop depth
    /// (up to 4x, capped) so one slow/retried model call does not consume the
    /// entire conversation budget.
    /// Default: 300s for on-device LLMs (Ollama) which are slower than cloud APIs.
    #[serde(default = "default_channel_message_timeout_secs")]
    pub message_timeout_secs: u64,
}

impl ChannelsConfig {
    /// get channels' metadata and `.is_some()`, except webhook
    #[rustfmt::skip]
    pub fn channels_except_webhook(&self) -> Vec<(Box<dyn super::traits::ConfigHandle>, bool)> {
        vec![
            (
                Box::new(ConfigWrapper::new(self.bridge.as_ref())),
                self.bridge.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.telegram.as_ref())),
                self.telegram.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.discord.as_ref())),
                self.discord.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.slack.as_ref())),
                self.slack.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.mattermost.as_ref())),
                self.mattermost.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.imessage.as_ref())),
                self.imessage.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.matrix.as_ref())),
                self.matrix.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.signal.as_ref())),
                self.signal.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.whatsapp.as_ref())),
                self.whatsapp.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.linq.as_ref())),
                self.linq.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.wati.as_ref())),
                self.wati.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.nextcloud_talk.as_ref())),
                self.nextcloud_talk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.email.as_ref())),
                self.email.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.irc.as_ref())),
                self.irc.is_some()
            ),
            (
                Box::new(ConfigWrapper::new(self.lark.as_ref())),
                self.lark.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.feishu.as_ref())),
                self.feishu.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.dingtalk.as_ref())),
                self.dingtalk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.qq.as_ref())),
                self.qq
                    .as_ref()
                    .is_some_and(|qq| qq.receive_mode == QQReceiveMode::Websocket)
            ),
            (
                Box::new(ConfigWrapper::new(self.nostr.as_ref())),
                self.nostr.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.clawdtalk.as_ref())),
                self.clawdtalk.is_some(),
            ),
        ]
    }

    pub fn channels(&self) -> Vec<(Box<dyn super::traits::ConfigHandle>, bool)> {
        let mut ret = self.channels_except_webhook();
        ret.push((
            Box::new(ConfigWrapper::new(self.webhook.as_ref())),
            self.webhook.is_some(),
        ));
        ret
    }
}

fn default_channel_message_timeout_secs() -> u64 {
    300
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            bridge: None,
            telegram: None,
            discord: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            irc: None,
            lark: None,
            feishu: None,
            dingtalk: None,
            qq: None,
            nostr: None,
            clawdtalk: None,
            message_timeout_secs: default_channel_message_timeout_secs(),
        }
    }
}

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

fn default_draft_update_interval_ms() -> u64 {
    500
}

fn default_telegram_stream_mode() -> StreamMode {
    StreamMode::Partial
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

fn resolve_group_reply_mode(
    group_reply: Option<&GroupReplyConfig>,
    default_mode: GroupReplyMode,
) -> GroupReplyMode {
    if let Some(mode) = group_reply.and_then(|cfg| cfg.mode) {
        return mode;
    }
    default_mode
}

fn clone_group_reply_allowed_sender_ids(group_reply: Option<&GroupReplyConfig>) -> Vec<String> {
    group_reply
        .map(|cfg| cfg.allowed_sender_ids.clone())
        .unwrap_or_default()
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

/// Slack bot channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SlackConfig {
    /// Slack bot OAuth token (xoxb-...).
    pub bot_token: String,
    /// Slack app-level token for Socket Mode (xapp-...).
    pub app_token: Option<String>,
    /// Optional channel ID to restrict the bot to a single channel.
    /// Omit (or set `"*"`) to listen across all accessible channels.
    pub channel_id: Option<String>,
    /// Allowed Slack user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
}

impl ChannelConfig for SlackConfig {
    fn name() -> &'static str {
        "Slack"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

impl SlackConfig {
    #[must_use]
    pub fn effective_group_reply_mode(&self) -> GroupReplyMode {
        resolve_group_reply_mode(self.group_reply.as_ref(), GroupReplyMode::AllMessages)
    }

    #[must_use]
    pub fn group_reply_allowed_sender_ids(&self) -> Vec<String> {
        clone_group_reply_allowed_sender_ids(self.group_reply.as_ref())
    }
}

/// Mattermost bot channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MattermostConfig {
    /// Mattermost server URL (e.g. `"https://mattermost.example.com"`).
    pub url: String,
    /// Mattermost bot access token.
    pub bot_token: String,
    /// Optional channel ID to restrict the bot to a single channel.
    pub channel_id: Option<String>,
    /// Allowed Mattermost user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true (default), replies thread on the original post.
    /// When false, replies go to the channel root.
    #[serde(default)]
    pub thread_replies: Option<bool>,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
}

impl ChannelConfig for MattermostConfig {
    fn name() -> &'static str {
        "Mattermost"
    }
    fn desc() -> &'static str {
        "connect to your bot"
    }
}

impl MattermostConfig {
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

/// iMessage channel configuration (macOS only).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IMessageConfig {
    /// Allowed iMessage contacts (phone numbers or email addresses). Empty = deny all.
    pub allowed_contacts: Vec<String>,
}

impl ChannelConfig for IMessageConfig {
    fn name() -> &'static str {
        "iMessage"
    }
    fn desc() -> &'static str {
        "macOS only"
    }
}

/// Matrix channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MatrixConfig {
    /// Matrix homeserver URL (e.g. `"https://matrix.org"`).
    pub homeserver: String,
    /// Matrix access token for the bot account.
    pub access_token: String,
    /// Optional Matrix user ID (e.g. `"@bot:matrix.org"`).
    #[serde(default)]
    pub user_id: Option<String>,
    /// Optional Matrix device ID.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Matrix room ID to listen in (e.g. `"!abc123:matrix.org"`).
    pub room_id: String,
    /// Allowed Matrix user IDs. Empty = deny all.
    pub allowed_users: Vec<String>,
    /// When true, only respond to direct rooms, explicit @-mentions, or replies to bot messages.
    #[serde(default)]
    pub mention_only: bool,
}

impl ChannelConfig for MatrixConfig {
    fn name() -> &'static str {
        "Matrix"
    }
    fn desc() -> &'static str {
        "self-hosted chat"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SignalConfig {
    /// Base URL for the signal-cli HTTP daemon (e.g. "http://127.0.0.1:8686").
    pub http_url: String,
    /// E.164 phone number of the signal-cli account (e.g. "+1234567890").
    pub account: String,
    /// Optional group ID to filter messages.
    /// - `None` or omitted: accept all messages (DMs and groups)
    /// - `"dm"`: only accept direct messages
    /// - Specific group ID: only accept messages from that group
    #[serde(default)]
    pub group_id: Option<String>,
    /// Allowed sender phone numbers (E.164) or "*" for all.
    #[serde(default)]
    pub allowed_from: Vec<String>,
    /// Skip messages that are attachment-only (no text body).
    #[serde(default)]
    pub ignore_attachments: bool,
    /// Skip incoming story messages.
    #[serde(default)]
    pub ignore_stories: bool,
}

impl ChannelConfig for SignalConfig {
    fn name() -> &'static str {
        "Signal"
    }
    fn desc() -> &'static str {
        "An open-source, encrypted messaging service"
    }
}

/// WhatsApp channel configuration (Cloud API or Web mode).
///
/// Set `phone_number_id` for Cloud API mode, or `session_path` for Web mode.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WhatsAppConfig {
    /// Access token from Meta Business Suite (Cloud API mode)
    #[serde(default)]
    pub access_token: Option<String>,
    /// Phone number ID from Meta Business API (Cloud API mode)
    #[serde(default)]
    pub phone_number_id: Option<String>,
    /// Webhook verify token (you define this, Meta sends it back for verification)
    /// Only used in Cloud API mode
    #[serde(default)]
    pub verify_token: Option<String>,
    /// App secret from Meta Business Suite (for webhook signature verification)
    /// Can also be set via `TOPCLAW_WHATSAPP_APP_SECRET` environment variable
    /// Only used in Cloud API mode
    #[serde(default)]
    pub app_secret: Option<String>,
    /// Session database path for WhatsApp Web client (Web mode)
    /// When set, enables native WhatsApp Web mode with wa-rs
    #[serde(default)]
    pub session_path: Option<String>,
    /// Phone number for pair code linking (Web mode, optional)
    /// Format: country code + number (e.g., "15551234567")
    /// If not set, QR code pairing will be used
    #[serde(default)]
    pub pair_phone: Option<String>,
    /// Custom pair code for linking (Web mode, optional)
    /// Leave empty to let WhatsApp generate one
    #[serde(default)]
    pub pair_code: Option<String>,
    /// Allowed phone numbers (E.164 format: +1234567890) or "*" for all
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
}

impl ChannelConfig for WhatsAppConfig {
    fn name() -> &'static str {
        "WhatsApp"
    }
    fn desc() -> &'static str {
        "Business Cloud API"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LinqConfig {
    /// Linq Partner API token (Bearer auth)
    pub api_token: String,
    /// Phone number to send from (E.164 format)
    pub from_phone: String,
    /// Webhook signing secret for signature verification
    #[serde(default)]
    pub signing_secret: Option<String>,
    /// Allowed sender handles (phone numbers) or "*" for all
    #[serde(default)]
    pub allowed_senders: Vec<String>,
}

impl ChannelConfig for LinqConfig {
    fn name() -> &'static str {
        "Linq"
    }
    fn desc() -> &'static str {
        "iMessage/RCS/SMS via Linq API"
    }
}

/// WATI WhatsApp Business API channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WatiConfig {
    /// WATI API token (Bearer auth).
    pub api_token: String,
    /// WATI API base URL (default: https://live-mt-server.wati.io).
    #[serde(default = "default_wati_api_url")]
    pub api_url: String,
    /// Tenant ID for multi-channel setups (optional).
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Allowed phone numbers (E.164 format) or "*" for all.
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
}

fn default_wati_api_url() -> String {
    "https://live-mt-server.wati.io".to_string()
}

impl ChannelConfig for WatiConfig {
    fn name() -> &'static str {
        "WATI"
    }
    fn desc() -> &'static str {
        "WhatsApp via WATI Business API"
    }
}

/// Nextcloud Talk bot configuration (webhook receive + OCS send API).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NextcloudTalkConfig {
    /// Nextcloud base URL (e.g. "https://cloud.example.com").
    pub base_url: String,
    /// Bot app token used for OCS API bearer auth.
    pub app_token: String,
    /// Shared secret for webhook signature verification.
    ///
    /// Can also be set via `TOPCLAW_NEXTCLOUD_TALK_WEBHOOK_SECRET`.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Allowed Nextcloud actor IDs (`[]` = deny all, `"*"` = allow all).
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

impl ChannelConfig for NextcloudTalkConfig {
    fn name() -> &'static str {
        "NextCloud Talk"
    }
    fn desc() -> &'static str {
        "NextCloud Talk platform"
    }
}

impl WhatsAppConfig {
    /// Detect which backend to use based on config fields.
    /// Returns "cloud" if phone_number_id is set, "web" if session_path is set.
    pub fn backend_type(&self) -> &'static str {
        if self.phone_number_id.is_some() {
            "cloud"
        } else if self.session_path.is_some() {
            "web"
        } else {
            // Default to Cloud API for backward compatibility
            "cloud"
        }
    }

    /// Check if this is a valid Cloud API config
    pub fn is_cloud_config(&self) -> bool {
        self.phone_number_id.is_some() && self.access_token.is_some() && self.verify_token.is_some()
    }

    /// Check if this is a valid Web config
    pub fn is_web_config(&self) -> bool {
        self.session_path.is_some()
    }

    /// Returns true when both Cloud and Web selectors are present.
    ///
    /// Runtime currently prefers Cloud mode in this case for backward compatibility.
    pub fn is_ambiguous_config(&self) -> bool {
        self.phone_number_id.is_some() && self.session_path.is_some()
    }
}

/// IRC channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrcConfig {
    /// IRC server hostname
    pub server: String,
    /// IRC server port (default: 6697 for TLS)
    #[serde(default = "default_irc_port")]
    pub port: u16,
    /// Bot nickname
    pub nickname: String,
    /// Username (defaults to nickname if not set)
    pub username: Option<String>,
    /// Channels to join on connect
    #[serde(default)]
    pub channels: Vec<String>,
    /// Allowed nicknames (case-insensitive) or "*" for all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Server password (for bouncers like ZNC)
    pub server_password: Option<String>,
    /// NickServ IDENTIFY password
    pub nickserv_password: Option<String>,
    /// SASL PLAIN password (IRCv3)
    pub sasl_password: Option<String>,
    /// Verify TLS certificate (default: true)
    pub verify_tls: Option<bool>,
}

impl ChannelConfig for IrcConfig {
    fn name() -> &'static str {
        "IRC"
    }
    fn desc() -> &'static str {
        "IRC over TLS"
    }
}

fn default_irc_port() -> u16 {
    6697
}

/// How TopClaw receives events from Feishu / Lark.
///
/// - `websocket` (default) — persistent WSS long-connection; no public URL required.
/// - `webhook`             — HTTP callback server; requires a public HTTPS endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LarkReceiveMode {
    #[default]
    Websocket,
    Webhook,
}

pub fn default_lark_draft_update_interval_ms() -> u64 {
    3000
}

pub fn default_lark_max_draft_edits() -> u32 {
    20
}

/// Lark configuration for messaging integration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LarkConfig {
    /// App ID from the Lark developer console.
    pub app_id: String,
    /// App Secret from the Lark developer console.
    pub app_secret: String,
    /// Encrypt key for webhook message decryption (optional)
    #[serde(default)]
    pub encrypt_key: Option<String>,
    /// Verification token for webhook validation (optional)
    #[serde(default)]
    pub verification_token: Option<String>,
    /// Allowed user IDs or union IDs (empty = deny all, "*" = allow all)
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
    /// Event receive mode: "websocket" (default) or "webhook"
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,
    /// HTTP port for webhook mode only. Must be set when receive_mode = "webhook".
    /// Not required (and ignored) for websocket mode.
    #[serde(default)]
    pub port: Option<u16>,
    /// Minimum interval (ms) between draft message edits. Default: 3000.
    #[serde(default = "default_lark_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// Maximum number of edits per draft message before stopping updates.
    #[serde(default = "default_lark_max_draft_edits")]
    pub max_draft_edits: u32,
}

impl ChannelConfig for LarkConfig {
    fn name() -> &'static str {
        "Lark"
    }
    fn desc() -> &'static str {
        "Lark Bot"
    }
}

impl LarkConfig {
    #[must_use]
    pub fn effective_group_reply_mode(&self) -> GroupReplyMode {
        resolve_group_reply_mode(self.group_reply.as_ref(), GroupReplyMode::AllMessages)
    }

    #[must_use]
    pub fn group_reply_allowed_sender_ids(&self) -> Vec<String> {
        clone_group_reply_allowed_sender_ids(self.group_reply.as_ref())
    }
}

/// Feishu configuration for messaging integration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeishuConfig {
    /// App ID from Feishu developer console
    pub app_id: String,
    /// App Secret from Feishu developer console
    pub app_secret: String,
    /// Encrypt key for webhook message decryption (optional)
    #[serde(default)]
    pub encrypt_key: Option<String>,
    /// Verification token for webhook validation (optional)
    #[serde(default)]
    pub verification_token: Option<String>,
    /// Allowed user IDs or union IDs (empty = deny all, "*" = allow all)
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Group-chat trigger controls.
    #[serde(default)]
    pub group_reply: Option<GroupReplyConfig>,
    /// Event receive mode: "websocket" (default) or "webhook"
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,
    /// HTTP port for webhook mode only. Must be set when receive_mode = "webhook".
    /// Not required (and ignored) for websocket mode.
    #[serde(default)]
    pub port: Option<u16>,
    /// Minimum interval between streaming draft edits (milliseconds).
    #[serde(default = "default_lark_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// Maximum number of draft edits per message before finalizing.
    #[serde(default = "default_lark_max_draft_edits")]
    pub max_draft_edits: u32,
}

impl ChannelConfig for FeishuConfig {
    fn name() -> &'static str {
        "Feishu"
    }
    fn desc() -> &'static str {
        "Feishu Bot"
    }
}

impl FeishuConfig {
    #[must_use]
    pub fn effective_group_reply_mode(&self) -> GroupReplyMode {
        resolve_group_reply_mode(self.group_reply.as_ref(), GroupReplyMode::AllMessages)
    }

    #[must_use]
    pub fn group_reply_allowed_sender_ids(&self) -> Vec<String> {
        clone_group_reply_allowed_sender_ids(self.group_reply.as_ref())
    }
}

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

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            sandbox: SandboxConfig::default(),
            resources: ResourceLimitsConfig::default(),
            audit: AuditConfig::default(),
            otp: OtpConfig::default(),
            estop: EstopConfig::default(),
            syscall_anomaly: SyscallAnomalyConfig::default(),
            canary_tokens: default_true(),
            semantic_guard: false,
            semantic_guard_collection: default_semantic_guard_collection(),
            semantic_guard_threshold: default_semantic_guard_threshold(),
        }
    }
}

fn default_semantic_guard_collection() -> String {
    "semantic_guard".to_string()
}

fn default_semantic_guard_threshold() -> f64 {
    0.82
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

fn default_syscall_anomaly_max_denied_events_per_minute() -> u32 {
    5
}

fn default_syscall_anomaly_max_total_events_per_minute() -> u32 {
    120
}

fn default_syscall_anomaly_max_alerts_per_minute() -> u32 {
    30
}

fn default_syscall_anomaly_alert_cooldown_secs() -> u64 {
    20
}

fn default_syscall_anomaly_log_path() -> String {
    "syscall-anomalies.log".to_string()
}

fn default_syscall_anomaly_baseline_syscalls() -> Vec<String> {
    vec![
        "read".to_string(),
        "write".to_string(),
        "open".to_string(),
        "openat".to_string(),
        "close".to_string(),
        "stat".to_string(),
        "fstat".to_string(),
        "newfstatat".to_string(),
        "lseek".to_string(),
        "mmap".to_string(),
        "mprotect".to_string(),
        "munmap".to_string(),
        "brk".to_string(),
        "rt_sigaction".to_string(),
        "rt_sigprocmask".to_string(),
        "ioctl".to_string(),
        "fcntl".to_string(),
        "access".to_string(),
        "pipe2".to_string(),
        "dup".to_string(),
        "dup2".to_string(),
        "dup3".to_string(),
        "epoll_create1".to_string(),
        "epoll_ctl".to_string(),
        "epoll_wait".to_string(),
        "poll".to_string(),
        "ppoll".to_string(),
        "select".to_string(),
        "futex".to_string(),
        "clock_gettime".to_string(),
        "nanosleep".to_string(),
        "getpid".to_string(),
        "gettid".to_string(),
        "set_tid_address".to_string(),
        "set_robust_list".to_string(),
        "clone".to_string(),
        "clone3".to_string(),
        "fork".to_string(),
        "execve".to_string(),
        "wait4".to_string(),
        "exit".to_string(),
        "exit_group".to_string(),
        "socket".to_string(),
        "connect".to_string(),
        "accept".to_string(),
        "accept4".to_string(),
        "listen".to_string(),
        "sendto".to_string(),
        "recvfrom".to_string(),
        "sendmsg".to_string(),
        "recvmsg".to_string(),
        "getsockname".to_string(),
        "getpeername".to_string(),
        "setsockopt".to_string(),
        "getsockopt".to_string(),
        "getrandom".to_string(),
        "statx".to_string(),
    ]
}

impl Default for SyscallAnomalyConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            strict_mode: false,
            alert_on_unknown_syscall: default_true(),
            max_denied_events_per_minute: default_syscall_anomaly_max_denied_events_per_minute(),
            max_total_events_per_minute: default_syscall_anomaly_max_total_events_per_minute(),
            max_alerts_per_minute: default_syscall_anomaly_max_alerts_per_minute(),
            alert_cooldown_secs: default_syscall_anomaly_alert_cooldown_secs(),
            log_path: default_syscall_anomaly_log_path(),
            baseline_syscalls: default_syscall_anomaly_baseline_syscalls(),
        }
    }
}

/// DingTalk configuration for Stream Mode messaging
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DingTalkConfig {
    /// Client ID (AppKey) from DingTalk developer console
    pub client_id: String,
    /// Client Secret (AppSecret) from DingTalk developer console
    pub client_secret: String,
    /// Allowed user IDs (staff IDs). Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

impl ChannelConfig for DingTalkConfig {
    fn name() -> &'static str {
        "DingTalk"
    }
    fn desc() -> &'static str {
        "DingTalk Stream Mode"
    }
}

/// QQ Official Bot configuration (Tencent QQ Bot SDK)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum QQReceiveMode {
    Websocket,
    #[default]
    Webhook,
}

/// QQ Official Bot configuration (Tencent QQ Bot SDK)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QQConfig {
    /// App ID from QQ Bot developer console
    pub app_id: String,
    /// App Secret from QQ Bot developer console
    pub app_secret: String,
    /// Allowed user IDs. Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Event receive mode: "webhook" (default) or "websocket".
    #[serde(default)]
    pub receive_mode: QQReceiveMode,
}

impl ChannelConfig for QQConfig {
    fn name() -> &'static str {
        "QQ Official"
    }
    fn desc() -> &'static str {
        "Tencent QQ Bot"
    }
}

/// Nostr channel configuration (NIP-04 + NIP-17 private messages)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NostrConfig {
    /// Private key in hex or nsec bech32 format
    pub private_key: String,
    /// Relay URLs (wss://). Defaults to popular public relays if omitted.
    #[serde(default = "default_nostr_relays")]
    pub relays: Vec<String>,
    /// Allowed sender public keys (hex or npub). Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_pubkeys: Vec<String>,
}

impl ChannelConfig for NostrConfig {
    fn name() -> &'static str {
        "Nostr"
    }
    fn desc() -> &'static str {
        "Nostr DMs"
    }
}

pub fn default_nostr_relays() -> Vec<String> {
    vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.primal.net".to_string(),
        "wss://relay.snort.social".to_string(),
    ]
}

// ── Config impl ──────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        let home =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let topclaw_dir = home.join(".topclaw");

        Self {
            workspace_dir: topclaw_dir.join("workspace"),
            config_path: topclaw_dir.join("config.toml"),
            api_key: None,
            api_url: None,
            default_provider: Some("openrouter".to_string()),
            provider_api: None,
            default_model: Some("anthropic/claude-sonnet-4.6".to_string()),
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
            self_improvement: SelfImprovementConfig::default(),
            goal_loop: GoalLoopConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            agents: HashMap::new(),
            coordination: CoordinationConfig::default(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            transcription: TranscriptionConfig::default(),
            agents_ipc: AgentsIpcConfig::default(),
            model_support_vision: None,
        }
    }
}

fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";

#[derive(Debug, Serialize, Deserialize)]
struct ActiveWorkspaceState {
    config_dir: String,
}

fn default_config_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".topclaw"))
}

fn active_workspace_state_path(marker_root: &Path) -> PathBuf {
    marker_root.join(ACTIVE_WORKSPACE_STATE_FILE)
}

/// Returns `true` if `path` lives under the OS temp directory.
fn is_temp_directory(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    // Canonicalize when possible to handle symlinks (macOS /var → /private/var)
    let canon_temp = temp.canonicalize().unwrap_or_else(|_| temp.clone());
    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canon_path.starts_with(&canon_temp)
}

async fn load_persisted_workspace_dirs(
    default_config_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let state_path = active_workspace_state_path(default_config_dir);
    if !state_path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&state_path).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "Failed to read active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let state: ActiveWorkspaceState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "Failed to parse active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let raw_config_dir = state.config_dir.trim();
    if raw_config_dir.is_empty() {
        tracing::warn!(
            "Ignoring active workspace marker {} because config_dir is empty",
            state_path.display()
        );
        return Ok(None);
    }

    let parsed_dir = PathBuf::from(raw_config_dir);
    let config_dir = if parsed_dir.is_absolute() {
        parsed_dir
    } else {
        default_config_dir.join(parsed_dir)
    };
    Ok(Some((config_dir.clone(), config_dir.join("workspace"))))
}

async fn remove_active_workspace_marker(marker_root: &Path) -> Result<()> {
    let state_path = active_workspace_state_path(marker_root);
    if !state_path.exists() {
        return Ok(());
    }

    fs::remove_file(&state_path).await.with_context(|| {
        format!(
            "Failed to clear active workspace marker: {}",
            state_path.display()
        )
    })?;

    if marker_root.exists() {
        sync_directory(marker_root).await?;
    }
    Ok(())
}

async fn write_active_workspace_marker(marker_root: &Path, config_dir: &Path) -> Result<()> {
    fs::create_dir_all(marker_root).await.with_context(|| {
        format!(
            "Failed to create active workspace marker root: {}",
            marker_root.display()
        )
    })?;

    let state = ActiveWorkspaceState {
        config_dir: config_dir.to_string_lossy().into_owned(),
    };
    let serialized =
        toml::to_string_pretty(&state).context("Failed to serialize active workspace marker")?;

    let temp_path = marker_root.join(format!(
        ".{ACTIVE_WORKSPACE_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, serialized).await.with_context(|| {
        format!(
            "Failed to write temporary active workspace marker: {}",
            temp_path.display()
        )
    })?;

    let state_path = active_workspace_state_path(marker_root);
    if let Err(error) = fs::rename(&temp_path, &state_path).await {
        let _ = fs::remove_file(&temp_path).await;
        anyhow::bail!(
            "Failed to atomically persist active workspace marker {}: {error}",
            state_path.display()
        );
    }

    sync_directory(marker_root).await?;
    Ok(())
}

pub(crate) async fn persist_active_workspace_config_dir(config_dir: &Path) -> Result<()> {
    let default_config_dir = default_config_dir()?;

    // Guard: never persist a temp-directory path as the active workspace.
    // This prevents transient test runs or one-off invocations from hijacking
    // the daemon's config resolution.
    #[cfg(not(test))]
    if is_temp_directory(config_dir) {
        tracing::warn!(
            path = %config_dir.display(),
            "Refusing to persist temp directory as active workspace marker"
        );
        return Ok(());
    }

    if config_dir == default_config_dir {
        remove_active_workspace_marker(&default_config_dir).await?;
        return Ok(());
    }

    // Primary marker lives with the selected config root to keep custom-home
    // layouts self-contained and writable in restricted environments.
    write_active_workspace_marker(config_dir, config_dir).await?;

    // Mirror into the default HOME-scoped root as a best-effort pointer for
    // later auto-discovery. Failure here must not break onboarding/update flows.
    if let Err(error) = write_active_workspace_marker(&default_config_dir, config_dir).await {
        tracing::warn!(
            selected_config_dir = %config_dir.display(),
            default_config_dir = %default_config_dir.display(),
            "Failed to mirror active workspace marker to default HOME config root; continuing with selected-root marker only: {error}"
        );
    }

    Ok(())
}

pub(crate) fn resolve_config_dir_for_workspace(workspace_dir: &Path) -> (PathBuf, PathBuf) {
    let workspace_config_dir = workspace_dir.to_path_buf();
    if workspace_config_dir.join("config.toml").exists() {
        return (
            workspace_config_dir.clone(),
            workspace_config_dir.join("workspace"),
        );
    }

    let legacy_config_dir = workspace_dir.parent().map(|parent| parent.join(".topclaw"));
    if let Some(legacy_dir) = legacy_config_dir {
        if legacy_dir.join("config.toml").exists() {
            return (legacy_dir, workspace_config_dir);
        }

        if workspace_dir
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return (legacy_dir, workspace_config_dir);
        }
    }

    (
        workspace_config_dir.clone(),
        workspace_config_dir.join("workspace"),
    )
}

/// Resolve the current runtime config/workspace directories for onboarding flows.
///
/// This mirrors the same precedence used by `Config::load_or_init()`:
/// `TOPCLAW_CONFIG_DIR` > `TOPCLAW_WORKSPACE` > active workspace marker > defaults.
pub(crate) async fn resolve_runtime_dirs_for_onboarding() -> Result<(PathBuf, PathBuf)> {
    let (default_topclaw_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
    let (config_dir, workspace_dir, _) =
        resolve_runtime_config_dirs(&default_topclaw_dir, &default_workspace_dir).await?;
    Ok((config_dir, workspace_dir))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConfigResolutionSource {
    EnvConfigDir,
    EnvWorkspace,
    ActiveWorkspaceMarker,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvConfigDir => "TOPCLAW_CONFIG_DIR",
            Self::EnvWorkspace => "TOPCLAW_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

async fn resolve_runtime_config_dirs(
    default_topclaw_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_config_dir) = std::env::var("TOPCLAW_CONFIG_DIR") {
        let custom_config_dir = custom_config_dir.trim();
        if !custom_config_dir.is_empty() {
            let topclaw_dir = PathBuf::from(custom_config_dir);
            return Ok((
                topclaw_dir.clone(),
                topclaw_dir.join("workspace"),
                ConfigResolutionSource::EnvConfigDir,
            ));
        }
    }

    if let Ok(custom_workspace) = std::env::var("TOPCLAW_WORKSPACE") {
        if !custom_workspace.is_empty() {
            let (topclaw_dir, workspace_dir) =
                resolve_config_dir_for_workspace(&PathBuf::from(custom_workspace));
            return Ok((
                topclaw_dir,
                workspace_dir,
                ConfigResolutionSource::EnvWorkspace,
            ));
        }
    }

    if let Some((topclaw_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_topclaw_dir).await?
    {
        return Ok((
            topclaw_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    Ok((
        default_topclaw_dir.to_path_buf(),
        default_workspace_dir.to_path_buf(),
        ConfigResolutionSource::DefaultConfigDir,
    ))
}

fn decrypt_optional_secret(
    store: &crate::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if crate::security::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .decrypt(&raw)
                    .with_context(|| format!("Failed to decrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

fn decrypt_secret(
    store: &crate::security::SecretStore,
    value: &mut String,
    field_name: &str,
) -> Result<()> {
    if crate::security::SecretStore::is_encrypted(value) {
        *value = store
            .decrypt(value)
            .with_context(|| format!("Failed to decrypt {field_name}"))?;
    }
    Ok(())
}

fn decrypt_vec_secrets(
    store: &crate::security::SecretStore,
    values: &mut [String],
    field_name: &str,
) -> Result<()> {
    for (idx, value) in values.iter_mut().enumerate() {
        if crate::security::SecretStore::is_encrypted(value) {
            *value = store
                .decrypt(value)
                .with_context(|| format!("Failed to decrypt {field_name}[{idx}]"))?;
        }
    }
    Ok(())
}

fn decrypt_map_secrets(
    store: &crate::security::SecretStore,
    values: &mut std::collections::HashMap<String, String>,
    field_name: &str,
) -> Result<()> {
    for (key, value) in values.iter_mut() {
        if crate::security::SecretStore::is_encrypted(value) {
            *value = store
                .decrypt(value)
                .with_context(|| format!("Failed to decrypt {field_name}.{key}"))?;
        }
    }
    Ok(())
}

fn encrypt_optional_secret(
    store: &crate::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if !crate::security::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .encrypt(&raw)
                    .with_context(|| format!("Failed to encrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

fn encrypt_secret(
    store: &crate::security::SecretStore,
    value: &mut String,
    field_name: &str,
) -> Result<()> {
    if !crate::security::SecretStore::is_encrypted(value) {
        *value = store
            .encrypt(value)
            .with_context(|| format!("Failed to encrypt {field_name}"))?;
    }
    Ok(())
}

fn encrypt_vec_secrets(
    store: &crate::security::SecretStore,
    values: &mut [String],
    field_name: &str,
) -> Result<()> {
    for (idx, value) in values.iter_mut().enumerate() {
        if !crate::security::SecretStore::is_encrypted(value) {
            *value = store
                .encrypt(value)
                .with_context(|| format!("Failed to encrypt {field_name}[{idx}]"))?;
        }
    }
    Ok(())
}

fn encrypt_map_secrets(
    store: &crate::security::SecretStore,
    values: &mut std::collections::HashMap<String, String>,
    field_name: &str,
) -> Result<()> {
    for (key, value) in values.iter_mut() {
        if !crate::security::SecretStore::is_encrypted(value) {
            *value = store
                .encrypt(value)
                .with_context(|| format!("Failed to encrypt {field_name}.{key}"))?;
        }
    }
    Ok(())
}

fn decrypt_channel_secrets(
    store: &crate::security::SecretStore,
    channels: &mut ChannelsConfig,
) -> Result<()> {
    if let Some(ref mut telegram) = channels.telegram {
        decrypt_secret(
            store,
            &mut telegram.bot_token,
            "config.channels_config.telegram.bot_token",
        )?;
    }
    if let Some(ref mut discord) = channels.discord {
        decrypt_secret(
            store,
            &mut discord.bot_token,
            "config.channels_config.discord.bot_token",
        )?;
    }
    if let Some(ref mut slack) = channels.slack {
        decrypt_secret(
            store,
            &mut slack.bot_token,
            "config.channels_config.slack.bot_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut slack.app_token,
            "config.channels_config.slack.app_token",
        )?;
    }
    if let Some(ref mut mattermost) = channels.mattermost {
        decrypt_secret(
            store,
            &mut mattermost.bot_token,
            "config.channels_config.mattermost.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        decrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    if let Some(ref mut bridge) = channels.bridge {
        if !bridge.auth_token.trim().is_empty() {
            decrypt_secret(
                store,
                &mut bridge.auth_token,
                "config.channels_config.bridge.auth_token",
            )?;
        }
    }
    if let Some(ref mut matrix) = channels.matrix {
        decrypt_secret(
            store,
            &mut matrix.access_token,
            "config.channels_config.matrix.access_token",
        )?;
    }
    if let Some(ref mut whatsapp) = channels.whatsapp {
        decrypt_optional_secret(
            store,
            &mut whatsapp.access_token,
            "config.channels_config.whatsapp.access_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut whatsapp.app_secret,
            "config.channels_config.whatsapp.app_secret",
        )?;
        decrypt_optional_secret(
            store,
            &mut whatsapp.verify_token,
            "config.channels_config.whatsapp.verify_token",
        )?;
    }
    if let Some(ref mut linq) = channels.linq {
        decrypt_secret(
            store,
            &mut linq.api_token,
            "config.channels_config.linq.api_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut linq.signing_secret,
            "config.channels_config.linq.signing_secret",
        )?;
    }
    if let Some(ref mut nextcloud) = channels.nextcloud_talk {
        decrypt_secret(
            store,
            &mut nextcloud.app_token,
            "config.channels_config.nextcloud_talk.app_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut nextcloud.webhook_secret,
            "config.channels_config.nextcloud_talk.webhook_secret",
        )?;
    }
    if let Some(ref mut irc) = channels.irc {
        decrypt_optional_secret(
            store,
            &mut irc.server_password,
            "config.channels_config.irc.server_password",
        )?;
        decrypt_optional_secret(
            store,
            &mut irc.nickserv_password,
            "config.channels_config.irc.nickserv_password",
        )?;
        decrypt_optional_secret(
            store,
            &mut irc.sasl_password,
            "config.channels_config.irc.sasl_password",
        )?;
    }
    if let Some(ref mut lark) = channels.lark {
        decrypt_secret(
            store,
            &mut lark.app_secret,
            "config.channels_config.lark.app_secret",
        )?;
        decrypt_optional_secret(
            store,
            &mut lark.encrypt_key,
            "config.channels_config.lark.encrypt_key",
        )?;
        decrypt_optional_secret(
            store,
            &mut lark.verification_token,
            "config.channels_config.lark.verification_token",
        )?;
    }
    if let Some(ref mut dingtalk) = channels.dingtalk {
        decrypt_secret(
            store,
            &mut dingtalk.client_secret,
            "config.channels_config.dingtalk.client_secret",
        )?;
    }
    if let Some(ref mut qq) = channels.qq {
        decrypt_secret(
            store,
            &mut qq.app_secret,
            "config.channels_config.qq.app_secret",
        )?;
    }
    if let Some(ref mut nostr) = channels.nostr {
        decrypt_secret(
            store,
            &mut nostr.private_key,
            "config.channels_config.nostr.private_key",
        )?;
    }
    if let Some(ref mut clawdtalk) = channels.clawdtalk {
        decrypt_secret(
            store,
            &mut clawdtalk.api_key,
            "config.channels_config.clawdtalk.api_key",
        )?;
        decrypt_optional_secret(
            store,
            &mut clawdtalk.webhook_secret,
            "config.channels_config.clawdtalk.webhook_secret",
        )?;
    }
    Ok(())
}

fn parse_telegram_allowed_users_env_value(
    raw_value: &str,
    env_name: &str,
    field_name: &str,
) -> Result<Vec<String>> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{field_name} env reference ${{env:{env_name}}} resolved to an empty value");
    }

    let mut resolved: Vec<String> = Vec::new();
    if trimmed.starts_with('[') {
        let parsed: serde_json::Value = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "{field_name} env reference ${{env:{env_name}}} must be valid JSON array or comma-separated list"
            )
        })?;
        let items = parsed.as_array().with_context(|| {
            format!("{field_name} env reference ${{env:{env_name}}} must be a JSON array")
        })?;
        for (idx, item) in items.iter().enumerate() {
            let candidate = match item {
                serde_json::Value::String(v) => v.trim().to_string(),
                serde_json::Value::Number(v) => v.to_string(),
                _ => {
                    anyhow::bail!(
                        "{field_name} env reference ${{env:{env_name}}}[{idx}] must be string or number"
                    );
                }
            };
            if !candidate.is_empty() {
                resolved.push(candidate);
            }
        }
    } else {
        resolved.extend(
            trimmed
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }

    if resolved.is_empty() {
        anyhow::bail!("{field_name} env reference ${{env:{env_name}}} produced no user IDs");
    }

    Ok(resolved)
}

fn resolve_telegram_allowed_users_env_refs(channels: &mut ChannelsConfig) -> Result<()> {
    let Some(telegram) = channels.telegram.as_mut() else {
        return Ok(());
    };

    let field_name = "config.channels_config.telegram.allowed_users";
    let mut expanded_allowed_users: Vec<String> = Vec::new();
    for (idx, raw_entry) in telegram.allowed_users.drain(..).enumerate() {
        let entry = raw_entry.trim();
        if entry.is_empty() {
            continue;
        }

        if let Some(env_expr) = entry
            .strip_prefix("${env:")
            .and_then(|value| value.strip_suffix('}'))
        {
            let env_name = env_expr.trim();
            if !is_valid_env_var_name(env_name) {
                anyhow::bail!(
                    "{field_name}[{idx}] has invalid env var name ({env_name}); expected [A-Za-z_][A-Za-z0-9_]*"
                );
            }
            let env_value = std::env::var(env_name).with_context(|| {
                format!("{field_name}[{idx}] references unset environment variable {env_name}")
            })?;
            let mut parsed =
                parse_telegram_allowed_users_env_value(&env_value, env_name, field_name)?;
            expanded_allowed_users.append(&mut parsed);
        } else {
            expanded_allowed_users.push(entry.to_string());
        }
    }

    telegram.allowed_users = expanded_allowed_users;
    Ok(())
}

fn encrypt_channel_secrets(
    store: &crate::security::SecretStore,
    channels: &mut ChannelsConfig,
) -> Result<()> {
    if let Some(ref mut telegram) = channels.telegram {
        encrypt_secret(
            store,
            &mut telegram.bot_token,
            "config.channels_config.telegram.bot_token",
        )?;
    }
    if let Some(ref mut discord) = channels.discord {
        encrypt_secret(
            store,
            &mut discord.bot_token,
            "config.channels_config.discord.bot_token",
        )?;
    }
    if let Some(ref mut slack) = channels.slack {
        encrypt_secret(
            store,
            &mut slack.bot_token,
            "config.channels_config.slack.bot_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut slack.app_token,
            "config.channels_config.slack.app_token",
        )?;
    }
    if let Some(ref mut mattermost) = channels.mattermost {
        encrypt_secret(
            store,
            &mut mattermost.bot_token,
            "config.channels_config.mattermost.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        encrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    if let Some(ref mut bridge) = channels.bridge {
        if !bridge.auth_token.trim().is_empty() {
            encrypt_secret(
                store,
                &mut bridge.auth_token,
                "config.channels_config.bridge.auth_token",
            )?;
        }
    }
    if let Some(ref mut matrix) = channels.matrix {
        encrypt_secret(
            store,
            &mut matrix.access_token,
            "config.channels_config.matrix.access_token",
        )?;
    }
    if let Some(ref mut whatsapp) = channels.whatsapp {
        encrypt_optional_secret(
            store,
            &mut whatsapp.access_token,
            "config.channels_config.whatsapp.access_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut whatsapp.app_secret,
            "config.channels_config.whatsapp.app_secret",
        )?;
        encrypt_optional_secret(
            store,
            &mut whatsapp.verify_token,
            "config.channels_config.whatsapp.verify_token",
        )?;
    }
    if let Some(ref mut linq) = channels.linq {
        encrypt_secret(
            store,
            &mut linq.api_token,
            "config.channels_config.linq.api_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut linq.signing_secret,
            "config.channels_config.linq.signing_secret",
        )?;
    }
    if let Some(ref mut nextcloud) = channels.nextcloud_talk {
        encrypt_secret(
            store,
            &mut nextcloud.app_token,
            "config.channels_config.nextcloud_talk.app_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut nextcloud.webhook_secret,
            "config.channels_config.nextcloud_talk.webhook_secret",
        )?;
    }
    if let Some(ref mut irc) = channels.irc {
        encrypt_optional_secret(
            store,
            &mut irc.server_password,
            "config.channels_config.irc.server_password",
        )?;
        encrypt_optional_secret(
            store,
            &mut irc.nickserv_password,
            "config.channels_config.irc.nickserv_password",
        )?;
        encrypt_optional_secret(
            store,
            &mut irc.sasl_password,
            "config.channels_config.irc.sasl_password",
        )?;
    }
    if let Some(ref mut lark) = channels.lark {
        encrypt_secret(
            store,
            &mut lark.app_secret,
            "config.channels_config.lark.app_secret",
        )?;
        encrypt_optional_secret(
            store,
            &mut lark.encrypt_key,
            "config.channels_config.lark.encrypt_key",
        )?;
        encrypt_optional_secret(
            store,
            &mut lark.verification_token,
            "config.channels_config.lark.verification_token",
        )?;
    }
    if let Some(ref mut dingtalk) = channels.dingtalk {
        encrypt_secret(
            store,
            &mut dingtalk.client_secret,
            "config.channels_config.dingtalk.client_secret",
        )?;
    }
    if let Some(ref mut qq) = channels.qq {
        encrypt_secret(
            store,
            &mut qq.app_secret,
            "config.channels_config.qq.app_secret",
        )?;
    }
    if let Some(ref mut nostr) = channels.nostr {
        encrypt_secret(
            store,
            &mut nostr.private_key,
            "config.channels_config.nostr.private_key",
        )?;
    }
    if let Some(ref mut clawdtalk) = channels.clawdtalk {
        encrypt_secret(
            store,
            &mut clawdtalk.api_key,
            "config.channels_config.clawdtalk.api_key",
        )?;
        encrypt_optional_secret(
            store,
            &mut clawdtalk.webhook_secret,
            "config.channels_config.clawdtalk.webhook_secret",
        )?;
    }
    Ok(())
}

fn config_dir_creation_error(path: &Path) -> String {
    format!(
        "Failed to create config directory: {}. If running as an OpenRC service, \
         ensure this path is writable by user 'topclaw'.",
        path.display()
    )
}

fn is_local_ollama_endpoint(api_url: Option<&str>) -> bool {
    let Some(raw) = api_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };

    reqwest::Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "0.0.0.0"))
}

fn has_ollama_cloud_credential(config_api_key: Option<&str>) -> bool {
    let config_key_present = config_api_key
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if config_key_present {
        return true;
    }

    ["OLLAMA_API_KEY", "TOPCLAW_API_KEY"].iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn normalize_wire_api(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "responses" => Some("responses"),
        "chat_completions" | "chat-completions" | "chat" | "chatcompletions" => {
            Some("chat_completions")
        }
        _ => None,
    }
}

fn read_codex_openai_api_key() -> Option<String> {
    let home = UserDirs::new()?.home_dir().to_path_buf();
    let auth_path = home.join(".codex").join("auth.json");
    let raw = std::fs::read_to_string(auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;

    parsed
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn normalize_top_level_table_aliases(raw_toml: &mut toml::Value) {
    let Some(root) = raw_toml.as_table_mut() else {
        return;
    };

    if root.contains_key("Gateway") {
        if root.contains_key("gateway") {
            let _ = root.remove("Gateway");
            tracing::warn!("Legacy table [Gateway] ignored because [gateway] is already present.");
        } else if let Some(value) = root.remove("Gateway") {
            root.insert("gateway".to_string(), value);
            tracing::warn!("Legacy table [Gateway] mapped to [gateway].");
        }
    }
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
            let mut raw_toml: toml::Value =
                toml::from_str(&contents).context("Failed to parse config file")?;
            normalize_top_level_table_aliases(&mut raw_toml);
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
            let store = crate::security::SecretStore::new(&topclaw_dir, config.secrets.encrypt);
            decrypt_optional_secret(&store, &mut config.api_key, "config.api_key")?;
            decrypt_optional_secret(
                &store,
                &mut config.composio.api_key,
                "config.composio.api_key",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.proxy.http_proxy,
                "config.proxy.http_proxy",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.proxy.https_proxy,
                "config.proxy.https_proxy",
            )?;
            decrypt_optional_secret(
                &store,
                &mut config.proxy.all_proxy,
                "config.proxy.all_proxy",
            )?;

            decrypt_optional_secret(
                &store,
                &mut config.browser.computer_use.api_key,
                "config.browser.computer_use.api_key",
            )?;

            decrypt_optional_secret(
                &store,
                &mut config.web_search.brave_api_key,
                "config.web_search.brave_api_key",
            )?;

            decrypt_optional_secret(
                &store,
                &mut config.storage.provider.config.db_url,
                "config.storage.provider.config.db_url",
            )?;
            decrypt_vec_secrets(
                &store,
                &mut config.reliability.api_keys,
                "config.reliability.api_keys",
            )?;
            decrypt_map_secrets(
                &store,
                &mut config.reliability.fallback_api_keys,
                "config.reliability.fallback_api_keys",
            )?;
            decrypt_vec_secrets(
                &store,
                &mut config.gateway.paired_tokens,
                "config.gateway.paired_tokens",
            )?;

            for agent in config.agents.values_mut() {
                decrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
            }

            decrypt_channel_secrets(&store, &mut config.channels_config)?;
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

    fn lookup_model_provider_profile(
        &self,
        provider_name: &str,
    ) -> Option<(String, ModelProviderConfig)> {
        let needle = provider_name.trim();
        if needle.is_empty() {
            return None;
        }

        self.model_providers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(needle))
            .map(|(name, profile)| (name.clone(), profile.clone()))
    }

    fn apply_named_model_provider_profile(&mut self) {
        let Some(current_provider) = self.default_provider.clone() else {
            return;
        };

        let Some((profile_key, profile)) = self.lookup_model_provider_profile(&current_provider)
        else {
            return;
        };

        let base_url = profile
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        if self
            .api_url
            .as_deref()
            .map(str::trim)
            .is_none_or(|value| value.is_empty())
        {
            if let Some(base_url) = base_url.as_ref() {
                self.api_url = Some(base_url.clone());
            }
        }

        if profile.requires_openai_auth
            && self
                .api_key
                .as_deref()
                .map(str::trim)
                .is_none_or(|value| value.is_empty())
        {
            let codex_key = std::env::var("OPENAI_API_KEY")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(read_codex_openai_api_key);
            if let Some(codex_key) = codex_key {
                self.api_key = Some(codex_key);
            }
        }

        let normalized_wire_api = profile.wire_api.as_deref().and_then(normalize_wire_api);
        let profile_name = profile
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if normalized_wire_api == Some("responses") {
            self.default_provider = Some("openai-codex".to_string());
            return;
        }

        if let Some(profile_name) = profile_name {
            if !profile_name.eq_ignore_ascii_case(&profile_key) {
                self.default_provider = Some(profile_name.to_string());
                return;
            }
        }

        if let Some(base_url) = base_url {
            self.default_provider = Some(format!("custom:{base_url}"));
        }
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

        for (profile_key, profile) in &self.model_providers {
            let profile_name = profile_key.trim();
            if profile_name.is_empty() {
                anyhow::bail!("model_providers contains an empty profile name");
            }

            let has_name = profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            let has_base_url = profile
                .base_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());

            if !has_name && !has_base_url {
                anyhow::bail!(
                    "model_providers.{profile_name} must define at least one of `name` or `base_url`"
                );
            }

            if let Some(base_url) = profile.base_url.as_deref().map(str::trim) {
                if !base_url.is_empty() {
                    let parsed = reqwest::Url::parse(base_url).with_context(|| {
                        format!("model_providers.{profile_name}.base_url is not a valid URL")
                    })?;
                    if !matches!(parsed.scheme(), "http" | "https") {
                        anyhow::bail!(
                            "model_providers.{profile_name}.base_url must use http/https"
                        );
                    }
                }
            }

            if let Some(wire_api) = profile.wire_api.as_deref().map(str::trim) {
                if !wire_api.is_empty() && normalize_wire_api(wire_api).is_none() {
                    anyhow::bail!(
                        "model_providers.{profile_name}.wire_api must be one of: responses, chat_completions"
                    );
                }
            }
        }

        // Ollama cloud-routing safety checks
        if self
            .default_provider
            .as_deref()
            .is_some_and(|provider| provider.trim().eq_ignore_ascii_case("ollama"))
            && self
                .default_model
                .as_deref()
                .is_some_and(|model| model.trim().ends_with(":cloud"))
        {
            if is_local_ollama_endpoint(self.api_url.as_deref()) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset. Set api_url to a remote Ollama endpoint (for example https://ollama.com)."
                );
            }

            if !has_ollama_cloud_credential(self.api_key.as_deref()) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but no API key is configured. Set api_key or OLLAMA_API_KEY."
                );
            }
        }

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
        // API Key: GLM_API_KEY overrides when provider is a GLM/Zhipu variant.
        if self.default_provider.as_deref().is_some_and(is_glm_alias) {
            if let Ok(key) = std::env::var("GLM_API_KEY") {
                if !key.is_empty() {
                    self.api_key = Some(key);
                }
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
        let store = crate::security::SecretStore::new(topclaw_dir, self.secrets.encrypt);

        encrypt_optional_secret(&store, &mut config_to_save.api_key, "config.api_key")?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.composio.api_key,
            "config.composio.api_key",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.proxy.http_proxy,
            "config.proxy.http_proxy",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.proxy.https_proxy,
            "config.proxy.https_proxy",
        )?;
        encrypt_optional_secret(
            &store,
            &mut config_to_save.proxy.all_proxy,
            "config.proxy.all_proxy",
        )?;

        encrypt_optional_secret(
            &store,
            &mut config_to_save.browser.computer_use.api_key,
            "config.browser.computer_use.api_key",
        )?;

        encrypt_optional_secret(
            &store,
            &mut config_to_save.web_search.brave_api_key,
            "config.web_search.brave_api_key",
        )?;

        encrypt_optional_secret(
            &store,
            &mut config_to_save.storage.provider.config.db_url,
            "config.storage.provider.config.db_url",
        )?;
        encrypt_vec_secrets(
            &store,
            &mut config_to_save.reliability.api_keys,
            "config.reliability.api_keys",
        )?;
        encrypt_map_secrets(
            &store,
            &mut config_to_save.reliability.fallback_api_keys,
            "config.reliability.fallback_api_keys",
        )?;
        encrypt_vec_secrets(
            &store,
            &mut config_to_save.gateway.paired_tokens,
            "config.gateway.paired_tokens",
        )?;

        for agent in config_to_save.agents.values_mut() {
            encrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
        }

        encrypt_channel_secrets(&store, &mut config_to_save.channels_config)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::autonomy::default_non_cli_excluded_tools;
    use crate::config::build_runtime_proxy_client;
    use crate::config::build_runtime_proxy_client_with_timeouts;
    use crate::config::proxy::{
        clear_runtime_proxy_client_cache, runtime_proxy_cache_key, runtime_proxy_client_cache,
    };
    use crate::config::BrowserComputerUseConfig;
    use crate::config::NodeControlConfig;
    use crate::config::NonCliNaturalLanguageApprovalMode;
    use crate::config::OtpMethod;
    use crate::config::PeripheralBoardConfig;
    use crate::config::SkillsPromptInjectionMode;
    use crate::config::WasmCapabilityEscalationMode;
    use crate::config::WasmModuleHashPolicy;
    use crate::security::{AutonomyLevel, ShellRedirectPolicy};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::sync::{Mutex, MutexGuard};
    use tokio::test;
    use tokio_stream::wrappers::ReadDirStream;
    use tokio_stream::StreamExt;

    // ── Defaults ─────────────────────────────────────────────

    #[test]
    async fn http_request_config_default_has_correct_values() {
        let cfg = HttpRequestConfig::default();
        assert_eq!(cfg.timeout_secs, 30);
        assert_eq!(cfg.max_response_size, 1_000_000);
        assert!(!cfg.enabled);
        assert!(cfg.allowed_domains.is_empty());
    }

    #[test]
    async fn config_default_has_sane_values() {
        let c = Config::default();
        assert_eq!(c.default_provider.as_deref(), Some("openrouter"));
        assert!(c.default_model.as_deref().unwrap().contains("claude"));
        assert!((c.default_temperature - 0.7).abs() < f64::EPSILON);
        assert!(c.api_key.is_none());
        assert!(!c.skills.open_skills_enabled);
        assert_eq!(
            c.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );
        assert!(c.workspace_dir.to_string_lossy().contains("workspace"));
        assert!(c.config_path.to_string_lossy().contains("config.toml"));
    }

    #[test]
    async fn config_debug_redacts_sensitive_values() {
        let mut config = Config::default();
        config.workspace_dir = PathBuf::from("/tmp/workspace");
        config.config_path = PathBuf::from("/tmp/config.toml");
        config.api_key = Some("root-credential".into());
        config.storage.provider.config.db_url = Some("postgres://user:pw@host/db".into());
        config.browser.computer_use.api_key = Some("browser-credential".into());
        config.gateway.paired_tokens = vec!["zc_0123456789abcdef".into()];
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "telegram-credential".into(),
            allowed_users: Vec::new(),
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });
        config.agents.insert(
            "worker".into(),
            DelegateAgentConfig {
                provider: "openrouter".into(),
                model: "model-test".into(),
                system_prompt: None,
                api_key: Some("agent-credential".into()),
                temperature: None,
                max_depth: 3,
                agentic: false,
                allowed_tools: Vec::new(),
                max_iterations: 10,
            },
        );

        let debug_output = format!("{config:?}");
        assert!(debug_output.contains("***REDACTED***"));

        for (idx, secret) in [
            "root-credential",
            "postgres://user:pw@host/db",
            "browser-credential",
            "zc_0123456789abcdef",
            "telegram-credential",
            "agent-credential",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                !debug_output.contains(secret),
                "debug output leaked secret value at index {idx}"
            );
        }

        assert!(!debug_output.contains("paired_tokens"));
        assert!(!debug_output.contains("bot_token"));
        assert!(!debug_output.contains("db_url"));
    }

    #[test]
    async fn config_dir_creation_error_mentions_openrc_and_path() {
        let msg = config_dir_creation_error(Path::new("/etc/topclaw"));
        assert!(msg.contains("/etc/topclaw"));
        assert!(msg.contains("OpenRC"));
        assert!(msg.contains("topclaw"));
    }

    #[test]
    async fn config_schema_export_contains_expected_contract_shape() {
        let schema = schemars::schema_for!(Config);
        let schema_json = serde_json::to_value(&schema).expect("schema should serialize to json");

        assert_eq!(
            schema_json
                .get("$schema")
                .and_then(serde_json::Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema")
        );

        let properties = schema_json
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose top-level properties");

        assert!(properties.contains_key("default_provider"));
        assert!(properties.contains_key("skills"));
        assert!(properties.contains_key("gateway"));
        assert!(properties.contains_key("channels_config"));
        assert!(!properties.contains_key("workspace_dir"));
        assert!(!properties.contains_key("config_path"));

        assert!(
            schema_json
                .get("$defs")
                .and_then(serde_json::Value::as_object)
                .is_some(),
            "schema should include reusable type definitions"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn save_sets_config_permissions_on_new_file() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");

        let mut config = Config::default();
        config.config_path = config_path.clone();
        config.workspace_dir = workspace_dir;

        config.save().await.expect("save config");

        let mode = std::fs::metadata(&config_path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    async fn observability_config_default() {
        let o = ObservabilityConfig::default();
        assert_eq!(o.backend, "none");
        assert_eq!(o.runtime_trace_mode, "none");
        assert_eq!(o.runtime_trace_path, "state/runtime-trace.jsonl");
        assert_eq!(o.runtime_trace_max_entries, 200);
    }

    #[test]
    async fn autonomy_config_default() {
        let a = AutonomyConfig::default();
        assert_eq!(a.level, AutonomyLevel::Supervised);
        assert!(a.workspace_only);
        assert!(a.allowed_commands.is_empty());
        assert!(a.forbidden_paths.contains(&"/etc".to_string()));
        assert_eq!(a.max_actions_per_hour, 20);
        assert_eq!(a.max_cost_per_day_cents, 500);
        assert!(a.require_approval_for_medium_risk);
        assert!(a.block_high_risk_commands);
        assert_eq!(a.shell_redirect_policy, ShellRedirectPolicy::Block);
        assert!(a.shell_env_passthrough.is_empty());
        assert!(a.non_cli_excluded_tools.contains(&"shell".to_string()));
        assert!(a.non_cli_excluded_tools.contains(&"browser".to_string()));
    }

    #[test]
    async fn autonomy_config_serde_defaults_non_cli_excluded_tools() {
        let raw = r#"
level = "supervised"
workspace_only = true
allowed_commands = ["git"]
forbidden_paths = ["/etc"]
max_actions_per_hour = 20
max_cost_per_day_cents = 500
require_approval_for_medium_risk = true
block_high_risk_commands = true
shell_env_passthrough = []
auto_approve = ["file_read"]
always_ask = []
        allowed_roots = []
"#;
        let parsed: AutonomyConfig = toml::from_str(raw).unwrap();
        assert_eq!(parsed.shell_redirect_policy, ShellRedirectPolicy::Block);
        assert_eq!(
            parsed.non_cli_excluded_tools,
            default_non_cli_excluded_tools()
        );
    }

    #[test]
    async fn config_validate_rejects_duplicate_non_cli_excluded_tools() {
        let mut cfg = Config::default();
        cfg.autonomy.non_cli_excluded_tools = vec!["shell".into(), "shell".into()];
        let err = cfg.validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("autonomy.non_cli_excluded_tools contains duplicate entry"));
    }

    #[test]
    async fn runtime_config_default() {
        let r = RuntimeConfig::default();
        assert_eq!(r.kind, "native");
        assert_eq!(r.docker.image, "alpine:3.20");
        assert_eq!(r.docker.network, "none");
        assert_eq!(r.docker.memory_limit_mb, Some(512));
        assert_eq!(r.docker.cpu_limit, Some(1.0));
        assert!(r.docker.read_only_rootfs);
        assert!(r.docker.mount_workspace);
        assert_eq!(r.wasm.tools_dir, "tools/wasm");
        assert_eq!(r.wasm.fuel_limit, 1_000_000);
        assert_eq!(r.wasm.memory_limit_mb, 64);
        assert_eq!(r.wasm.max_module_size_mb, 50);
        assert!(!r.wasm.allow_workspace_read);
        assert!(!r.wasm.allow_workspace_write);
        assert!(r.wasm.allowed_hosts.is_empty());
        assert!(r.wasm.security.require_workspace_relative_tools_dir);
        assert!(r.wasm.security.reject_symlink_modules);
        assert!(r.wasm.security.reject_symlink_tools_dir);
        assert!(r.wasm.security.strict_host_validation);
        assert_eq!(
            r.wasm.security.capability_escalation_mode,
            WasmCapabilityEscalationMode::Deny
        );
        assert_eq!(
            r.wasm.security.module_hash_policy,
            WasmModuleHashPolicy::Warn
        );
        assert!(r.wasm.security.module_sha256.is_empty());
    }

    #[test]
    async fn heartbeat_config_default() {
        let h = HeartbeatConfig::default();
        assert!(!h.enabled);
        assert_eq!(h.interval_minutes, 30);
        assert!(h.message.is_none());
        assert!(h.target.is_none());
        assert!(h.to.is_none());
    }

    #[test]
    async fn cron_config_default() {
        let c = CronConfig::default();
        assert!(c.enabled);
        assert_eq!(c.max_run_history, 50);
    }

    #[test]
    async fn cron_config_serde_roundtrip() {
        let c = CronConfig {
            enabled: false,
            max_run_history: 100,
        };
        let json = serde_json::to_string(&c).unwrap();
        let parsed: CronConfig = serde_json::from_str(&json).unwrap();
        assert!(!parsed.enabled);
        assert_eq!(parsed.max_run_history, 100);
    }

    #[test]
    async fn config_defaults_cron_when_section_missing() {
        let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;

        let parsed: Config = toml::from_str(toml_str).unwrap();
        assert!(parsed.cron.enabled);
        assert_eq!(parsed.cron.max_run_history, 50);
    }

    #[test]
    async fn memory_config_default_hygiene_settings() {
        let m = MemoryConfig::default();
        assert_eq!(m.backend, "sqlite");
        assert!(m.auto_save);
        assert!(m.hygiene_enabled);
        assert_eq!(m.archive_after_days, 7);
        assert_eq!(m.purge_after_days, 30);
        assert_eq!(m.conversation_retention_days, 30);
        assert!(m.sqlite_open_timeout_secs.is_none());
    }

    #[test]
    async fn storage_provider_config_defaults() {
        let storage = StorageConfig::default();
        assert!(storage.provider.config.provider.is_empty());
        assert!(storage.provider.config.db_url.is_none());
        assert_eq!(storage.provider.config.schema, "public");
        assert_eq!(storage.provider.config.table, "memories");
        assert!(storage.provider.config.connect_timeout_secs.is_none());
    }

    #[test]
    async fn channels_config_default() {
        let c = ChannelsConfig::default();
        assert!(c.cli);
        assert!(c.telegram.is_none());
        assert!(c.discord.is_none());
    }

    // ── Serde round-trip ─────────────────────────────────────

    #[test]
    async fn config_toml_roundtrip() {
        let config = Config {
            workspace_dir: PathBuf::from("/tmp/test/workspace"),
            config_path: PathBuf::from("/tmp/test/config.toml"),
            api_key: Some("sk-test-key".into()),
            api_url: None,
            default_provider: Some("openrouter".into()),
            provider_api: None,
            default_model: Some("gpt-4o".into()),
            model_providers: HashMap::new(),
            provider: ProviderConfig::default(),
            default_temperature: 0.5,
            observability: ObservabilityConfig {
                backend: "log".into(),
                ..ObservabilityConfig::default()
            },
            autonomy: AutonomyConfig {
                level: AutonomyLevel::Full,
                workspace_only: false,
                allowed_commands: vec!["docker".into()],
                forbidden_paths: vec!["/secret".into()],
                max_actions_per_hour: 50,
                max_cost_per_day_cents: 1000,
                require_approval_for_medium_risk: false,
                block_high_risk_commands: true,
                shell_redirect_policy: ShellRedirectPolicy::Strip,
                shell_env_passthrough: vec!["DATABASE_URL".into()],
                auto_approve: vec!["file_read".into()],
                always_ask: vec![],
                allowed_roots: vec![],
                non_cli_excluded_tools: vec![],
                non_cli_approval_approvers: vec![],
                non_cli_natural_language_approval_mode:
                    NonCliNaturalLanguageApprovalMode::RequestConfirm,
                non_cli_natural_language_approval_mode_by_channel: HashMap::new(),
            },
            security: SecurityConfig::default(),
            runtime: RuntimeConfig {
                kind: "docker".into(),
                ..RuntimeConfig::default()
            },
            research: ResearchPhaseConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            coordination: CoordinationConfig::default(),
            skills: SkillsConfig::default(),
            workspaces: WorkspacesConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            query_classification: QueryClassificationConfig::default(),
            heartbeat: HeartbeatConfig {
                enabled: true,
                interval_minutes: 15,
                message: Some("Check London time".into()),
                target: Some("telegram".into()),
                to: Some("123456".into()),
            },
            cron: CronConfig::default(),
            self_improvement: SelfImprovementConfig::default(),
            goal_loop: GoalLoopConfig::default(),
            channels_config: ChannelsConfig {
                cli: true,
                bridge: None,
                telegram: Some(TelegramConfig {
                    bot_token: "123:ABC".into(),
                    allowed_users: vec!["user1".into()],
                    stream_mode: StreamMode::default(),
                    draft_update_interval_ms: default_draft_update_interval_ms(),
                    interrupt_on_new_message: false,
                    group_reply: None,
                    base_url: None,
                }),
                discord: None,
                slack: None,
                mattermost: None,
                webhook: None,
                imessage: None,
                matrix: None,
                signal: None,
                whatsapp: None,
                linq: None,
                wati: None,
                nextcloud_talk: None,
                email: None,
                irc: None,
                lark: None,
                feishu: None,
                dingtalk: None,
                qq: None,
                nostr: None,
                clawdtalk: None,
                message_timeout_secs: 300,
            },
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            agent: AgentConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            agents: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            transcription: TranscriptionConfig::default(),
            agents_ipc: AgentsIpcConfig::default(),
            model_support_vision: None,
        };

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.api_key, config.api_key);
        assert_eq!(parsed.default_provider, config.default_provider);
        assert_eq!(parsed.default_model, config.default_model);
        assert!((parsed.default_temperature - config.default_temperature).abs() < f64::EPSILON);
        assert_eq!(parsed.observability.backend, "log");
        assert_eq!(parsed.observability.runtime_trace_mode, "none");
        assert_eq!(parsed.autonomy.level, AutonomyLevel::Full);
        assert!(!parsed.autonomy.workspace_only);
        assert_eq!(parsed.runtime.kind, "docker");
        assert!(parsed.heartbeat.enabled);
        assert_eq!(parsed.heartbeat.interval_minutes, 15);
        assert_eq!(
            parsed.heartbeat.message.as_deref(),
            Some("Check London time")
        );
        assert_eq!(parsed.heartbeat.target.as_deref(), Some("telegram"));
        assert_eq!(parsed.heartbeat.to.as_deref(), Some("123456"));
        assert!(parsed.channels_config.telegram.is_some());
        assert_eq!(
            parsed.channels_config.telegram.unwrap().bot_token,
            "123:ABC"
        );
    }

    #[test]
    async fn config_minimal_toml_uses_defaults() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert!(parsed.api_key.is_none());
        assert!(parsed.default_provider.is_none());
        assert_eq!(parsed.observability.backend, "none");
        assert_eq!(parsed.observability.runtime_trace_mode, "none");
        assert_eq!(parsed.autonomy.level, AutonomyLevel::Supervised);
        assert!(parsed.autonomy.workspace_only);
        assert_eq!(parsed.runtime.kind, "native");
        assert!(!parsed.heartbeat.enabled);
        assert!(parsed.channels_config.cli);
        assert!(parsed.memory.hygiene_enabled);
        assert_eq!(parsed.memory.archive_after_days, 7);
        assert_eq!(parsed.memory.purge_after_days, 30);
        assert_eq!(parsed.memory.conversation_retention_days, 30);
    }

    #[test]
    async fn storage_provider_db_url_deserializes() {
        let raw = r#"
default_temperature = 0.7

[storage.provider.config]
provider = "postgres"
db_url = "postgres://postgres:postgres@localhost:5432/topclaw"
schema = "public"
table = "memories"
connect_timeout_secs = 12
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.storage.provider.config.provider, "postgres");
        assert_eq!(
            parsed.storage.provider.config.db_url.as_deref(),
            Some("postgres://postgres:postgres@localhost:5432/topclaw")
        );
        assert_eq!(parsed.storage.provider.config.schema, "public");
        assert_eq!(parsed.storage.provider.config.table, "memories");
        assert_eq!(
            parsed.storage.provider.config.connect_timeout_secs,
            Some(12)
        );
    }

    #[test]
    async fn runtime_reasoning_enabled_deserializes() {
        let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_enabled = false
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.runtime.reasoning_enabled, Some(false));
    }

    #[test]
    async fn runtime_wasm_deserializes() {
        let raw = r#"
default_temperature = 0.7

[runtime]
kind = "wasm"

[runtime.wasm]
tools_dir = "skills/wasm"
fuel_limit = 500000
memory_limit_mb = 32
max_module_size_mb = 8
allow_workspace_read = true
allow_workspace_write = false
allowed_hosts = ["api.example.com", "cdn.example.com:443"]

[runtime.wasm.security]
require_workspace_relative_tools_dir = false
reject_symlink_modules = false
reject_symlink_tools_dir = false
strict_host_validation = false
capability_escalation_mode = "clamp"
module_hash_policy = "enforce"

[runtime.wasm.security.module_sha256]
calc = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.runtime.kind, "wasm");
        assert_eq!(parsed.runtime.wasm.tools_dir, "skills/wasm");
        assert_eq!(parsed.runtime.wasm.fuel_limit, 500_000);
        assert_eq!(parsed.runtime.wasm.memory_limit_mb, 32);
        assert_eq!(parsed.runtime.wasm.max_module_size_mb, 8);
        assert!(parsed.runtime.wasm.allow_workspace_read);
        assert!(!parsed.runtime.wasm.allow_workspace_write);
        assert_eq!(
            parsed.runtime.wasm.allowed_hosts,
            vec!["api.example.com", "cdn.example.com:443"]
        );
        assert!(
            !parsed
                .runtime
                .wasm
                .security
                .require_workspace_relative_tools_dir
        );
        assert!(!parsed.runtime.wasm.security.reject_symlink_modules);
        assert!(!parsed.runtime.wasm.security.reject_symlink_tools_dir);
        assert!(!parsed.runtime.wasm.security.strict_host_validation);
        assert_eq!(
            parsed.runtime.wasm.security.capability_escalation_mode,
            WasmCapabilityEscalationMode::Clamp
        );
        assert_eq!(
            parsed.runtime.wasm.security.module_hash_policy,
            WasmModuleHashPolicy::Enforce
        );
        assert_eq!(
            parsed.runtime.wasm.security.module_sha256.get("calc"),
            Some(&"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
        );
    }

    #[test]
    async fn runtime_wasm_dev_template_deserializes() {
        let raw = include_str!("../../dev/config.wasm.dev.toml");
        let parsed: Config = toml::from_str(raw).expect("dev wasm template should parse");

        assert_eq!(parsed.runtime.kind, "wasm");
        assert!(parsed.runtime.wasm.allow_workspace_read);
        assert!(parsed.runtime.wasm.allow_workspace_write);
        assert_eq!(
            parsed.runtime.wasm.security.capability_escalation_mode,
            WasmCapabilityEscalationMode::Clamp
        );
    }

    #[test]
    async fn runtime_wasm_staging_template_deserializes() {
        let raw = include_str!("../../dev/config.wasm.staging.toml");
        let parsed: Config = toml::from_str(raw).expect("staging wasm template should parse");

        assert_eq!(parsed.runtime.kind, "wasm");
        assert!(parsed.runtime.wasm.allow_workspace_read);
        assert!(!parsed.runtime.wasm.allow_workspace_write);
        assert_eq!(
            parsed.runtime.wasm.security.capability_escalation_mode,
            WasmCapabilityEscalationMode::Deny
        );
    }

    #[test]
    async fn runtime_wasm_prod_template_deserializes() {
        let raw = include_str!("../../dev/config.wasm.prod.toml");
        let parsed: Config = toml::from_str(raw).expect("prod wasm template should parse");

        assert_eq!(parsed.runtime.kind, "wasm");
        assert!(!parsed.runtime.wasm.allow_workspace_read);
        assert!(!parsed.runtime.wasm.allow_workspace_write);
        assert!(parsed.runtime.wasm.allowed_hosts.is_empty());
        assert_eq!(
            parsed.runtime.wasm.security.capability_escalation_mode,
            WasmCapabilityEscalationMode::Deny
        );
    }

    #[test]
    async fn model_support_vision_deserializes() {
        let raw = r#"
default_temperature = 0.7
model_support_vision = true
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.model_support_vision, Some(true));

        // Default (omitted) should be None
        let raw_no_vision = r#"
default_temperature = 0.7
"#;
        let parsed2: Config = toml::from_str(raw_no_vision).unwrap();
        assert_eq!(parsed2.model_support_vision, None);
    }

    #[test]
    async fn provider_reasoning_level_deserializes() {
        let raw = r#"
default_temperature = 0.7

[provider]
reasoning_level = "high"
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.provider.reasoning_level.as_deref(), Some("high"));
        assert_eq!(
            parsed.effective_provider_reasoning_level().as_deref(),
            Some("high")
        );
    }

    #[test]
    async fn agent_config_defaults() {
        let cfg = AgentConfig::default();
        assert!(!cfg.compact_context);
        assert_eq!(cfg.max_tool_iterations, 20);
        assert_eq!(cfg.max_history_messages, 50);
        assert!(!cfg.parallel_tools);
        assert_eq!(cfg.tool_dispatcher, "auto");
    }

    #[test]
    async fn agent_config_deserializes() {
        let raw = r#"
default_temperature = 0.7
[agent]
compact_context = true
max_tool_iterations = 20
max_history_messages = 80
parallel_tools = true
tool_dispatcher = "xml"
"#;
        let parsed: Config = toml::from_str(raw).unwrap();
        assert!(parsed.agent.compact_context);
        assert_eq!(parsed.agent.max_tool_iterations, 20);
        assert_eq!(parsed.agent.max_history_messages, 80);
        assert!(parsed.agent.parallel_tools);
        assert_eq!(parsed.agent.tool_dispatcher, "xml");
    }

    #[tokio::test]
    async fn sync_directory_handles_existing_directory() {
        let dir = std::env::temp_dir().join(format!(
            "topclaw_test_sync_directory_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        sync_directory(&dir).await.unwrap();

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_and_load_tmpdir() {
        let dir = std::env::temp_dir().join("topclaw_test_config");
        let _ = fs::remove_dir_all(&dir).await;
        fs::create_dir_all(&dir).await.unwrap();

        let config_path = dir.join("config.toml");
        let config = Config {
            workspace_dir: dir.join("workspace"),
            config_path: config_path.clone(),
            api_key: Some("sk-roundtrip".into()),
            api_url: None,
            default_provider: Some("openrouter".into()),
            provider_api: None,
            default_model: Some("test-model".into()),
            model_providers: HashMap::new(),
            provider: ProviderConfig::default(),
            default_temperature: 0.9,
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            security: SecurityConfig::default(),
            runtime: RuntimeConfig::default(),
            research: ResearchPhaseConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            coordination: CoordinationConfig::default(),
            skills: SkillsConfig::default(),
            workspaces: WorkspacesConfig::default(),
            model_routes: Vec::new(),
            embedding_routes: Vec::new(),
            query_classification: QueryClassificationConfig::default(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            self_improvement: SelfImprovementConfig::default(),
            goal_loop: GoalLoopConfig::default(),
            channels_config: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            web_fetch: WebFetchConfig::default(),
            web_search: WebSearchConfig::default(),
            proxy: ProxyConfig::default(),
            agent: AgentConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            agents: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            transcription: TranscriptionConfig::default(),
            agents_ipc: AgentsIpcConfig::default(),
            model_support_vision: None,
        };

        config.save().await.unwrap();
        assert!(config_path.exists());

        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        let loaded: Config = toml::from_str(&contents).unwrap();
        assert!(loaded
            .api_key
            .as_deref()
            .is_some_and(crate::security::SecretStore::is_encrypted));
        let store = crate::security::SecretStore::new(&dir, true);
        let decrypted = store.decrypt(loaded.api_key.as_deref().unwrap()).unwrap();
        assert_eq!(decrypted, "sk-roundtrip");
        assert_eq!(loaded.default_model.as_deref(), Some("test-model"));
        assert!((loaded.default_temperature - 0.9).abs() < f64::EPSILON);

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_encrypts_nested_credentials() {
        let dir = std::env::temp_dir().join(format!(
            "topclaw_test_nested_credentials_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        let mut config = Config::default();
        config.workspace_dir = dir.join("workspace");
        config.config_path = dir.join("config.toml");
        config.api_key = Some("root-credential".into());
        config.composio.api_key = Some("composio-credential".into());
        config.proxy.http_proxy = Some("http://user:pass@proxy.internal:8080".into());
        config.proxy.https_proxy = Some("https://user:pass@proxy.internal:8443".into());
        config.proxy.all_proxy = Some("socks5://user:pass@proxy.internal:1080".into());
        config.browser.computer_use.api_key = Some("browser-credential".into());
        config.web_search.brave_api_key = Some("brave-credential".into());
        config.storage.provider.config.db_url = Some("postgres://user:pw@host/db".into());
        config.reliability.api_keys = vec!["backup-credential".into()];
        config.reliability.fallback_api_keys.insert(
            "custom:https://api-a.example.com/v1".into(),
            "fallback-a-credential".into(),
        );
        config.gateway.paired_tokens = vec!["zc_0123456789abcdef".into()];
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "telegram-credential".into(),
            allowed_users: Vec::new(),
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        config.agents.insert(
            "worker".into(),
            DelegateAgentConfig {
                provider: "openrouter".into(),
                model: "model-test".into(),
                system_prompt: None,
                api_key: Some("agent-credential".into()),
                temperature: None,
                max_depth: 3,
                agentic: false,
                allowed_tools: Vec::new(),
                max_iterations: 10,
            },
        );

        config.save().await.unwrap();

        let contents = tokio::fs::read_to_string(config.config_path.clone())
            .await
            .unwrap();
        let stored: Config = toml::from_str(&contents).unwrap();
        let store = crate::security::SecretStore::new(&dir, true);

        let root_encrypted = stored.api_key.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(root_encrypted));
        assert_eq!(store.decrypt(root_encrypted).unwrap(), "root-credential");

        let composio_encrypted = stored.composio.api_key.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            composio_encrypted
        ));
        assert_eq!(
            store.decrypt(composio_encrypted).unwrap(),
            "composio-credential"
        );

        let proxy_http_encrypted = stored.proxy.http_proxy.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            proxy_http_encrypted
        ));
        assert_eq!(
            store.decrypt(proxy_http_encrypted).unwrap(),
            "http://user:pass@proxy.internal:8080"
        );
        let proxy_https_encrypted = stored.proxy.https_proxy.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            proxy_https_encrypted
        ));
        assert_eq!(
            store.decrypt(proxy_https_encrypted).unwrap(),
            "https://user:pass@proxy.internal:8443"
        );
        let proxy_all_encrypted = stored.proxy.all_proxy.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            proxy_all_encrypted
        ));
        assert_eq!(
            store.decrypt(proxy_all_encrypted).unwrap(),
            "socks5://user:pass@proxy.internal:1080"
        );

        let browser_encrypted = stored.browser.computer_use.api_key.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            browser_encrypted
        ));
        assert_eq!(
            store.decrypt(browser_encrypted).unwrap(),
            "browser-credential"
        );

        let web_search_encrypted = stored.web_search.brave_api_key.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(
            web_search_encrypted
        ));
        assert_eq!(
            store.decrypt(web_search_encrypted).unwrap(),
            "brave-credential"
        );

        let worker = stored.agents.get("worker").unwrap();
        let worker_encrypted = worker.api_key.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(worker_encrypted));
        assert_eq!(store.decrypt(worker_encrypted).unwrap(), "agent-credential");

        let storage_db_url = stored.storage.provider.config.db_url.as_deref().unwrap();
        assert!(crate::security::SecretStore::is_encrypted(storage_db_url));
        assert_eq!(
            store.decrypt(storage_db_url).unwrap(),
            "postgres://user:pw@host/db"
        );

        let reliability_key = &stored.reliability.api_keys[0];
        assert!(crate::security::SecretStore::is_encrypted(reliability_key));
        assert_eq!(store.decrypt(reliability_key).unwrap(), "backup-credential");
        let fallback_key = stored
            .reliability
            .fallback_api_keys
            .get("custom:https://api-a.example.com/v1")
            .expect("fallback key should exist");
        assert!(crate::security::SecretStore::is_encrypted(fallback_key));
        assert_eq!(
            store.decrypt(fallback_key).unwrap(),
            "fallback-a-credential"
        );

        let paired_token = &stored.gateway.paired_tokens[0];
        assert!(crate::security::SecretStore::is_encrypted(paired_token));
        assert_eq!(store.decrypt(paired_token).unwrap(), "zc_0123456789abcdef");

        let telegram_token = stored
            .channels_config
            .telegram
            .as_ref()
            .unwrap()
            .bot_token
            .clone();
        assert!(crate::security::SecretStore::is_encrypted(&telegram_token));
        assert_eq!(
            store.decrypt(&telegram_token).unwrap(),
            "telegram-credential"
        );

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_atomic_cleanup() {
        let dir =
            std::env::temp_dir().join(format!("topclaw_test_config_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();

        let config_path = dir.join("config.toml");
        let mut config = Config::default();
        config.workspace_dir = dir.join("workspace");
        config.config_path = config_path.clone();
        config.default_model = Some("model-a".into());
        config.save().await.unwrap();
        assert!(config_path.exists());

        config.default_model = Some("model-b".into());
        config.save().await.unwrap();

        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(contents.contains("model-b"));

        let names: Vec<String> = ReadDirStream::new(fs::read_dir(&dir).await.unwrap())
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect()
            .await;
        assert!(!names.iter().any(|name| name.contains(".tmp-")));
        assert!(!names.iter().any(|name| name.ends_with(".bak")));

        let _ = fs::remove_dir_all(&dir).await;
    }

    // ── Telegram / Discord config ────────────────────────────

    #[test]
    async fn telegram_config_serde() {
        let tc = TelegramConfig {
            bot_token: "123:XYZ".into(),
            allowed_users: vec!["alice".into(), "bob".into()],
            stream_mode: StreamMode::Partial,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: true,
            group_reply: None,
            base_url: None,
        };
        let json = serde_json::to_string(&tc).unwrap();
        let parsed: TelegramConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bot_token, "123:XYZ");
        assert_eq!(parsed.allowed_users.len(), 2);
        assert_eq!(parsed.stream_mode, StreamMode::Partial);
        assert_eq!(parsed.draft_update_interval_ms, 500);
        assert!(parsed.interrupt_on_new_message);
    }

    #[test]
    async fn telegram_allowed_users_env_ref_expands_comma_list() {
        let env_name = "TOPCLAW_TEST_TELEGRAM_ALLOWED_USERS_CSV";
        std::env::set_var(env_name, "1001, 1002, *");

        let mut channels = ChannelsConfig::default();
        channels.telegram = Some(TelegramConfig {
            bot_token: "123:XYZ".into(),
            allowed_users: vec![format!("${{env:{env_name}}}")],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        let result = resolve_telegram_allowed_users_env_refs(&mut channels);
        std::env::remove_var(env_name);
        result.expect("env reference should expand");

        let telegram = channels.telegram.expect("telegram config should exist");
        assert_eq!(telegram.allowed_users, vec!["1001", "1002", "*"]);
    }

    #[test]
    async fn telegram_allowed_users_env_ref_expands_json_array() {
        let env_name = "TOPCLAW_TEST_TELEGRAM_ALLOWED_USERS_JSON";
        std::env::set_var(env_name, r#"["1001", 1002, "*"]"#);

        let mut channels = ChannelsConfig::default();
        channels.telegram = Some(TelegramConfig {
            bot_token: "123:XYZ".into(),
            allowed_users: vec![format!("${{env:{env_name}}}")],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        let result = resolve_telegram_allowed_users_env_refs(&mut channels);
        std::env::remove_var(env_name);
        result.expect("JSON env reference should expand");

        let telegram = channels.telegram.expect("telegram config should exist");
        assert_eq!(telegram.allowed_users, vec!["1001", "1002", "*"]);
    }

    #[test]
    async fn telegram_allowed_users_env_ref_missing_var_fails() {
        let env_name = "TOPCLAW_TEST_TELEGRAM_ALLOWED_USERS_MISSING";
        std::env::remove_var(env_name);

        let mut channels = ChannelsConfig::default();
        channels.telegram = Some(TelegramConfig {
            bot_token: "123:XYZ".into(),
            allowed_users: vec![format!("${{env:{env_name}}}")],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        let err = resolve_telegram_allowed_users_env_refs(&mut channels)
            .expect_err("unset env var should fail");
        let message = err.to_string();
        assert!(message.contains("allowed_users"));
        assert!(message.contains(env_name));
    }

    #[test]
    async fn telegram_allowed_users_env_ref_invalid_env_name_fails() {
        let mut channels = ChannelsConfig::default();
        channels.telegram = Some(TelegramConfig {
            bot_token: "123:XYZ".into(),
            allowed_users: vec!["${env:NOT VALID}".to_string()],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        let err = resolve_telegram_allowed_users_env_refs(&mut channels)
            .expect_err("invalid env var name should fail");
        assert!(err.to_string().contains("invalid env var name"));
    }

    #[test]
    async fn telegram_config_defaults_stream_partial() {
        let json = r#"{"bot_token":"tok","allowed_users":[]}"#;
        let parsed: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.stream_mode, StreamMode::Partial);
        assert_eq!(parsed.draft_update_interval_ms, 500);
        assert!(!parsed.interrupt_on_new_message);
        assert!(parsed.base_url.is_none());
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
        assert!(parsed.group_reply_allowed_sender_ids().is_empty());
    }

    #[test]
    async fn telegram_config_custom_base_url() {
        let json = r#"{"bot_token":"tok","allowed_users":[],"base_url":"https://tapi.bale.ai"}"#;
        let parsed: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.base_url, Some("https://tapi.bale.ai".to_string()));
    }

    #[test]
    async fn telegram_group_reply_config_supports_explicit_mode() {
        let json = r#"{
            "bot_token":"tok",
            "allowed_users":["*"],
            "group_reply":{
                "mode":"mention_only",
                "allowed_sender_ids":["1001","1002"]
            }
        }"#;

        let parsed: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::MentionOnly
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["1001".to_string(), "1002".to_string()]
        );
    }

    #[test]
    async fn discord_config_serde() {
        let dc = DiscordConfig {
            bot_token: "discord-token".into(),
            guild_id: Some("12345".into()),
            allowed_users: vec![],
            listen_to_bots: false,
            group_reply: None,
        };
        let json = serde_json::to_string(&dc).unwrap();
        let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bot_token, "discord-token");
        assert_eq!(parsed.guild_id.as_deref(), Some("12345"));
    }

    #[test]
    async fn discord_config_optional_guild() {
        let dc = DiscordConfig {
            bot_token: "tok".into(),
            guild_id: None,
            allowed_users: vec![],
            listen_to_bots: false,
            group_reply: None,
        };
        let json = serde_json::to_string(&dc).unwrap();
        let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.guild_id.is_none());
    }

    #[test]
    async fn discord_group_reply_mode_defaults_to_all_messages() {
        let json = r#"{"bot_token":"tok"}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
        assert!(parsed.group_reply_allowed_sender_ids().is_empty());
    }

    #[test]
    async fn discord_group_reply_mode_supports_explicit_override() {
        let json = r#"{
            "bot_token":"tok",
            "group_reply":{
                "mode":"all_messages",
                "allowed_sender_ids":["111"]
            }
        }"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["111".to_string()]
        );
    }

    // ── iMessage / Matrix config ────────────────────────────

    #[test]
    async fn imessage_config_serde() {
        let ic = IMessageConfig {
            allowed_contacts: vec!["+1234567890".into(), "user@icloud.com".into()],
        };
        let json = serde_json::to_string(&ic).unwrap();
        let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.allowed_contacts.len(), 2);
        assert_eq!(parsed.allowed_contacts[0], "+1234567890");
    }

    #[test]
    async fn imessage_config_empty_contacts() {
        let ic = IMessageConfig {
            allowed_contacts: vec![],
        };
        let json = serde_json::to_string(&ic).unwrap();
        let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.allowed_contacts.is_empty());
    }

    #[test]
    async fn imessage_config_wildcard() {
        let ic = IMessageConfig {
            allowed_contacts: vec!["*".into()],
        };
        let toml_str = toml::to_string(&ic).unwrap();
        let parsed: IMessageConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.allowed_contacts, vec!["*"]);
    }

    #[test]
    async fn matrix_config_serde() {
        let mc = MatrixConfig {
            homeserver: "https://matrix.org".into(),
            access_token: "syt_token_abc".into(),
            user_id: Some("@bot:matrix.org".into()),
            device_id: Some("DEVICE123".into()),
            room_id: "!room123:matrix.org".into(),
            allowed_users: vec!["@user:matrix.org".into()],
            mention_only: false,
        };
        let json = serde_json::to_string(&mc).unwrap();
        let parsed: MatrixConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.homeserver, "https://matrix.org");
        assert_eq!(parsed.access_token, "syt_token_abc");
        assert_eq!(parsed.user_id.as_deref(), Some("@bot:matrix.org"));
        assert_eq!(parsed.device_id.as_deref(), Some("DEVICE123"));
        assert_eq!(parsed.room_id, "!room123:matrix.org");
        assert_eq!(parsed.allowed_users.len(), 1);
    }

    #[test]
    async fn matrix_config_toml_roundtrip() {
        let mc = MatrixConfig {
            homeserver: "https://synapse.local:8448".into(),
            access_token: "tok".into(),
            user_id: None,
            device_id: None,
            room_id: "!abc:synapse.local".into(),
            allowed_users: vec!["@admin:synapse.local".into(), "*".into()],
            mention_only: true,
        };
        let toml_str = toml::to_string(&mc).unwrap();
        let parsed: MatrixConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.homeserver, "https://synapse.local:8448");
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    async fn matrix_config_backward_compatible_without_session_hints() {
        let toml = r#"
homeserver = "https://matrix.org"
access_token = "tok"
room_id = "!ops:matrix.org"
allowed_users = ["@ops:matrix.org"]
"#;

        let parsed: MatrixConfig = toml::from_str(toml).unwrap();
        assert_eq!(parsed.homeserver, "https://matrix.org");
        assert!(parsed.user_id.is_none());
        assert!(parsed.device_id.is_none());
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn signal_config_serde() {
        let sc = SignalConfig {
            http_url: "http://127.0.0.1:8686".into(),
            account: "+1234567890".into(),
            group_id: Some("group123".into()),
            allowed_from: vec!["+1111111111".into()],
            ignore_attachments: true,
            ignore_stories: false,
        };
        let json = serde_json::to_string(&sc).unwrap();
        let parsed: SignalConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.http_url, "http://127.0.0.1:8686");
        assert_eq!(parsed.account, "+1234567890");
        assert_eq!(parsed.group_id.as_deref(), Some("group123"));
        assert_eq!(parsed.allowed_from.len(), 1);
        assert!(parsed.ignore_attachments);
        assert!(!parsed.ignore_stories);
    }

    #[test]
    async fn signal_config_toml_roundtrip() {
        let sc = SignalConfig {
            http_url: "http://localhost:8080".into(),
            account: "+9876543210".into(),
            group_id: None,
            allowed_from: vec!["*".into()],
            ignore_attachments: false,
            ignore_stories: true,
        };
        let toml_str = toml::to_string(&sc).unwrap();
        let parsed: SignalConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.http_url, "http://localhost:8080");
        assert_eq!(parsed.account, "+9876543210");
        assert!(parsed.group_id.is_none());
        assert!(parsed.ignore_stories);
    }

    #[test]
    async fn signal_config_defaults() {
        let json = r#"{"http_url":"http://127.0.0.1:8686","account":"+1234567890"}"#;
        let parsed: SignalConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.group_id.is_none());
        assert!(parsed.allowed_from.is_empty());
        assert!(!parsed.ignore_attachments);
        assert!(!parsed.ignore_stories);
    }

    #[test]
    async fn channels_config_with_imessage_and_matrix() {
        let c = ChannelsConfig {
            cli: true,
            bridge: None,
            telegram: None,
            discord: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: Some(IMessageConfig {
                allowed_contacts: vec!["+1".into()],
            }),
            matrix: Some(MatrixConfig {
                homeserver: "https://m.org".into(),
                access_token: "tok".into(),
                user_id: None,
                device_id: None,
                room_id: "!r:m".into(),
                allowed_users: vec!["@u:m".into()],
                mention_only: false,
            }),
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            irc: None,
            lark: None,
            feishu: None,
            dingtalk: None,
            qq: None,
            nostr: None,
            clawdtalk: None,
            message_timeout_secs: 300,
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.imessage.is_some());
        assert!(parsed.matrix.is_some());
        assert_eq!(parsed.imessage.unwrap().allowed_contacts, vec!["+1"]);
        assert_eq!(parsed.matrix.unwrap().homeserver, "https://m.org");
    }

    #[test]
    async fn channels_config_default_has_no_imessage_matrix() {
        let c = ChannelsConfig::default();
        assert!(c.imessage.is_none());
        assert!(c.matrix.is_none());
    }

    #[test]
    async fn bridge_config_deserializes_with_safe_defaults() {
        let parsed: BridgeConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.bind_host, "127.0.0.1");
        assert_eq!(parsed.bind_port, 8765);
        assert_eq!(parsed.path, "/ws");
        assert!(parsed.auth_token.is_empty());
        assert!(parsed.allowed_senders.is_empty());
        assert!(!parsed.allow_public_bind);
        assert_eq!(parsed.max_connections, 64);
    }

    #[test]
    async fn channels_config_supports_bridge_section() {
        let toml_str = r#"
cli = true

[bridge]
bind_host = "127.0.0.1"
bind_port = 9010
path = "/bridge"
auth_token = "test-token"
allowed_senders = ["sender_a", "sender_b"]
allow_public_bind = false
max_connections = 16
"#;
        let parsed: ChannelsConfig = toml::from_str(toml_str).unwrap();
        let bridge = parsed.bridge.expect("bridge should be present");
        assert_eq!(bridge.bind_host, "127.0.0.1");
        assert_eq!(bridge.bind_port, 9010);
        assert_eq!(bridge.path, "/bridge");
        assert_eq!(bridge.auth_token, "test-token");
        assert_eq!(bridge.allowed_senders, vec!["sender_a", "sender_b"]);
        assert!(!bridge.allow_public_bind);
        assert_eq!(bridge.max_connections, 16);
    }

    // ── Edge cases: serde(default) for allowed_users ─────────

    #[test]
    async fn discord_config_deserializes_without_allowed_users() {
        // Old configs won't have allowed_users — serde(default) should fill vec![]
        let json = r#"{"bot_token":"tok","guild_id":"123"}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.allowed_users.is_empty());
    }

    #[test]
    async fn discord_config_deserializes_with_allowed_users() {
        let json = r#"{"bot_token":"tok","guild_id":"123","allowed_users":["111","222"]}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.allowed_users, vec!["111", "222"]);
    }

    #[test]
    async fn slack_config_deserializes_without_allowed_users() {
        let json = r#"{"bot_token":"xoxb-tok"}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
    }

    #[test]
    async fn slack_config_deserializes_with_allowed_users() {
        let json = r#"{"bot_token":"xoxb-tok","allowed_users":["U111"]}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.allowed_users, vec!["U111"]);
    }

    #[test]
    async fn discord_config_toml_backward_compat() {
        let toml_str = r#"
bot_token = "tok"
guild_id = "123"
"#;
        let parsed: DiscordConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.bot_token, "tok");
    }

    #[test]
    async fn slack_config_toml_backward_compat() {
        let toml_str = r#"
bot_token = "xoxb-tok"
channel_id = "C123"
"#;
        let parsed: SlackConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.channel_id.as_deref(), Some("C123"));
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
    }

    #[test]
    async fn slack_group_reply_config_supports_sender_overrides() {
        let json = r#"{
            "bot_token":"xoxb-tok",
            "group_reply":{
                "mode":"mention_only",
                "allowed_sender_ids":["U111"]
            }
        }"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::MentionOnly
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["U111".to_string()]
        );
    }

    #[test]
    async fn mattermost_group_reply_mode_defaults_to_all_messages() {
        let json = r#"{"url":"https://mm.example.com","bot_token":"token"}"#;
        let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
    }

    #[test]
    async fn mattermost_group_reply_mode_supports_explicit_override() {
        let json = r#"{
            "url":"https://mm.example.com",
            "bot_token":"token",
            "group_reply":{
                "mode":"all_messages",
                "allowed_sender_ids":["u1","u2"]
            }
        }"#;
        let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["u1".to_string(), "u2".to_string()]
        );
    }

    #[test]
    async fn webhook_config_with_secret() {
        let json = r#"{"port":8080,"secret":"my-secret-key"}"#;
        let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.secret.as_deref(), Some("my-secret-key"));
    }

    #[test]
    async fn webhook_config_without_secret() {
        let json = r#"{"port":8080}"#;
        let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.secret.is_none());
        assert_eq!(parsed.port, 8080);
    }

    // ── WhatsApp config ──────────────────────────────────────

    #[test]
    async fn whatsapp_config_serde() {
        let wc = WhatsAppConfig {
            access_token: Some("EAABx...".into()),
            phone_number_id: Some("123456789".into()),
            verify_token: Some("my-verify-token".into()),
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1234567890".into(), "+9876543210".into()],
        };
        let json = serde_json::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, Some("EAABx...".into()));
        assert_eq!(parsed.phone_number_id, Some("123456789".into()));
        assert_eq!(parsed.verify_token, Some("my-verify-token".into()));
        assert_eq!(parsed.allowed_numbers.len(), 2);
    }

    #[test]
    async fn whatsapp_config_toml_roundtrip() {
        let wc = WhatsAppConfig {
            access_token: Some("tok".into()),
            phone_number_id: Some("12345".into()),
            verify_token: Some("verify".into()),
            app_secret: Some("secret123".into()),
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1".into()],
        };
        let toml_str = toml::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.phone_number_id, Some("12345".into()));
        assert_eq!(parsed.allowed_numbers, vec!["+1"]);
    }

    #[test]
    async fn whatsapp_config_deserializes_without_allowed_numbers() {
        let json = r#"{"access_token":"tok","phone_number_id":"123","verify_token":"ver"}"#;
        let parsed: WhatsAppConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.allowed_numbers.is_empty());
    }

    #[test]
    async fn whatsapp_config_wildcard_allowed() {
        let wc = WhatsAppConfig {
            access_token: Some("tok".into()),
            phone_number_id: Some("123".into()),
            verify_token: Some("ver".into()),
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["*".into()],
        };
        let toml_str = toml::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.allowed_numbers, vec!["*"]);
    }

    #[test]
    async fn whatsapp_config_backend_type_cloud_precedence_when_ambiguous() {
        let wc = WhatsAppConfig {
            access_token: Some("tok".into()),
            phone_number_id: Some("123".into()),
            verify_token: Some("ver".into()),
            app_secret: None,
            session_path: Some("~/.topclaw/state/whatsapp-web/session.db".into()),
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1".into()],
        };
        assert!(wc.is_ambiguous_config());
        assert_eq!(wc.backend_type(), "cloud");
    }

    #[test]
    async fn whatsapp_config_backend_type_web() {
        let wc = WhatsAppConfig {
            access_token: None,
            phone_number_id: None,
            verify_token: None,
            app_secret: None,
            session_path: Some("~/.topclaw/state/whatsapp-web/session.db".into()),
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec![],
        };
        assert!(!wc.is_ambiguous_config());
        assert_eq!(wc.backend_type(), "web");
    }

    #[test]
    async fn channels_config_with_whatsapp() {
        let c = ChannelsConfig {
            cli: true,
            bridge: None,
            telegram: None,
            discord: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: Some(WhatsAppConfig {
                access_token: Some("tok".into()),
                phone_number_id: Some("123".into()),
                verify_token: Some("ver".into()),
                app_secret: None,
                session_path: None,
                pair_phone: None,
                pair_code: None,
                allowed_numbers: vec!["+1".into()],
            }),
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            irc: None,
            lark: None,
            feishu: None,
            dingtalk: None,
            qq: None,
            nostr: None,
            clawdtalk: None,
            message_timeout_secs: 300,
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.whatsapp.is_some());
        let wa = parsed.whatsapp.unwrap();
        assert_eq!(wa.phone_number_id, Some("123".into()));
        assert_eq!(wa.allowed_numbers, vec!["+1"]);
    }

    #[test]
    async fn channels_config_default_has_no_whatsapp() {
        let c = ChannelsConfig::default();
        assert!(c.whatsapp.is_none());
    }

    #[test]
    async fn channels_config_default_has_no_nextcloud_talk() {
        let c = ChannelsConfig::default();
        assert!(c.nextcloud_talk.is_none());
    }

    // ══════════════════════════════════════════════════════════
    // SECURITY CHECKLIST TESTS — Gateway config
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn checklist_gateway_default_requires_pairing() {
        let g = GatewayConfig::default();
        assert!(g.require_pairing, "Pairing must be required by default");
    }

    #[test]
    async fn checklist_gateway_default_blocks_public_bind() {
        let g = GatewayConfig::default();
        assert!(
            !g.allow_public_bind,
            "Public bind must be blocked by default"
        );
    }

    #[test]
    async fn checklist_gateway_default_no_tokens() {
        let g = GatewayConfig::default();
        assert!(
            g.paired_tokens.is_empty(),
            "No pre-paired tokens by default"
        );
        assert_eq!(g.pair_rate_limit_per_minute, 10);
        assert_eq!(g.webhook_rate_limit_per_minute, 60);
        assert!(!g.trust_forwarded_headers);
        assert!(g.trusted_proxy_cidrs.is_empty());
        assert_eq!(g.rate_limit_max_keys, 10_000);
        assert_eq!(g.idempotency_ttl_secs, 300);
        assert_eq!(g.idempotency_max_keys, 10_000);
        assert!(!g.node_control.enabled);
        assert!(g.node_control.auth_token.is_none());
        assert!(g.node_control.allowed_node_ids.is_empty());
    }

    #[test]
    async fn checklist_gateway_cli_default_host_is_localhost() {
        // The CLI default for --host is 127.0.0.1 (checked in main.rs)
        // Here we verify the config default matches
        let c = Config::default();
        assert!(
            c.gateway.require_pairing,
            "Config default must require pairing"
        );
        assert!(
            !c.gateway.allow_public_bind,
            "Config default must block public bind"
        );
    }

    #[test]
    async fn checklist_gateway_serde_roundtrip() {
        let g = GatewayConfig {
            port: 42617,
            host: "127.0.0.1".into(),
            require_pairing: true,
            allow_public_bind: false,
            paired_tokens: vec!["zc_test_token".into()],
            pair_rate_limit_per_minute: 12,
            webhook_rate_limit_per_minute: 80,
            trust_forwarded_headers: true,
            trusted_proxy_cidrs: vec!["10.0.0.0/8".into(), "192.168.0.0/16".into()],
            rate_limit_max_keys: 2048,
            idempotency_ttl_secs: 600,
            idempotency_max_keys: 4096,
            node_control: NodeControlConfig {
                enabled: true,
                auth_token: Some("node-token".into()),
                allowed_node_ids: vec!["node-1".into(), "node-2".into()],
            },
        };
        let toml_str = toml::to_string(&g).unwrap();
        let parsed: GatewayConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.require_pairing);
        assert!(!parsed.allow_public_bind);
        assert_eq!(parsed.paired_tokens, vec!["zc_test_token"]);
        assert_eq!(parsed.pair_rate_limit_per_minute, 12);
        assert_eq!(parsed.webhook_rate_limit_per_minute, 80);
        assert!(parsed.trust_forwarded_headers);
        assert_eq!(
            parsed.trusted_proxy_cidrs,
            vec!["10.0.0.0/8", "192.168.0.0/16"]
        );
        assert_eq!(parsed.rate_limit_max_keys, 2048);
        assert_eq!(parsed.idempotency_ttl_secs, 600);
        assert_eq!(parsed.idempotency_max_keys, 4096);
        assert!(parsed.node_control.enabled);
        assert_eq!(
            parsed.node_control.auth_token.as_deref(),
            Some("node-token")
        );
        assert_eq!(
            parsed.node_control.allowed_node_ids,
            vec!["node-1", "node-2"]
        );
    }

    #[test]
    async fn checklist_gateway_backward_compat_no_gateway_section() {
        // Old configs without [gateway] should get secure defaults
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert!(
            parsed.gateway.require_pairing,
            "Missing [gateway] must default to require_pairing=true"
        );
        assert!(
            !parsed.gateway.allow_public_bind,
            "Missing [gateway] must default to allow_public_bind=false"
        );
    }

    #[test]
    async fn checklist_gateway_backward_compat_accepts_legacy_gateway_table_alias() {
        let mut raw: toml::Value = toml::from_str(
            r#"
default_temperature = 0.7
[Gateway]
require_pairing = false
"#,
        )
        .unwrap();

        normalize_top_level_table_aliases(&mut raw);
        let parsed: Config = raw.try_into().unwrap();
        assert!(
            !parsed.gateway.require_pairing,
            "Legacy [Gateway] alias should map to [gateway]"
        );
    }

    #[test]
    async fn checklist_autonomy_default_is_workspace_scoped() {
        let a = AutonomyConfig::default();
        assert!(a.workspace_only, "Default autonomy must be workspace_only");
        assert!(
            a.forbidden_paths.contains(&"/etc".to_string()),
            "Must block /etc"
        );
        assert!(
            a.forbidden_paths.contains(&"/proc".to_string()),
            "Must block /proc"
        );
        assert!(
            a.forbidden_paths.contains(&"~/.ssh".to_string()),
            "Must block ~/.ssh"
        );
    }

    // ══════════════════════════════════════════════════════════
    // COMPOSIO CONFIG TESTS
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn composio_config_default_disabled() {
        let c = ComposioConfig::default();
        assert!(!c.enabled, "Composio must be disabled by default");
        assert!(c.api_key.is_none(), "No API key by default");
        assert_eq!(c.entity_id, "default");
    }

    #[test]
    async fn composio_config_serde_roundtrip() {
        let c = ComposioConfig {
            enabled: true,
            api_key: Some("comp-key-123".into()),
            entity_id: "user42".into(),
        };
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: ComposioConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.api_key.as_deref(), Some("comp-key-123"));
        assert_eq!(parsed.entity_id, "user42");
    }

    #[test]
    async fn composio_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert!(
            !parsed.composio.enabled,
            "Missing [composio] must default to disabled"
        );
        assert!(parsed.composio.api_key.is_none());
    }

    #[test]
    async fn composio_config_partial_toml() {
        let toml_str = r"
enabled = true
";
        let parsed: ComposioConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.api_key.is_none());
        assert_eq!(parsed.entity_id, "default");
    }

    // ══════════════════════════════════════════════════════════
    // SECRETS CONFIG TESTS
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn secrets_config_default_encrypts() {
        let s = SecretsConfig::default();
        assert!(s.encrypt, "Encryption must be enabled by default");
    }

    #[test]
    async fn secrets_config_serde_roundtrip() {
        let s = SecretsConfig { encrypt: false };
        let toml_str = toml::to_string(&s).unwrap();
        let parsed: SecretsConfig = toml::from_str(&toml_str).unwrap();
        assert!(!parsed.encrypt);
    }

    #[test]
    async fn secrets_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert!(
            parsed.secrets.encrypt,
            "Missing [secrets] must default to encrypt=true"
        );
    }

    #[test]
    async fn config_default_has_composio_and_secrets() {
        let c = Config::default();
        assert!(!c.composio.enabled);
        assert!(c.composio.api_key.is_none());
        assert!(c.secrets.encrypt);
        assert!(!c.browser.enabled);
        assert!(c.browser.allowed_domains.is_empty());
    }

    #[test]
    async fn browser_config_default_disabled() {
        let b = BrowserConfig::default();
        assert!(!b.enabled);
        assert!(b.allowed_domains.is_empty());
        assert_eq!(b.backend, "agent_browser");
        assert!(b.native_headless);
        assert_eq!(b.native_webdriver_url, "http://127.0.0.1:9515");
        assert!(b.native_chrome_path.is_none());
        assert_eq!(b.computer_use.endpoint, "http://127.0.0.1:8787/v1/actions");
        assert_eq!(b.computer_use.timeout_ms, 15_000);
        assert!(!b.computer_use.allow_remote_endpoint);
        assert!(b.computer_use.window_allowlist.is_empty());
        assert!(b.computer_use.max_coordinate_x.is_none());
        assert!(b.computer_use.max_coordinate_y.is_none());
    }

    #[test]
    async fn browser_config_serde_roundtrip() {
        let b = BrowserConfig {
            enabled: true,
            allowed_domains: vec!["example.com".into(), "docs.example.com".into()],
            browser_open: "chrome".into(),
            session_name: None,
            backend: "auto".into(),
            native_headless: false,
            native_webdriver_url: "http://localhost:4444".into(),
            native_chrome_path: Some("/usr/bin/chromium".into()),
            computer_use: BrowserComputerUseConfig {
                endpoint: "https://computer-use.example.com/v1/actions".into(),
                api_key: Some("test-token".into()),
                timeout_ms: 8_000,
                allow_remote_endpoint: true,
                window_allowlist: vec!["Chrome".into(), "Visual Studio Code".into()],
                max_coordinate_x: Some(3840),
                max_coordinate_y: Some(2160),
            },
        };
        let toml_str = toml::to_string(&b).unwrap();
        let parsed: BrowserConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.allowed_domains.len(), 2);
        assert_eq!(parsed.allowed_domains[0], "example.com");
        assert_eq!(parsed.backend, "auto");
        assert!(!parsed.native_headless);
        assert_eq!(parsed.native_webdriver_url, "http://localhost:4444");
        assert_eq!(
            parsed.native_chrome_path.as_deref(),
            Some("/usr/bin/chromium")
        );
        assert_eq!(
            parsed.computer_use.endpoint,
            "https://computer-use.example.com/v1/actions"
        );
        assert_eq!(parsed.computer_use.api_key.as_deref(), Some("test-token"));
        assert_eq!(parsed.computer_use.timeout_ms, 8_000);
        assert!(parsed.computer_use.allow_remote_endpoint);
        assert_eq!(parsed.computer_use.window_allowlist.len(), 2);
        assert_eq!(parsed.computer_use.max_coordinate_x, Some(3840));
        assert_eq!(parsed.computer_use.max_coordinate_y, Some(2160));
    }

    #[test]
    async fn browser_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert!(!parsed.browser.enabled);
        assert!(parsed.browser.allowed_domains.is_empty());
    }

    // ── Environment variable overrides (Docker support) ─────────

    async fn env_override_lock() -> MutexGuard<'static, ()> {
        static ENV_OVERRIDE_TEST_LOCK: Mutex<()> = Mutex::const_new(());
        ENV_OVERRIDE_TEST_LOCK.lock().await
    }

    fn clear_proxy_env_test_vars() {
        for key in [
            "TOPCLAW_PROXY_ENABLED",
            "TOPCLAW_HTTP_PROXY",
            "TOPCLAW_HTTPS_PROXY",
            "TOPCLAW_ALL_PROXY",
            "TOPCLAW_NO_PROXY",
            "TOPCLAW_PROXY_SCOPE",
            "TOPCLAW_PROXY_SERVICES",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    async fn env_override_api_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert!(config.api_key.is_none());

        std::env::set_var("TOPCLAW_API_KEY", "sk-test-env-key");
        config.apply_env_overrides();
        assert_eq!(config.api_key.as_deref(), Some("sk-test-env-key"));

        std::env::remove_var("TOPCLAW_API_KEY");
    }

    #[test]
    async fn env_override_api_key_ignores_generic_alias() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::remove_var("TOPCLAW_API_KEY");
        std::env::set_var("API_KEY", "sk-fallback-key");
        config.apply_env_overrides();
        assert!(config.api_key.is_none());

        std::env::remove_var("API_KEY");
    }

    #[test]
    async fn env_override_provider() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_PROVIDER", "anthropic");
        config.apply_env_overrides();
        assert_eq!(config.default_provider.as_deref(), Some("anthropic"));

        std::env::remove_var("TOPCLAW_PROVIDER");
    }

    #[test]
    async fn env_override_open_skills_enabled_and_dir() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert!(!config.skills.open_skills_enabled);
        assert!(config.skills.open_skills_dir.is_none());
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );

        std::env::set_var("TOPCLAW_OPEN_SKILLS_ENABLED", "true");
        std::env::set_var("TOPCLAW_OPEN_SKILLS_DIR", "/tmp/open-skills");
        std::env::set_var("TOPCLAW_SKILLS_PROMPT_MODE", "compact");
        config.apply_env_overrides();

        assert!(config.skills.open_skills_enabled);
        assert_eq!(
            config.skills.open_skills_dir.as_deref(),
            Some("/tmp/open-skills")
        );
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );

        std::env::remove_var("TOPCLAW_OPEN_SKILLS_ENABLED");
        std::env::remove_var("TOPCLAW_OPEN_SKILLS_DIR");
        std::env::remove_var("TOPCLAW_SKILLS_PROMPT_MODE");
    }

    #[test]
    async fn env_override_open_skills_enabled_invalid_value_keeps_existing_value() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.skills.open_skills_enabled = true;
        config.skills.prompt_injection_mode = SkillsPromptInjectionMode::Compact;

        std::env::set_var("TOPCLAW_OPEN_SKILLS_ENABLED", "maybe");
        std::env::set_var("TOPCLAW_SKILLS_PROMPT_MODE", "invalid");
        config.apply_env_overrides();

        assert!(config.skills.open_skills_enabled);
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );
        std::env::remove_var("TOPCLAW_OPEN_SKILLS_ENABLED");
        std::env::remove_var("TOPCLAW_SKILLS_PROMPT_MODE");
    }

    #[test]
    async fn provider_api_requires_custom_default_provider() {
        let mut config = Config::default();
        config.default_provider = Some("openai".to_string());
        config.provider_api = Some(ProviderApiMode::OpenAiResponses);

        let err = config
            .validate()
            .expect_err("provider_api should be rejected for non-custom provider");
        assert!(err.to_string().contains(
            "provider_api is only valid when default_provider uses the custom:<url> format"
        ));
    }

    #[test]
    async fn provider_api_invalid_value_is_rejected() {
        let toml = r#"
default_provider = "custom:https://example.com/v1"
default_model = "gpt-4o"
default_temperature = 0.7
provider_api = "not-a-real-mode"
"#;
        let parsed = toml::from_str::<Config>(toml);
        assert!(
            parsed.is_err(),
            "invalid provider_api should fail to deserialize"
        );
    }

    #[test]
    async fn model_route_max_tokens_must_be_positive_when_set() {
        let mut config = Config::default();
        config.model_routes = vec![ModelRouteConfig {
            hint: "reasoning".to_string(),
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4.6".to_string(),
            max_tokens: Some(0),
            api_key: None,
        }];

        let err = config
            .validate()
            .expect_err("model route max_tokens=0 should be rejected");
        assert!(err
            .to_string()
            .contains("model_routes[0].max_tokens must be greater than 0"));
    }

    #[test]
    async fn env_override_glm_api_key_for_regional_aliases() {
        let _env_guard = env_override_lock().await;
        let mut config = Config {
            default_provider: Some("glm-cn".to_string()),
            ..Config::default()
        };

        std::env::set_var("GLM_API_KEY", "glm-regional-key");
        config.apply_env_overrides();
        assert_eq!(config.api_key.as_deref(), Some("glm-regional-key"));

        std::env::remove_var("GLM_API_KEY");
    }

    #[test]
    async fn env_override_zai_api_key_for_regional_aliases() {
        let _env_guard = env_override_lock().await;
        let mut config = Config {
            default_provider: Some("zai-cn".to_string()),
            ..Config::default()
        };

        std::env::set_var("ZAI_API_KEY", "zai-regional-key");
        config.apply_env_overrides();
        assert_eq!(config.api_key.as_deref(), Some("zai-regional-key"));

        std::env::remove_var("ZAI_API_KEY");
    }

    #[test]
    async fn env_override_model() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_MODEL", "gpt-4o");
        config.apply_env_overrides();
        assert_eq!(config.default_model.as_deref(), Some("gpt-4o"));

        std::env::remove_var("TOPCLAW_MODEL");
    }

    #[test]
    async fn model_provider_profile_maps_to_custom_endpoint() {
        let _env_guard = env_override_lock().await;
        let mut config = Config {
            default_provider: Some("sub2api".to_string()),
            model_providers: HashMap::from([(
                "sub2api".to_string(),
                ModelProviderConfig {
                    name: Some("sub2api".to_string()),
                    base_url: Some("https://api.tonsof.blue/v1".to_string()),
                    wire_api: None,
                    requires_openai_auth: false,
                },
            )]),
            ..Config::default()
        };

        config.apply_env_overrides();
        assert_eq!(
            config.default_provider.as_deref(),
            Some("custom:https://api.tonsof.blue/v1")
        );
        assert_eq!(
            config.api_url.as_deref(),
            Some("https://api.tonsof.blue/v1")
        );
    }

    #[test]
    async fn model_provider_profile_responses_uses_openai_codex_and_openai_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config {
            default_provider: Some("sub2api".to_string()),
            model_providers: HashMap::from([(
                "sub2api".to_string(),
                ModelProviderConfig {
                    name: Some("sub2api".to_string()),
                    base_url: Some("https://api.tonsof.blue".to_string()),
                    wire_api: Some("responses".to_string()),
                    requires_openai_auth: true,
                },
            )]),
            api_key: None,
            ..Config::default()
        };

        std::env::set_var("OPENAI_API_KEY", "sk-test-codex-key");
        config.apply_env_overrides();
        std::env::remove_var("OPENAI_API_KEY");

        assert_eq!(config.default_provider.as_deref(), Some("openai-codex"));
        assert_eq!(config.api_url.as_deref(), Some("https://api.tonsof.blue"));
        assert_eq!(config.api_key.as_deref(), Some("sk-test-codex-key"));
    }

    #[test]
    async fn validate_ollama_cloud_model_requires_remote_api_url() {
        let _env_guard = env_override_lock().await;
        let config = Config {
            default_provider: Some("ollama".to_string()),
            default_model: Some("glm-5:cloud".to_string()),
            api_url: None,
            api_key: Some("ollama-key".to_string()),
            ..Config::default()
        };

        let error = config.validate().expect_err("expected validation to fail");
        assert!(error.to_string().contains(
            "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset"
        ));
    }

    #[test]
    async fn validate_ollama_cloud_model_accepts_remote_endpoint_and_env_key() {
        let _env_guard = env_override_lock().await;
        let config = Config {
            default_provider: Some("ollama".to_string()),
            default_model: Some("glm-5:cloud".to_string()),
            api_url: Some("https://ollama.com/api".to_string()),
            api_key: None,
            ..Config::default()
        };

        std::env::set_var("OLLAMA_API_KEY", "ollama-env-key");
        let result = config.validate();
        std::env::remove_var("OLLAMA_API_KEY");

        assert!(result.is_ok(), "expected validation to pass: {result:?}");
    }

    #[test]
    async fn validate_rejects_unknown_model_provider_wire_api() {
        let _env_guard = env_override_lock().await;
        let config = Config {
            default_provider: Some("sub2api".to_string()),
            model_providers: HashMap::from([(
                "sub2api".to_string(),
                ModelProviderConfig {
                    name: Some("sub2api".to_string()),
                    base_url: Some("https://api.tonsof.blue/v1".to_string()),
                    wire_api: Some("ws".to_string()),
                    requires_openai_auth: false,
                },
            )]),
            ..Config::default()
        };

        let error = config.validate().expect_err("expected validation failure");
        assert!(error
            .to_string()
            .contains("wire_api must be one of: responses, chat_completions"));
    }

    #[test]
    async fn env_override_workspace() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_WORKSPACE", "/custom/workspace");
        config.apply_env_overrides();
        assert_eq!(config.workspace_dir, PathBuf::from("/custom/workspace"));

        std::env::remove_var("TOPCLAW_WORKSPACE");
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_env_workspace_first() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let workspace_dir = default_config_dir.join("profile-a");

        std::env::set_var("TOPCLAW_WORKSPACE", &workspace_dir);
        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
        assert_eq!(config_dir, workspace_dir);
        assert_eq!(resolved_workspace_dir, workspace_dir.join("workspace"));

        std::env::remove_var("TOPCLAW_WORKSPACE");
        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_env_config_dir_first() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let explicit_config_dir = default_config_dir.join("explicit-config");
        let marker_config_dir = default_config_dir.join("profiles").join("alpha");
        let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        fs::create_dir_all(&default_config_dir).await.unwrap();
        let state = ActiveWorkspaceState {
            config_dir: marker_config_dir.to_string_lossy().into_owned(),
        };
        fs::write(&state_path, toml::to_string(&state).unwrap())
            .await
            .unwrap();

        std::env::set_var("TOPCLAW_CONFIG_DIR", &explicit_config_dir);
        std::env::remove_var("TOPCLAW_WORKSPACE");

        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::EnvConfigDir);
        assert_eq!(config_dir, explicit_config_dir);
        assert_eq!(
            resolved_workspace_dir,
            explicit_config_dir.join("workspace")
        );

        std::env::remove_var("TOPCLAW_CONFIG_DIR");
        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_active_workspace_marker() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let marker_config_dir = default_config_dir.join("profiles").join("alpha");
        let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        std::env::remove_var("TOPCLAW_WORKSPACE");
        fs::create_dir_all(&default_config_dir).await.unwrap();
        let state = ActiveWorkspaceState {
            config_dir: marker_config_dir.to_string_lossy().into_owned(),
        };
        fs::write(&state_path, toml::to_string(&state).unwrap())
            .await
            .unwrap();

        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::ActiveWorkspaceMarker);
        assert_eq!(config_dir, marker_config_dir);
        assert_eq!(resolved_workspace_dir, marker_config_dir.join("workspace"));

        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_falls_back_to_default_layout() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");

        std::env::remove_var("TOPCLAW_WORKSPACE");
        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
        assert_eq!(config_dir, default_config_dir);
        assert_eq!(resolved_workspace_dir, default_workspace_dir);

        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn load_or_init_workspace_override_uses_workspace_root_for_config() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("profile-a");

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);
        std::env::set_var("TOPCLAW_WORKSPACE", &workspace_dir);

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir.join("workspace"));
        assert_eq!(config.config_path, workspace_dir.join("config.toml"));
        assert!(workspace_dir.join("config.toml").exists());

        std::env::remove_var("TOPCLAW_WORKSPACE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_workspace_suffix_uses_legacy_config_layout() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("workspace");
        let legacy_config_path = temp_home.join(".topclaw").join("config.toml");

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);
        std::env::set_var("TOPCLAW_WORKSPACE", &workspace_dir);

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, legacy_config_path);
        assert!(config.config_path.exists());

        std::env::remove_var("TOPCLAW_WORKSPACE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_workspace_override_keeps_existing_legacy_config() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("custom-workspace");
        let legacy_config_dir = temp_home.join(".topclaw");
        let legacy_config_path = legacy_config_dir.join("config.toml");

        fs::create_dir_all(&legacy_config_dir).await.unwrap();
        fs::write(
            &legacy_config_path,
            r#"default_temperature = 0.7
default_model = "legacy-model"
"#,
        )
        .await
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);
        std::env::set_var("TOPCLAW_WORKSPACE", &workspace_dir);

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, legacy_config_path);
        assert_eq!(config.default_model.as_deref(), Some("legacy-model"));

        std::env::remove_var("TOPCLAW_WORKSPACE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_uses_persisted_active_workspace_marker() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let custom_config_dir = temp_home.join("profiles").join("agent-alpha");

        fs::create_dir_all(&custom_config_dir).await.unwrap();
        fs::write(
            custom_config_dir.join("config.toml"),
            "default_temperature = 0.7\ndefault_model = \"persisted-profile\"\n",
        )
        .await
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);
        std::env::remove_var("TOPCLAW_WORKSPACE");

        persist_active_workspace_config_dir(&custom_config_dir)
            .await
            .unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.config_path, custom_config_dir.join("config.toml"));
        assert_eq!(config.workspace_dir, custom_config_dir.join("workspace"));
        assert_eq!(config.default_model.as_deref(), Some("persisted-profile"));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_env_workspace_override_takes_priority_over_marker() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let marker_config_dir = temp_home.join("profiles").join("persisted-profile");
        let env_workspace_dir = temp_home.join("env-workspace");

        fs::create_dir_all(&marker_config_dir).await.unwrap();
        fs::write(
            marker_config_dir.join("config.toml"),
            "default_temperature = 0.7\ndefault_model = \"marker-model\"\n",
        )
        .await
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);
        persist_active_workspace_config_dir(&marker_config_dir)
            .await
            .unwrap();
        std::env::set_var("TOPCLAW_WORKSPACE", &env_workspace_dir);

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.workspace_dir, env_workspace_dir.join("workspace"));
        assert_eq!(config.config_path, env_workspace_dir.join("config.toml"));

        std::env::remove_var("TOPCLAW_WORKSPACE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn persist_active_workspace_marker_is_written_to_selected_config_root() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let default_config_dir = temp_home.join(".topclaw");
        let custom_config_dir = temp_home.join("profiles").join("custom-profile");
        let default_marker_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);
        let custom_marker_path = custom_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);

        persist_active_workspace_config_dir(&custom_config_dir)
            .await
            .unwrap();

        assert!(custom_marker_path.exists());
        assert!(default_marker_path.exists());

        let custom_state: ActiveWorkspaceState =
            toml::from_str(&fs::read_to_string(&custom_marker_path).await.unwrap()).unwrap();
        assert_eq!(PathBuf::from(custom_state.config_dir), custom_config_dir);

        let default_state: ActiveWorkspaceState =
            toml::from_str(&fs::read_to_string(&default_marker_path).await.unwrap()).unwrap();
        assert_eq!(PathBuf::from(default_state.config_dir), custom_config_dir);

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn persist_active_workspace_marker_tolerates_restricted_default_home_root() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let default_config_root_blocker = temp_home.join(".topclaw");
        let custom_config_dir = temp_home.join("profiles").join("restricted-home-profile");
        let custom_marker_path = custom_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        fs::create_dir_all(&custom_config_dir).await.unwrap();
        fs::write(&default_config_root_blocker, "blocked-as-file")
            .await
            .unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);

        persist_active_workspace_config_dir(&custom_config_dir)
            .await
            .unwrap();

        assert!(custom_marker_path.exists());
        assert!(default_config_root_blocker.is_file());

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn persist_active_workspace_marker_is_cleared_for_default_config_dir() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("topclaw_test_home_{}", uuid::Uuid::new_v4()));
        let default_config_dir = temp_home.join(".topclaw");
        let custom_config_dir = temp_home.join("profiles").join("custom-profile");
        let marker_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);
        let custom_marker_path = custom_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_home);

        persist_active_workspace_config_dir(&custom_config_dir)
            .await
            .unwrap();
        assert!(marker_path.exists());
        assert!(custom_marker_path.exists());

        persist_active_workspace_config_dir(&default_config_dir)
            .await
            .unwrap();
        assert!(!marker_path.exists());
        assert!(custom_marker_path.exists());

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn env_override_empty_values_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_provider = config.default_provider.clone();

        std::env::set_var("TOPCLAW_PROVIDER", "");
        config.apply_env_overrides();
        assert_eq!(config.default_provider, original_provider);

        std::env::remove_var("TOPCLAW_PROVIDER");
    }

    #[test]
    async fn env_override_gateway_port() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.gateway.port, 42617);

        std::env::set_var("TOPCLAW_GATEWAY_PORT", "8080");
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, 8080);

        std::env::remove_var("TOPCLAW_GATEWAY_PORT");
    }

    #[test]
    async fn env_override_port_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::remove_var("TOPCLAW_GATEWAY_PORT");
        std::env::set_var("PORT", "9000");
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, 9000);

        std::env::remove_var("PORT");
    }

    #[test]
    async fn env_override_gateway_host() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.gateway.host, "127.0.0.1");

        std::env::set_var("TOPCLAW_GATEWAY_HOST", "0.0.0.0");
        config.apply_env_overrides();
        assert_eq!(config.gateway.host, "0.0.0.0");

        std::env::remove_var("TOPCLAW_GATEWAY_HOST");
    }

    #[test]
    async fn env_override_host_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::remove_var("TOPCLAW_GATEWAY_HOST");
        std::env::set_var("HOST", "0.0.0.0");
        config.apply_env_overrides();
        assert_eq!(config.gateway.host, "0.0.0.0");

        std::env::remove_var("HOST");
    }

    #[test]
    async fn env_override_temperature() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_TEMPERATURE", "0.5");
        config.apply_env_overrides();
        assert!((config.default_temperature - 0.5).abs() < f64::EPSILON);

        std::env::remove_var("TOPCLAW_TEMPERATURE");
    }

    #[test]
    async fn env_override_temperature_out_of_range_ignored() {
        let _env_guard = env_override_lock().await;
        // Clean up any leftover env vars from other tests
        std::env::remove_var("TOPCLAW_TEMPERATURE");

        let mut config = Config::default();
        let original_temp = config.default_temperature;

        // Temperature > 2.0 should be ignored
        std::env::set_var("TOPCLAW_TEMPERATURE", "3.0");
        config.apply_env_overrides();
        assert!(
            (config.default_temperature - original_temp).abs() < f64::EPSILON,
            "Temperature 3.0 should be ignored (out of range)"
        );

        std::env::remove_var("TOPCLAW_TEMPERATURE");
    }

    #[test]
    async fn env_override_reasoning_enabled() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.runtime.reasoning_enabled, None);

        std::env::set_var("TOPCLAW_REASONING_ENABLED", "false");
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(false));

        std::env::set_var("TOPCLAW_REASONING_ENABLED", "true");
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(true));

        std::env::remove_var("TOPCLAW_REASONING_ENABLED");
    }

    #[test]
    async fn env_override_reasoning_invalid_value_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.runtime.reasoning_enabled = Some(false);

        std::env::set_var("TOPCLAW_REASONING_ENABLED", "maybe");
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(false));

        std::env::remove_var("TOPCLAW_REASONING_ENABLED");
    }

    #[test]
    async fn env_override_model_support_vision() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.model_support_vision, None);

        std::env::set_var("TOPCLAW_MODEL_SUPPORT_VISION", "true");
        config.apply_env_overrides();
        assert_eq!(config.model_support_vision, Some(true));

        std::env::set_var("TOPCLAW_MODEL_SUPPORT_VISION", "false");
        config.apply_env_overrides();
        assert_eq!(config.model_support_vision, Some(false));

        std::env::set_var("TOPCLAW_MODEL_SUPPORT_VISION", "maybe");
        config.model_support_vision = Some(true);
        config.apply_env_overrides();
        assert_eq!(config.model_support_vision, Some(true));

        std::env::remove_var("TOPCLAW_MODEL_SUPPORT_VISION");
    }

    #[test]
    async fn env_override_invalid_port_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_port = config.gateway.port;

        std::env::set_var("PORT", "not_a_number");
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, original_port);

        std::env::remove_var("PORT");
    }

    #[test]
    async fn env_override_web_search_config() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_WEB_SEARCH_ENABLED", "false");
        std::env::set_var("TOPCLAW_WEB_SEARCH_PROVIDER", "brave");
        std::env::set_var("TOPCLAW_WEB_SEARCH_MAX_RESULTS", "7");
        std::env::set_var("TOPCLAW_WEB_SEARCH_TIMEOUT_SECS", "20");
        std::env::set_var("TOPCLAW_BRAVE_API_KEY", "brave-test-key");

        config.apply_env_overrides();

        assert!(!config.web_search.enabled);
        assert_eq!(config.web_search.provider, "brave");
        assert_eq!(config.web_search.max_results, 7);
        assert_eq!(config.web_search.timeout_secs, 20);
        assert_eq!(
            config.web_search.brave_api_key.as_deref(),
            Some("brave-test-key")
        );

        std::env::remove_var("TOPCLAW_WEB_SEARCH_ENABLED");
        std::env::remove_var("TOPCLAW_WEB_SEARCH_PROVIDER");
        std::env::remove_var("TOPCLAW_WEB_SEARCH_MAX_RESULTS");
        std::env::remove_var("TOPCLAW_WEB_SEARCH_TIMEOUT_SECS");
        std::env::remove_var("TOPCLAW_BRAVE_API_KEY");
    }

    #[test]
    async fn env_override_web_search_invalid_values_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_max_results = config.web_search.max_results;
        let original_timeout = config.web_search.timeout_secs;

        std::env::set_var("TOPCLAW_WEB_SEARCH_MAX_RESULTS", "99");
        std::env::set_var("TOPCLAW_WEB_SEARCH_TIMEOUT_SECS", "0");

        config.apply_env_overrides();

        assert_eq!(config.web_search.max_results, original_max_results);
        assert_eq!(config.web_search.timeout_secs, original_timeout);

        std::env::remove_var("TOPCLAW_WEB_SEARCH_MAX_RESULTS");
        std::env::remove_var("TOPCLAW_WEB_SEARCH_TIMEOUT_SECS");
    }

    #[test]
    async fn env_override_storage_provider_config() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        std::env::set_var("TOPCLAW_STORAGE_PROVIDER", "postgres");
        std::env::set_var("TOPCLAW_STORAGE_DB_URL", "postgres://example/db");
        std::env::set_var("TOPCLAW_STORAGE_CONNECT_TIMEOUT_SECS", "15");

        config.apply_env_overrides();

        assert_eq!(config.storage.provider.config.provider, "postgres");
        assert_eq!(
            config.storage.provider.config.db_url.as_deref(),
            Some("postgres://example/db")
        );
        assert_eq!(
            config.storage.provider.config.connect_timeout_secs,
            Some(15)
        );

        std::env::remove_var("TOPCLAW_STORAGE_PROVIDER");
        std::env::remove_var("TOPCLAW_STORAGE_DB_URL");
        std::env::remove_var("TOPCLAW_STORAGE_CONNECT_TIMEOUT_SECS");
    }

    #[test]
    async fn proxy_config_scope_services_requires_entries_when_enabled() {
        let proxy = ProxyConfig {
            enabled: true,
            http_proxy: Some("http://127.0.0.1:7890".into()),
            https_proxy: None,
            all_proxy: None,
            no_proxy: Vec::new(),
            scope: ProxyScope::Services,
            services: Vec::new(),
        };

        let error = proxy.validate().unwrap_err().to_string();
        assert!(error.contains("proxy.scope='services'"));
    }

    #[test]
    async fn env_override_proxy_scope_services() {
        let _env_guard = env_override_lock().await;
        clear_proxy_env_test_vars();

        let mut config = Config::default();
        std::env::set_var("TOPCLAW_PROXY_ENABLED", "true");
        std::env::set_var("TOPCLAW_HTTP_PROXY", "http://127.0.0.1:7890");
        std::env::set_var(
            "TOPCLAW_PROXY_SERVICES",
            "provider.openai, tool.http_request",
        );
        std::env::set_var("TOPCLAW_PROXY_SCOPE", "services");

        config.apply_env_overrides();

        assert!(config.proxy.enabled);
        assert_eq!(config.proxy.scope, ProxyScope::Services);
        assert_eq!(
            config.proxy.http_proxy.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert!(config.proxy.should_apply_to_service("provider.openai"));
        assert!(config.proxy.should_apply_to_service("tool.http_request"));
        assert!(!config.proxy.should_apply_to_service("provider.anthropic"));

        clear_proxy_env_test_vars();
    }

    #[test]
    async fn env_override_proxy_scope_environment_applies_process_env() {
        let _env_guard = env_override_lock().await;
        clear_proxy_env_test_vars();

        let mut config = Config::default();
        std::env::set_var("TOPCLAW_PROXY_ENABLED", "true");
        std::env::set_var("TOPCLAW_PROXY_SCOPE", "environment");
        std::env::set_var("TOPCLAW_HTTP_PROXY", "http://127.0.0.1:7890");
        std::env::set_var("TOPCLAW_HTTPS_PROXY", "http://127.0.0.1:7891");
        std::env::set_var("TOPCLAW_NO_PROXY", "localhost,127.0.0.1");

        config.apply_env_overrides();

        assert_eq!(config.proxy.scope, ProxyScope::Environment);
        assert_eq!(
            std::env::var("HTTP_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7891")
        );
        assert!(std::env::var("NO_PROXY")
            .ok()
            .is_some_and(|value| value.contains("localhost")));

        clear_proxy_env_test_vars();
    }

    fn runtime_proxy_cache_contains(cache_key: &str) -> bool {
        match runtime_proxy_client_cache().read() {
            Ok(guard) => guard.contains_key(cache_key),
            Err(poisoned) => poisoned.into_inner().contains_key(cache_key),
        }
    }

    #[test]
    async fn runtime_proxy_client_cache_reuses_default_profile_key() {
        let service_key = format!(
            "provider.cache_test.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let cache_key = runtime_proxy_cache_key(&service_key, None, None);

        clear_runtime_proxy_client_cache();
        assert!(!runtime_proxy_cache_contains(&cache_key));

        let _ = build_runtime_proxy_client(&service_key);
        assert!(runtime_proxy_cache_contains(&cache_key));

        let _ = build_runtime_proxy_client(&service_key);
        assert!(runtime_proxy_cache_contains(&cache_key));
    }

    #[test]
    async fn set_runtime_proxy_config_clears_runtime_proxy_client_cache() {
        let service_key = format!(
            "provider.cache_timeout_test.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let cache_key = runtime_proxy_cache_key(&service_key, Some(30), Some(5));

        clear_runtime_proxy_client_cache();
        let _ = build_runtime_proxy_client_with_timeouts(&service_key, 30, 5);
        assert!(runtime_proxy_cache_contains(&cache_key));

        set_runtime_proxy_config(ProxyConfig::default());
        assert!(!runtime_proxy_cache_contains(&cache_key));
    }

    #[test]
    async fn gateway_config_default_values() {
        let g = GatewayConfig::default();
        assert_eq!(g.port, 42617);
        assert_eq!(g.host, "127.0.0.1");
        assert!(g.require_pairing);
        assert!(!g.allow_public_bind);
        assert!(g.paired_tokens.is_empty());
        assert!(!g.trust_forwarded_headers);
        assert_eq!(g.rate_limit_max_keys, 10_000);
        assert_eq!(g.idempotency_max_keys, 10_000);
        assert!(!g.node_control.enabled);
        assert!(g.node_control.auth_token.is_none());
        assert!(g.node_control.allowed_node_ids.is_empty());
    }

    // ── Peripherals config ───────────────────────────────────────

    #[test]
    async fn peripherals_config_default_disabled() {
        let p = PeripheralsConfig::default();
        assert!(!p.enabled);
        assert!(p.boards.is_empty());
    }

    #[test]
    async fn peripheral_board_config_defaults() {
        let b = PeripheralBoardConfig::default();
        assert!(b.board.is_empty());
        assert_eq!(b.transport, "serial");
        assert!(b.path.is_none());
        assert_eq!(b.baud, 115_200);
    }

    #[test]
    async fn peripherals_config_toml_roundtrip() {
        let p = PeripheralsConfig {
            enabled: true,
            boards: vec![PeripheralBoardConfig {
                board: "nucleo-f401re".into(),
                transport: "serial".into(),
                path: Some("/dev/ttyACM0".into()),
                baud: 115_200,
            }],
            datasheet_dir: None,
        };
        let toml_str = toml::to_string(&p).unwrap();
        let parsed: PeripheralsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.boards.len(), 1);
        assert_eq!(parsed.boards[0].board, "nucleo-f401re");
        assert_eq!(parsed.boards[0].path.as_deref(), Some("/dev/ttyACM0"));
    }

    #[test]
    async fn lark_config_serde() {
        let lc = LarkConfig {
            app_id: "cli_123456".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["user_123".into(), "user_456".into()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            draft_update_interval_ms: default_lark_draft_update_interval_ms(),
            max_draft_edits: default_lark_max_draft_edits(),
        };
        let json = serde_json::to_string(&lc).unwrap();
        let parsed: LarkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.app_id, "cli_123456");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
        assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    async fn lark_config_toml_roundtrip() {
        let lc = LarkConfig {
            app_id: "cli_123456".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["*".into()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
            draft_update_interval_ms: default_lark_draft_update_interval_ms(),
            max_draft_edits: default_lark_max_draft_edits(),
        };
        let toml_str = toml::to_string(&lc).unwrap();
        let parsed: LarkConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.app_id, "cli_123456");
        assert_eq!(parsed.app_secret, "secret_abc");
    }

    #[test]
    async fn lark_config_deserializes_without_optional_fields() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.encrypt_key.is_none());
        assert!(parsed.verification_token.is_none());
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
    }

    #[test]
    async fn lark_config_with_wildcard_allowed_users() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret","allowed_users":["*"]}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.allowed_users, vec!["*"]);
    }

    #[test]
    async fn lark_group_reply_mode_supports_explicit_override() {
        let json = r#"{
            "app_id":"cli_123",
            "app_secret":"secret",
            "group_reply":{
                "mode":"all_messages",
                "allowed_sender_ids":["ou_1"]
            }
        }"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["ou_1".to_string()]
        );
    }

    #[test]
    async fn feishu_config_serde() {
        let fc = FeishuConfig {
            app_id: "cli_feishu_123".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["user_123".into(), "user_456".into()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            draft_update_interval_ms: default_lark_draft_update_interval_ms(),
            max_draft_edits: default_lark_max_draft_edits(),
        };
        let json = serde_json::to_string(&fc).unwrap();
        let parsed: FeishuConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.app_id, "cli_feishu_123");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
        assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    async fn feishu_config_toml_roundtrip() {
        let fc = FeishuConfig {
            app_id: "cli_feishu_123".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["*".into()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
            draft_update_interval_ms: default_lark_draft_update_interval_ms(),
            max_draft_edits: default_lark_max_draft_edits(),
        };
        let toml_str = toml::to_string(&fc).unwrap();
        let parsed: FeishuConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.app_id, "cli_feishu_123");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.receive_mode, LarkReceiveMode::Webhook);
        assert_eq!(parsed.port, Some(9898));
    }

    #[test]
    async fn feishu_config_deserializes_without_optional_fields() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
        let parsed: FeishuConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.encrypt_key.is_none());
        assert!(parsed.verification_token.is_none());
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.receive_mode, LarkReceiveMode::Websocket);
        assert!(parsed.port.is_none());
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::AllMessages
        );
    }

    #[test]
    async fn feishu_group_reply_mode_supports_mention_only() {
        let json = r#"{
            "app_id":"cli_123",
            "app_secret":"secret",
            "group_reply":{
                "mode":"mention_only",
                "allowed_sender_ids":["ou_9"]
            }
        }"#;
        let parsed: FeishuConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.effective_group_reply_mode(),
            GroupReplyMode::MentionOnly
        );
        assert_eq!(
            parsed.group_reply_allowed_sender_ids(),
            vec!["ou_9".to_string()]
        );
    }

    #[test]
    async fn qq_config_defaults_to_webhook_receive_mode() {
        let json = r#"{"app_id":"123","app_secret":"secret"}"#;
        let parsed: QQConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.receive_mode, QQReceiveMode::Webhook);
        assert!(parsed.allowed_users.is_empty());
    }

    #[test]
    async fn qq_config_toml_roundtrip_receive_mode() {
        let qc = QQConfig {
            app_id: "123".into(),
            app_secret: "secret".into(),
            allowed_users: vec!["*".into()],
            receive_mode: QQReceiveMode::Websocket,
        };
        let toml_str = toml::to_string(&qc).unwrap();
        let parsed: QQConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.receive_mode, QQReceiveMode::Websocket);
        assert_eq!(parsed.allowed_users, vec!["*"]);
    }

    #[test]
    async fn nextcloud_talk_config_serde() {
        let nc = NextcloudTalkConfig {
            base_url: "https://cloud.example.com".into(),
            app_token: "app-token".into(),
            webhook_secret: Some("webhook-secret".into()),
            allowed_users: vec!["user_a".into(), "*".into()],
        };

        let json = serde_json::to_string(&nc).unwrap();
        let parsed: NextcloudTalkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.base_url, "https://cloud.example.com");
        assert_eq!(parsed.app_token, "app-token");
        assert_eq!(parsed.webhook_secret.as_deref(), Some("webhook-secret"));
        assert_eq!(parsed.allowed_users, vec!["user_a", "*"]);
    }

    #[test]
    async fn nextcloud_talk_config_defaults_optional_fields() {
        let json = r#"{"base_url":"https://cloud.example.com","app_token":"app-token"}"#;
        let parsed: NextcloudTalkConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.webhook_secret.is_none());
        assert!(parsed.allowed_users.is_empty());
    }

    // ── Config file permission hardening (Unix only) ───────────────

    #[cfg(unix)]
    #[test]
    async fn new_config_file_has_restricted_permissions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Create a config and save it
        let mut config = Config::default();
        config.config_path = config_path.clone();
        config.save().await.unwrap();

        let meta = fs::metadata(&config_path).await.unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "New config file should be owner-only (0600), got {mode:o}"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn save_restricts_existing_world_readable_config_to_owner_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        let mut config = Config::default();
        config.config_path = config_path.clone();
        config.save().await.unwrap();

        // Simulate the regression state observed in issue #1345.
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let loose_mode = std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            loose_mode, 0o644,
            "test setup requires world-readable config"
        );

        config.default_temperature = 0.6;
        config.save().await.unwrap();

        let hardened_mode = std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            hardened_mode, 0o600,
            "Saving config should restore owner-only permissions (0600)"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn world_readable_config_is_detectable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Create a config file with intentionally loose permissions
        std::fs::write(&config_path, "# test config").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let meta = std::fs::metadata(&config_path).unwrap();
        let mode = meta.permissions().mode();
        assert!(
            mode & 0o004 != 0,
            "Test setup: file should be world-readable (mode {mode:o})"
        );
    }

    #[test]
    async fn transcription_config_defaults() {
        let tc = TranscriptionConfig::default();
        assert!(!tc.enabled);
        assert!(tc.api_url.contains("groq.com"));
        assert_eq!(tc.model, "whisper-large-v3-turbo");
        assert!(tc.language.is_none());
        assert_eq!(tc.max_duration_secs, 120);
    }

    #[test]
    async fn config_roundtrip_with_transcription() {
        let mut config = Config::default();
        config.transcription.enabled = true;
        config.transcription.language = Some("en".into());

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert!(parsed.transcription.enabled);
        assert_eq!(parsed.transcription.language.as_deref(), Some("en"));
        assert_eq!(parsed.transcription.model, "whisper-large-v3-turbo");
    }

    #[test]
    async fn config_without_transcription_uses_defaults() {
        let toml_str = r#"
            default_provider = "openrouter"
            default_model = "test-model"
            default_temperature = 0.7
        "#;
        let parsed: Config = toml::from_str(toml_str).unwrap();
        assert!(!parsed.transcription.enabled);
        assert_eq!(parsed.transcription.max_duration_secs, 120);
    }

    #[test]
    async fn security_defaults_are_backward_compatible() {
        let parsed: Config = toml::from_str(
            r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7
"#,
        )
        .unwrap();

        assert!(!parsed.security.otp.enabled);
        assert_eq!(parsed.security.otp.method, OtpMethod::Totp);
        assert!(!parsed.security.estop.enabled);
        assert!(parsed.security.estop.require_otp_to_resume);
        assert!(parsed.security.syscall_anomaly.enabled);
        assert!(parsed.security.syscall_anomaly.alert_on_unknown_syscall);
        assert!(!parsed.security.syscall_anomaly.baseline_syscalls.is_empty());
        assert!(parsed.security.canary_tokens);
        assert!(!parsed.security.semantic_guard);
        assert_eq!(parsed.security.semantic_guard_collection, "semantic_guard");
        assert!((parsed.security.semantic_guard_threshold - 0.82).abs() < f64::EPSILON);
    }

    #[test]
    async fn security_toml_parses_otp_and_estop_sections() {
        let parsed: Config = toml::from_str(
            r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7

[security]
canary_tokens = false
semantic_guard = true
semantic_guard_collection = "semantic_guard_custom"
semantic_guard_threshold = 0.91

[security.otp]
enabled = true
method = "totp"
token_ttl_secs = 30
cache_valid_secs = 120
gated_actions = ["browser_open"]
gated_domains = ["*.chase.com", "accounts.google.com"]
gated_domain_categories = ["banking"]

[security.estop]
enabled = true
state_file = "~/.topclaw/estop-state.json"
require_otp_to_resume = true

[security.syscall_anomaly]
enabled = true
strict_mode = true
alert_on_unknown_syscall = true
max_denied_events_per_minute = 3
max_total_events_per_minute = 60
max_alerts_per_minute = 10
alert_cooldown_secs = 15
log_path = "syscall-anomalies.log"
baseline_syscalls = ["read", "write", "openat", "close"]
"#,
        )
        .unwrap();

        assert!(parsed.security.otp.enabled);
        assert!(parsed.security.estop.enabled);
        assert!(parsed.security.syscall_anomaly.strict_mode);
        assert_eq!(
            parsed.security.syscall_anomaly.max_denied_events_per_minute,
            3
        );
        assert_eq!(
            parsed.security.syscall_anomaly.max_total_events_per_minute,
            60
        );
        assert_eq!(parsed.security.syscall_anomaly.max_alerts_per_minute, 10);
        assert_eq!(parsed.security.syscall_anomaly.alert_cooldown_secs, 15);
        assert_eq!(parsed.security.syscall_anomaly.baseline_syscalls.len(), 4);
        assert_eq!(parsed.security.otp.gated_actions.len(), 1);
        assert_eq!(parsed.security.otp.gated_domains.len(), 2);
        assert!(!parsed.security.canary_tokens);
        assert!(parsed.security.semantic_guard);
        assert_eq!(
            parsed.security.semantic_guard_collection,
            "semantic_guard_custom"
        );
        assert!((parsed.security.semantic_guard_threshold - 0.91).abs() < f64::EPSILON);
        parsed.validate().unwrap();
    }

    #[test]
    async fn security_validation_rejects_invalid_domain_glob() {
        let mut config = Config::default();
        config.security.otp.gated_domains = vec!["bad domain.com".into()];

        let err = config.validate().expect_err("expected invalid domain glob");
        assert!(err.to_string().contains("gated_domains"));
    }

    #[test]
    async fn reliability_validation_rejects_empty_fallback_api_key_value() {
        let mut config = Config::default();
        config.reliability.fallback_providers = vec!["openrouter".to_string()];
        config
            .reliability
            .fallback_api_keys
            .insert("openrouter".to_string(), "   ".to_string());

        let err = config
            .validate()
            .expect_err("expected fallback_api_keys empty value validation failure");
        assert!(err
            .to_string()
            .contains("reliability.fallback_api_keys.openrouter must not be empty"));
    }

    #[test]
    async fn reliability_validation_rejects_unmapped_fallback_api_key_entry() {
        let mut config = Config::default();
        config.reliability.fallback_providers = vec!["openrouter".to_string()];
        config
            .reliability
            .fallback_api_keys
            .insert("anthropic".to_string(), "sk-ant-test".to_string());

        let err = config
            .validate()
            .expect_err("expected fallback_api_keys mapping validation failure");
        assert!(err
            .to_string()
            .contains("reliability.fallback_api_keys.anthropic has no matching entry"));
    }

    #[test]
    async fn security_validation_rejects_unknown_domain_category() {
        let mut config = Config::default();
        config.security.otp.gated_domain_categories = vec!["not_real".into()];

        let err = config
            .validate()
            .expect_err("expected unknown domain category");
        assert!(err.to_string().contains("gated_domain_categories"));
    }

    #[test]
    async fn security_validation_rejects_zero_token_ttl() {
        let mut config = Config::default();
        config.security.otp.token_ttl_secs = 0;

        let err = config
            .validate()
            .expect_err("expected ttl validation failure");
        assert!(err.to_string().contains("token_ttl_secs"));
    }

    #[test]
    async fn security_validation_rejects_zero_syscall_threshold() {
        let mut config = Config::default();
        config.security.syscall_anomaly.max_denied_events_per_minute = 0;

        let err = config
            .validate()
            .expect_err("expected syscall threshold validation failure");
        assert!(err.to_string().contains("max_denied_events_per_minute"));
    }

    #[test]
    async fn security_validation_rejects_invalid_syscall_baseline_name() {
        let mut config = Config::default();
        config.security.syscall_anomaly.baseline_syscalls =
            vec!["openat".into(), "bad name".into()];

        let err = config
            .validate()
            .expect_err("expected syscall baseline name validation failure");
        assert!(err.to_string().contains("baseline_syscalls"));
    }

    #[test]
    async fn security_validation_rejects_zero_syscall_alert_budget() {
        let mut config = Config::default();
        config.security.syscall_anomaly.max_alerts_per_minute = 0;

        let err = config
            .validate()
            .expect_err("expected syscall alert budget validation failure");
        assert!(err.to_string().contains("max_alerts_per_minute"));
    }

    #[test]
    async fn security_validation_rejects_zero_syscall_cooldown() {
        let mut config = Config::default();
        config.security.syscall_anomaly.alert_cooldown_secs = 0;

        let err = config
            .validate()
            .expect_err("expected syscall cooldown validation failure");
        assert!(err.to_string().contains("alert_cooldown_secs"));
    }

    #[test]
    async fn security_validation_rejects_denied_threshold_above_total_threshold() {
        let mut config = Config::default();
        config.security.syscall_anomaly.max_denied_events_per_minute = 10;
        config.security.syscall_anomaly.max_total_events_per_minute = 5;

        let err = config
            .validate()
            .expect_err("expected syscall threshold ordering validation failure");
        assert!(err
            .to_string()
            .contains("max_denied_events_per_minute must be less than or equal"));
    }

    #[test]
    async fn security_validation_rejects_empty_semantic_guard_collection() {
        let mut config = Config::default();
        config.security.semantic_guard_collection = "   ".to_string();

        let err = config
            .validate()
            .expect_err("expected semantic_guard_collection validation failure");
        assert!(err
            .to_string()
            .contains("security.semantic_guard_collection"));
    }

    #[test]
    async fn security_validation_rejects_invalid_semantic_guard_threshold() {
        let mut config = Config::default();
        config.security.semantic_guard_threshold = 1.5;

        let err = config
            .validate()
            .expect_err("expected semantic_guard_threshold validation failure");
        assert!(err
            .to_string()
            .contains("security.semantic_guard_threshold"));
    }

    #[test]
    async fn coordination_config_defaults() {
        let config = Config::default();
        assert!(config.coordination.enabled);
        assert_eq!(config.coordination.lead_agent, "delegate-lead");
        assert_eq!(config.coordination.max_inbox_messages_per_agent, 256);
        assert_eq!(config.coordination.max_dead_letters, 256);
        assert_eq!(config.coordination.max_context_entries, 512);
        assert_eq!(config.coordination.max_seen_message_ids, 4096);
    }

    #[test]
    async fn config_roundtrip_with_coordination_section() {
        let mut config = Config::default();
        config.coordination.enabled = true;
        config.coordination.lead_agent = "runtime-lead".into();
        config.coordination.max_inbox_messages_per_agent = 128;
        config.coordination.max_dead_letters = 64;
        config.coordination.max_context_entries = 32;
        config.coordination.max_seen_message_ids = 1024;

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert!(parsed.coordination.enabled);
        assert_eq!(parsed.coordination.lead_agent, "runtime-lead");
        assert_eq!(parsed.coordination.max_inbox_messages_per_agent, 128);
        assert_eq!(parsed.coordination.max_dead_letters, 64);
        assert_eq!(parsed.coordination.max_context_entries, 32);
        assert_eq!(parsed.coordination.max_seen_message_ids, 1024);
    }

    #[test]
    async fn coordination_validation_rejects_invalid_limits_and_lead_agent() {
        let mut config = Config::default();
        config.coordination.max_inbox_messages_per_agent = 0;
        let err = config
            .validate()
            .expect_err("expected coordination inbox limit validation failure");
        assert!(err
            .to_string()
            .contains("coordination.max_inbox_messages_per_agent"));

        let mut config = Config::default();
        config.coordination.max_dead_letters = 0;
        let err = config
            .validate()
            .expect_err("expected coordination dead-letter limit validation failure");
        assert!(err.to_string().contains("coordination.max_dead_letters"));

        let mut config = Config::default();
        config.coordination.max_context_entries = 0;
        let err = config
            .validate()
            .expect_err("expected coordination context limit validation failure");
        assert!(err.to_string().contains("coordination.max_context_entries"));

        let mut config = Config::default();
        config.coordination.max_seen_message_ids = 0;
        let err = config
            .validate()
            .expect_err("expected coordination dedupe-window validation failure");
        assert!(err
            .to_string()
            .contains("coordination.max_seen_message_ids"));

        let mut config = Config::default();
        config.coordination.lead_agent = "   ".into();
        let err = config
            .validate()
            .expect_err("expected coordination lead-agent validation failure");
        assert!(err.to_string().contains("coordination.lead_agent"));
    }

    #[test]
    async fn coordination_validation_allows_empty_lead_agent_when_disabled() {
        let mut config = Config::default();
        config.coordination.enabled = false;
        config.coordination.lead_agent = String::new();
        config
            .validate()
            .expect("disabled coordination should allow empty lead agent");
    }

    #[test]
    async fn workspaces_config_defaults_disabled() {
        let config = Config::default();
        assert!(!config.workspaces.enabled);
        assert!(config.workspaces.root.is_none());
    }

    #[test]
    async fn workspaces_config_resolve_root_default_and_relative_override() {
        let config_dir = std::path::PathBuf::from("/tmp/topclaw-config-root");
        let default_cfg = WorkspacesConfig::default();
        assert_eq!(
            default_cfg.resolve_root(&config_dir),
            config_dir.join("workspaces")
        );

        let relative_cfg = WorkspacesConfig {
            enabled: true,
            root: Some("profiles".into()),
        };
        assert_eq!(
            relative_cfg.resolve_root(&config_dir),
            config_dir.join("profiles")
        );
    }
}
