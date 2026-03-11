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
pub mod agents_ipc;
pub mod audit;
pub mod autonomy;
pub mod bridge;
pub mod browser;
pub mod browser_computer_use;
pub mod composio;
pub mod coordination;
pub mod cost;
pub mod cron;
pub mod delegate_agent;
pub mod embedding_route;
pub mod estop;
pub mod gateway;
pub mod goal_loop;
pub mod hardware;
pub mod heartbeat;
pub mod hooks;
pub mod http_request;
pub mod identity;
pub mod model_provider;
pub mod model_route;
pub mod multimodal;
pub mod observability;
pub mod otp;
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
pub mod self_improvement;
pub mod skills;
pub mod storage_provider;
pub mod traits;
pub mod transcription;
pub mod tunnel;
pub mod web_tools;
pub mod workspaces;

#[allow(unused_imports)]
pub use agent::AgentConfig;
#[allow(unused_imports)]
pub use agents_ipc::AgentsIpcConfig;
#[allow(unused_imports)]
pub use audit::AuditConfig;
#[allow(unused_imports)]
pub use autonomy::{AutonomyConfig, NonCliNaturalLanguageApprovalMode};
#[allow(unused_imports)]
pub use bridge::BridgeConfig;
#[allow(unused_imports)]
pub use browser::BrowserConfig;
#[allow(unused_imports)]
pub use browser_computer_use::BrowserComputerUseConfig;
#[allow(unused_imports)]
pub use composio::ComposioConfig;
#[allow(unused_imports)]
pub use coordination::CoordinationConfig;
#[allow(unused_imports)]
pub use cost::CostConfig;
#[allow(unused_imports)]
pub use cron::CronConfig;
#[allow(unused_imports)]
pub use delegate_agent::DelegateAgentConfig;
#[allow(unused_imports)]
pub use embedding_route::EmbeddingRouteConfig;
#[allow(unused_imports)]
pub use estop::EstopConfig;
#[allow(unused_imports)]
pub use gateway::{GatewayConfig, NodeControlConfig};
#[allow(unused_imports)]
pub use goal_loop::GoalLoopConfig;
#[allow(unused_imports)]
pub use hardware::{HardwareConfig, HardwareTransport, PeripheralBoardConfig, PeripheralsConfig};
#[allow(unused_imports)]
pub use heartbeat::HeartbeatConfig;
#[allow(unused_imports)]
pub use hooks::{BuiltinHooksConfig, HooksConfig};
#[allow(unused_imports)]
pub use http_request::HttpRequestConfig;
#[allow(unused_imports)]
pub use identity::IdentityConfig;
#[allow(unused_imports)]
pub use model_provider::ModelProviderConfig;
#[allow(unused_imports)]
pub use model_route::ModelRouteConfig;
#[allow(unused_imports)]
pub use multimodal::MultimodalConfig;
#[allow(unused_imports)]
pub use observability::ObservabilityConfig;
#[allow(unused_imports)]
pub use otp::{OtpConfig, OtpMethod};
#[allow(unused_imports)]
pub use provider::ProviderConfig;
#[allow(unused_imports)]
pub use proxy::{
    apply_runtime_proxy_to_builder, build_runtime_proxy_client,
    build_runtime_proxy_client_with_timeouts, runtime_proxy_config, set_runtime_proxy_config,
    ProxyConfig, ProxyScope,
};
#[allow(unused_imports)]
pub use query_classification::{ClassificationRule, QueryClassificationConfig};
#[allow(unused_imports)]
pub use reliability::ReliabilityConfig;
#[allow(unused_imports)]
pub use research::{ResearchPhaseConfig, ResearchTrigger};
#[allow(unused_imports)]
pub use resource_limits::ResourceLimitsConfig;
#[allow(unused_imports)]
pub use runtime::{
    DockerRuntimeConfig, RuntimeConfig, WasmCapabilityEscalationMode, WasmModuleHashPolicy,
    WasmRuntimeConfig, WasmSecurityConfig,
};
#[allow(unused_imports)]
pub use sandbox::{SandboxBackend, SandboxConfig};
#[allow(unused_imports)]
pub use scheduler::SchedulerConfig;
#[allow(unused_imports)]
pub use schema::{
    ChannelsConfig, Config, DiscordConfig, FeishuConfig, GroupReplyConfig, GroupReplyMode,
    IMessageConfig, LarkConfig, MatrixConfig, MemoryConfig, NextcloudTalkConfig, QdrantConfig,
    SecurityConfig, SlackConfig, StorageConfig, StreamMode, SyscallAnomalyConfig, TelegramConfig,
    WebhookConfig,
};
#[allow(unused_imports)]
pub use secrets::SecretsConfig;
pub use self_improvement::SelfImprovementConfig;
#[allow(unused_imports)]
pub use skills::{parse_skills_prompt_injection_mode, SkillsConfig, SkillsPromptInjectionMode};
#[allow(unused_imports)]
pub use storage_provider::{StorageProviderConfig, StorageProviderSection};
pub use transcription::TranscriptionConfig;
#[allow(unused_imports)]
pub use tunnel::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
    TunnelConfig,
};
#[allow(unused_imports)]
pub use web_tools::{WebFetchConfig, WebSearchConfig};
#[allow(unused_imports)]
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
            mention_only: false,
            group_reply: None,
            base_url: None,
        };

        let discord = DiscordConfig {
            bot_token: "token".into(),
            guild_id: Some("123".into()),
            allowed_users: vec![],
            listen_to_bots: false,
            mention_only: false,
            group_reply: None,
        };

        let lark = LarkConfig {
            app_id: "app-id".into(),
            app_secret: "app-secret".into(),
            encrypt_key: None,
            verification_token: None,
            allowed_users: vec![],
            mention_only: false,
            group_reply: None,
            use_feishu: false,
            receive_mode: crate::config::schema::LarkReceiveMode::Websocket,
            port: None,
            draft_update_interval_ms: crate::config::schema::default_lark_draft_update_interval_ms(
            ),
            max_draft_edits: crate::config::schema::default_lark_max_draft_edits(),
        };
        let feishu = FeishuConfig {
            app_id: "app-id".into(),
            app_secret: "app-secret".into(),
            encrypt_key: None,
            verification_token: None,
            allowed_users: vec![],
            group_reply: None,
            receive_mode: crate::config::schema::LarkReceiveMode::Websocket,
            port: None,
            draft_update_interval_ms: crate::config::schema::default_lark_draft_update_interval_ms(
            ),
            max_draft_edits: crate::config::schema::default_lark_max_draft_edits(),
        };

        let nextcloud_talk = NextcloudTalkConfig {
            base_url: "https://cloud.example.com".into(),
            app_token: "app-token".into(),
            webhook_secret: None,
            allowed_users: vec!["*".into()],
        };

        assert_eq!(telegram.allowed_users.len(), 1);
        assert_eq!(discord.guild_id.as_deref(), Some("123"));
        assert_eq!(lark.app_id, "app-id");
        assert_eq!(feishu.app_id, "app-id");
        assert_eq!(nextcloud_talk.base_url, "https://cloud.example.com");
    }
}
