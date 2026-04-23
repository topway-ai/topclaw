//! Configuration surface for TopClaw.
//!
//! The config subsystem is both an internal wiring layer and a user-facing API:
//! `config.toml`, environment-variable overrides, and generated schemas all map
//! back to the types re-exported from this module.
//!
//! Most callers interact with [`Config`](schema::Config). The smaller config
//! structs re-exported here model specific runtime areas such as providers,
//! channels, sandboxing, memory, and observability.
pub mod agent;
pub mod audit;
pub mod autonomy;
pub mod browser;
pub mod browser_domain_grants;
pub mod coordination;
pub mod cost;
pub mod cron;
pub mod delegate_agent;
pub mod embedding_route;
pub mod estop;
pub mod gateway;
pub mod heartbeat;
pub mod hooks;
pub mod http_request;
pub mod model_provider;
pub mod model_route;
pub mod multimodal;
pub mod observability;
pub mod otp;
pub mod patch;
pub mod provider;
pub mod proxy;
pub mod query_classification;
pub mod reliability;
pub mod research;
pub mod resource_limits;
pub mod runtime;
pub mod sandbox;
pub mod scheduler;
pub mod schema;
pub mod secrets;
pub mod skills;
pub mod storage_provider;
pub mod traits;
pub mod transcription;
pub mod tunnel;
pub mod web_tools;
pub mod workspaces;

pub use agent::AgentConfig;
pub use audit::AuditConfig;
pub use autonomy::{AutonomyConfig, NonCliNaturalLanguageApprovalMode};
pub use browser::{BrowserComputerUseConfig, BrowserConfig};
pub use browser_domain_grants::{BrowserAllowlist, BrowserDomainGrant};
pub use coordination::CoordinationConfig;
pub use cost::CostConfig;
pub use cron::CronConfig;
pub use delegate_agent::DelegateAgentConfig;
pub use embedding_route::EmbeddingRouteConfig;
pub use estop::EstopConfig;
pub use gateway::{GatewayConfig, NodeControlConfig};
pub use heartbeat::HeartbeatConfig;
pub use hooks::{BuiltinHooksConfig, HooksConfig};
pub use http_request::HttpRequestConfig;
pub use model_provider::ModelProviderConfig;
pub use model_route::ModelRouteConfig;
pub use multimodal::MultimodalConfig;
pub use observability::ObservabilityConfig;
pub use otp::{OtpConfig, OtpMethod};
pub use provider::ProviderConfig;
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
pub use query_classification::{ClassificationRule, QueryClassificationConfig};
pub use reliability::ReliabilityConfig;
pub use research::{ResearchPhaseConfig, ResearchTrigger};
pub use resource_limits::ResourceLimitsConfig;
pub use runtime::{
    DockerRuntimeConfig, RuntimeConfig, WasmCapabilityEscalationMode, WasmModuleHashPolicy,
    WasmRuntimeConfig, WasmSecurityConfig,
};
// Note: Docker/Wasm runtime config types kept for backward-compatible config deserialization
pub use sandbox::{SandboxBackend, SandboxConfig};
pub use scheduler::SchedulerConfig;
pub use schema::{config_dir_for_home, default_config_dir, default_config_dir_or_fallback};
pub use schema::{
    ChannelsConfig, Config, DiscordConfig, GroupReplyConfig, GroupReplyMode, MemoryConfig,
    SecurityConfig, StorageConfig, StreamMode, SyscallAnomalyConfig, TelegramConfig, WebhookConfig,
};
pub use secrets::SecretsConfig;
pub use skills::{parse_skills_prompt_injection_mode, SkillsConfig, SkillsPromptInjectionMode};
pub use storage_provider::{StorageProviderConfig, StorageProviderSection};
pub use transcription::TranscriptionConfig;
pub use tunnel::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
    TunnelConfig,
};
pub use web_tools::{WebFetchConfig, WebSearchConfig};
pub use workspaces::WorkspacesConfig;

pub fn name_and_presence<T: traits::ChannelConfig>(channel: Option<&T>) -> (&'static str, bool) {
    (T::name(), channel.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_config_default_is_constructible() {
        let config = Config::default();

        assert!(config.default_provider.is_some());
        assert!(config.default_model.is_some());
        assert!(config.default_temperature > 0.0);
    }

    #[test]
    fn reexported_channel_configs_are_constructible() {
        let telegram = TelegramConfig {
            bot_token: "token".into(),
            allowed_users: vec!["alice".into()],
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        };

        let discord = DiscordConfig {
            bot_token: "token".into(),
            guild_id: Some("123".into()),
            allowed_users: vec![],
            listen_to_bots: false,
            group_reply: None,
        };

        assert_eq!(telegram.allowed_users.len(), 1);
        assert_eq!(discord.guild_id.as_deref(), Some("123"));
    }
}
