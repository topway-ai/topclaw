//! Channel subsystem for messaging platform integrations.
//!
//! This module provides the multi-channel messaging infrastructure that connects
//! TopClaw to external platforms. Each channel implements the [`Channel`] trait
//! defined in [`traits`], which provides a uniform interface for sending messages,
//! listening for incoming messages, health checking, and typing indicators.
//!
//! Channels are instantiated by [`start_channels`] based on the runtime configuration.
//! The subsystem manages per-sender conversation history, concurrent message processing
//! with configurable parallelism, and exponential-backoff reconnection for resilience.
//!
//! # Feature-Gated Channels
//!
//! Channels are compiled conditionally based on feature flags to reduce binary size:
//! - `channel-telegram` - Telegram bot support (default)
//! - `channel-discord` - Discord bot support (feature-gated)
//!
//! # Extension
//!
//! To add a new channel, implement [`Channel`] in a new submodule and wire it into
//! [`start_channels`]. See `AGENTS.md` §7.2 for the full change playbook.

// ============================================================================
// Submodules
// ============================================================================

mod capability_detection;
mod capability_recovery;
pub mod cli;
mod command_handler;
mod context;
#[cfg(feature = "channel-discord")]
pub mod discord;
mod dispatch;
mod factory;
mod helpers;
mod message_processing;
mod prompt;
mod route_state;
mod runtime_commands;
mod runtime_config;
mod runtime_help;
pub(crate) mod runtime_helpers;
mod sanitize;
mod startup;
pub mod telegram;
pub mod traits;
pub mod transcription;

// ============================================================================
// Public API re-exports
// ============================================================================

pub use cli::CliChannel;
#[cfg(feature = "channel-discord")]
pub use discord::DiscordChannel;
pub use factory::{collect_configured_channels, ConfiguredChannel};
pub use prompt::{build_system_prompt, build_system_prompt_with_mode};
pub use startup::{doctor_channels, handle_command, start_channels};
pub use telegram::TelegramChannel;
pub use traits::{Channel, SendMessage};

// Re-export for crate-internal use
pub(crate) use runtime_commands::APPROVAL_ALL_TOOLS_ONCE_TOKEN;
pub(crate) use sanitize::sanitize_channel_response;

// Re-export constants needed by parent module
pub(super) use context::BOOTSTRAP_MAX_CHARS;

// ============================================================================
// Internal re-exports for test access
// ============================================================================

// Pull items from extracted submodules into this namespace so the existing
// test module (which uses `use super::*`) continues to compile unchanged.

use context::*;
use helpers::*;
use sanitize::strip_tool_call_tags;

#[cfg(test)]
mod tests {
    use super::capability_detection::*;
    use super::capability_recovery::*;
    use super::context::*;
    use super::dispatch::*;
    use super::helpers::*;
    use super::message_processing::*;
    use super::prompt::build_channel_system_prompt;
    use super::route_state::{
        append_sender_turn, compact_sender_history, rollback_orphan_user_turn,
    };
    use super::runtime_commands::*;
    use super::runtime_config::*;
    use super::runtime_helpers::*;
    use super::sanitize::*;
    use super::startup::*;
    use super::*;
    use crate::approval::ApprovalManager;
    use crate::config::Config;
    use crate::memory::{Memory, MemoryCategory, SqliteMemory};
    use crate::observability::NoopObserver;
    use crate::providers::{self, ChatMessage, Provider};
    use crate::tools::{Tool, ToolResult};
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn make_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        // Create minimal workspace files
        std::fs::write(tmp.path().join("SOUL.md"), "# Soul\nBe helpful.").unwrap();
        std::fs::write(tmp.path().join("IDENTITY.md"), "# Identity\nName: TopClaw").unwrap();
        std::fs::write(tmp.path().join("USER.md"), "# User\nName: Test User").unwrap();
        std::fs::write(
            tmp.path().join("AGENTS.md"),
            "# Agents\nFollow instructions.",
        )
        .unwrap();
        std::fs::write(tmp.path().join("TOOLS.md"), "# Tools\nUse shell carefully.").unwrap();
        std::fs::write(
            tmp.path().join("HEARTBEAT.md"),
            "# Heartbeat\nCheck status.",
        )
        .unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "# Memory\nUser likes Rust.").unwrap();
        tmp
    }

    #[test]
    fn effective_channel_message_timeout_secs_clamps_to_minimum() {
        assert_eq!(
            effective_channel_message_timeout_secs(0),
            MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
        );
        assert_eq!(
            effective_channel_message_timeout_secs(15),
            MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
        );
        assert_eq!(effective_channel_message_timeout_secs(300), 300);
    }

    #[test]
    fn channel_message_timeout_budget_scales_with_tool_iterations() {
        assert_eq!(channel_message_timeout_budget_secs(300, 1), 300);
        assert_eq!(channel_message_timeout_budget_secs(300, 2), 600);
        assert_eq!(channel_message_timeout_budget_secs(300, 3), 900);
    }

    #[test]
    fn channel_message_timeout_budget_uses_safe_defaults_and_cap() {
        // 0 iterations falls back to 1x timeout budget.
        assert_eq!(channel_message_timeout_budget_secs(300, 0), 300);
        // Large iteration counts are capped to avoid runaway waits.
        assert_eq!(
            channel_message_timeout_budget_secs(300, 10),
            300 * CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP
        );
    }

    #[test]
    fn parse_runtime_command_allows_approval_commands_on_non_model_channels() {
        assert_eq!(
            parse_runtime_command("slack", "/approve-request shell"),
            Some(ChannelRuntimeCommand::RequestToolApproval(
                "shell".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve-all-once"),
            Some(ChannelRuntimeCommand::RequestAllToolsOnce)
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve-confirm apr-deadbeef"),
            Some(ChannelRuntimeCommand::ConfirmToolApproval(
                "apr-deadbeef".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve-allow apr-deadbeef"),
            Some(ChannelRuntimeCommand::ApprovePendingRequest(
                "apr-deadbeef".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve-deny apr-deadbeef"),
            Some(ChannelRuntimeCommand::DenyToolApproval(
                "apr-deadbeef".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve-pending"),
            Some(ChannelRuntimeCommand::ListPendingApprovals)
        );
        assert_eq!(
            parse_runtime_command("slack", "/approve shell"),
            Some(ChannelRuntimeCommand::ApproveTool("shell".to_string()))
        );
        assert_eq!(
            parse_runtime_command("slack", "/unapprove shell"),
            Some(ChannelRuntimeCommand::UnapproveTool("shell".to_string()))
        );
        assert_eq!(
            parse_runtime_command("slack", "/approvals"),
            Some(ChannelRuntimeCommand::ListApprovals)
        );
        assert_eq!(parse_runtime_command("slack", "/models"), None);
    }

    #[test]
    fn parse_runtime_command_supports_natural_language_approval_intents() {
        assert_eq!(
            parse_runtime_command("telegram", "授权工具 shell"),
            Some(ChannelRuntimeCommand::RequestToolApproval(
                "shell".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("telegram", "请放开 shell"),
            Some(ChannelRuntimeCommand::RequestToolApproval(
                "shell".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("telegram", "approve tool shell"),
            Some(ChannelRuntimeCommand::RequestToolApproval(
                "shell".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("telegram", "请一次性允许所有工具和命令"),
            Some(ChannelRuntimeCommand::RequestAllToolsOnce)
        );
        assert_eq!(
            parse_runtime_command("telegram", "确认授权 apr-deadbeef"),
            Some(ChannelRuntimeCommand::ConfirmToolApproval(
                "apr-deadbeef".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("telegram", "confirm apr-deadbeef"),
            Some(ChannelRuntimeCommand::ConfirmToolApproval(
                "apr-deadbeef".to_string()
            ))
        );
        assert_eq!(
            parse_runtime_command("telegram", "撤销工具 shell"),
            Some(ChannelRuntimeCommand::UnapproveTool("shell".to_string()))
        );
        assert_eq!(
            parse_runtime_command("telegram", "revoke tool shell"),
            Some(ChannelRuntimeCommand::UnapproveTool("shell".to_string()))
        );
        assert_eq!(
            parse_runtime_command("telegram", "查看授权"),
            Some(ChannelRuntimeCommand::ListApprovals)
        );
        assert_eq!(
            parse_runtime_command("telegram", "show approvals"),
            Some(ChannelRuntimeCommand::ListApprovals)
        );
        assert_eq!(
            parse_runtime_command("telegram", "show pending approvals"),
            Some(ChannelRuntimeCommand::ListPendingApprovals)
        );
        assert_eq!(parse_runtime_command("telegram", "请帮我执行shell"), None);
    }

    #[test]
    fn context_window_overflow_error_detector_matches_known_messages() {
        let overflow_err = anyhow::anyhow!(
            "OpenAI Codex stream error: Your input exceeds the context window of this model."
        );
        assert!(is_context_window_overflow_error(&overflow_err));

        let other_err =
            anyhow::anyhow!("OpenAI Codex API error (502 Bad Gateway): error code: 502");
        assert!(!is_context_window_overflow_error(&other_err));
    }

    #[test]
    fn heartbeat_ok_sentinel_detection_supports_prefix_and_case_insensitive() {
        assert!(is_heartbeat_ok_sentinel("HEARTBEAT_OK"));
        assert!(is_heartbeat_ok_sentinel(" heartbeat_ok - no updates"));
        assert!(is_heartbeat_ok_sentinel("\nHeArTbEaT_oK still nominal"));
        assert!(!is_heartbeat_ok_sentinel("The heartbeat is healthy"));
    }

    #[test]
    fn agent_noop_sentinel_detection_supports_heartbeat_ok_and_no_reply() {
        assert!(is_agent_noop_sentinel("HEARTBEAT_OK"));
        assert!(is_agent_noop_sentinel(" no_reply "));
        assert!(!is_agent_noop_sentinel("status update available"));
    }

    #[test]
    fn memory_context_skip_rules_exclude_history_blobs() {
        assert!(should_skip_memory_context_entry(
            "telegram_123_history",
            r#"[{"role":"user"}]"#
        ));
        assert!(should_skip_memory_context_entry(
            "assistant_resp_legacy",
            "fabricated memory"
        ));
        assert!(!should_skip_memory_context_entry("telegram_123_45", "hi"));
    }

    #[test]
    fn normalize_cached_channel_turns_merges_consecutive_user_turns() {
        let turns = vec![
            ChatMessage::user("forwarded content"),
            ChatMessage::user("summarize this"),
        ];

        let normalized = normalize_cached_channel_turns(turns);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].role, "user");
        assert!(normalized[0].content.contains("forwarded content"));
        assert!(normalized[0].content.contains("summarize this"));
    }

    #[test]
    fn normalize_cached_channel_turns_merges_consecutive_assistant_turns() {
        let turns = vec![
            ChatMessage::user("first user"),
            ChatMessage::assistant("assistant part 1"),
            ChatMessage::assistant("assistant part 2"),
            ChatMessage::user("next user"),
        ];

        let normalized = normalize_cached_channel_turns(turns);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].role, "user");
        assert_eq!(normalized[1].role, "assistant");
        assert_eq!(normalized[2].role, "user");
        assert!(normalized[1].content.contains("assistant part 1"));
        assert!(normalized[1].content.contains("assistant part 2"));
    }

    /// Verify that an orphan user turn followed by a failure-marker assistant
    /// turn normalizes correctly, so the LLM sees the failed request as closed
    /// and does not re-execute it on the next user message.
    #[test]
    fn normalize_preserves_failure_marker_after_orphan_user_turn() {
        let turns = vec![
            ChatMessage::user("download something from GitHub"),
            ChatMessage::assistant("[Task failed — not continuing this request]"),
            ChatMessage::user("what is WAL?"),
        ];

        let normalized = normalize_cached_channel_turns(turns);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].role, "user");
        assert_eq!(normalized[1].role, "assistant");
        assert!(normalized[1].content.contains("Task failed"));
        assert_eq!(normalized[2].role, "user");
        assert_eq!(normalized[2].content, "what is WAL?");
    }

    /// Same as above but for the timeout variant.
    #[test]
    fn normalize_preserves_timeout_marker_after_orphan_user_turn() {
        let turns = vec![
            ChatMessage::user("run a long task"),
            ChatMessage::assistant("[Task timed out — not continuing this request]"),
            ChatMessage::user("next question"),
        ];

        let normalized = normalize_cached_channel_turns(turns);
        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[1].role, "assistant");
        assert!(normalized[1].content.contains("Task timed out"));
        assert_eq!(normalized[2].content, "next question");
    }

    #[test]
    fn compact_sender_history_keeps_recent_truncated_messages() {
        let mut histories = HashMap::new();
        let sender = "telegram_u1".to_string();
        histories.insert(
            sender.clone(),
            (0..20)
                .map(|idx| {
                    let content = format!("msg-{idx}-{}", "x".repeat(700));
                    if idx % 2 == 0 {
                        ChatMessage::user(content)
                    } else {
                        ChatMessage::assistant(content)
                    }
                })
                .collect::<Vec<_>>(),
        );

        let ctx = ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("system".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(histories)),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        };

        assert!(compact_sender_history(&ctx, &sender));

        let histories = ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let kept = histories
            .get(&sender)
            .expect("sender history should remain");
        assert_eq!(kept.len(), CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES);
        assert!(kept.iter().all(|turn| {
            let len = turn.content.chars().count();
            len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS
                || (len <= CHANNEL_HISTORY_COMPACT_CONTENT_CHARS + 3
                    && turn.content.ends_with("..."))
        }));
    }

    #[test]
    fn append_sender_turn_stores_single_turn_per_call() {
        let sender = "telegram_u2".to_string();
        let ctx = ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("system".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        };

        append_sender_turn(&ctx, &sender, ChatMessage::user("hello"));

        let histories = ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let turns = histories.get(&sender).expect("sender history should exist");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello");
    }

    #[test]
    fn rollback_orphan_user_turn_removes_only_latest_matching_user_turn() {
        let sender = "telegram_u3".to_string();
        let mut histories = HashMap::new();
        histories.insert(
            sender.clone(),
            vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("ok"),
                ChatMessage::user("pending"),
            ],
        );
        let ctx = ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("system".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(histories)),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        };

        assert!(rollback_orphan_user_turn(&ctx, &sender, "pending"));

        let histories = ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let turns = histories
            .get(&sender)
            .expect("sender history should remain");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "first");
        assert_eq!(turns[1].content, "ok");
    }

    struct DummyProvider;

    #[async_trait::async_trait]
    impl Provider for DummyProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".to_string())
        }
    }

    struct HeartbeatOkProvider;

    #[async_trait::async_trait]
    impl Provider for HeartbeatOkProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("HEARTBEAT_OK".to_string())
        }
    }

    struct AggregatedFailureProvider;

    #[async_trait::async_trait]
    impl Provider for AggregatedFailureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            anyhow::bail!(
                "All providers/models failed. Attempts:\nprovider=openrouter model=minimax/minimax-m2.7 attempt 1/3: retryable; error=error decoding response body\nprovider=openrouter model=minimax/minimax-m2.7 attempt 2/3: retryable; error=error decoding response body\nprovider=openrouter model=minimax/minimax-m2.7 attempt 3/3: retryable; error=error decoding response body"
            )
        }
    }

    #[derive(Default)]
    struct RecordingChannel {
        sent_messages: tokio::sync::Mutex<Vec<String>>,
        start_typing_calls: AtomicUsize,
        stop_typing_calls: AtomicUsize,
        reactions_added: tokio::sync::Mutex<Vec<(String, String, String)>>,
        reactions_removed: tokio::sync::Mutex<Vec<(String, String, String)>>,
    }

    #[derive(Default)]
    struct TelegramRecordingChannel {
        sent_messages: tokio::sync::Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct DraftStreamingRecordingChannel {
        sent_messages: tokio::sync::Mutex<Vec<String>>,
        draft_updates: tokio::sync::Mutex<Vec<String>>,
        finalized_drafts: tokio::sync::Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct TelegramDraftStreamingRecordingChannel {
        sent_messages: tokio::sync::Mutex<Vec<String>>,
        draft_updates: tokio::sync::Mutex<Vec<String>>,
        finalized_drafts: tokio::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Channel for TelegramRecordingChannel {
        fn name(&self) -> &str {
            "telegram"
        }

        async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
            self.sent_messages
                .lock()
                .await
                .push(format!("{}:{}", message.recipient, message.content));
            Ok(())
        }

        async fn listen(
            &self,
            _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Channel for DraftStreamingRecordingChannel {
        fn name(&self) -> &str {
            "draft-streaming-channel"
        }

        async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
            self.sent_messages
                .lock()
                .await
                .push(format!("{}:{}", message.recipient, message.content));
            Ok(())
        }

        async fn listen(
            &self,
            _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn supports_draft_updates(&self) -> bool {
            true
        }

        async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
            self.sent_messages
                .lock()
                .await
                .push(format!("draft:{}:{}", message.recipient, message.content));
            Ok(Some("draft-1".to_string()))
        }

        async fn update_draft(
            &self,
            _recipient: &str,
            _message_id: &str,
            text: &str,
        ) -> anyhow::Result<Option<String>> {
            self.draft_updates.lock().await.push(text.to_string());
            Ok(None)
        }

        async fn finalize_draft(
            &self,
            _recipient: &str,
            _message_id: &str,
            text: &str,
        ) -> anyhow::Result<()> {
            self.finalized_drafts.lock().await.push(text.to_string());
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Channel for TelegramDraftStreamingRecordingChannel {
        fn name(&self) -> &str {
            "telegram"
        }

        async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
            self.sent_messages
                .lock()
                .await
                .push(format!("{}:{}", message.recipient, message.content));
            Ok(())
        }

        async fn listen(
            &self,
            _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn supports_draft_updates(&self) -> bool {
            true
        }

        async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
            self.sent_messages
                .lock()
                .await
                .push(format!("draft:{}:{}", message.recipient, message.content));
            Ok(Some("draft-1".to_string()))
        }

        async fn update_draft(
            &self,
            recipient: &str,
            _message_id: &str,
            text: &str,
        ) -> anyhow::Result<Option<String>> {
            self.draft_updates
                .lock()
                .await
                .push(format!("{recipient}:{text}"));
            Ok(None)
        }

        async fn finalize_draft(
            &self,
            recipient: &str,
            _message_id: &str,
            text: &str,
        ) -> anyhow::Result<()> {
            self.finalized_drafts
                .lock()
                .await
                .push(format!("{recipient}:{text}"));
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Channel for RecordingChannel {
        fn name(&self) -> &str {
            "test-channel"
        }

        async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
            self.sent_messages
                .lock()
                .await
                .push(format!("{}:{}", message.recipient, message.content));
            Ok(())
        }

        async fn listen(
            &self,
            _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_typing(&self, _recipient: &str) -> anyhow::Result<()> {
            self.start_typing_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn stop_typing(&self, _recipient: &str) -> anyhow::Result<()> {
            self.stop_typing_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn add_reaction(
            &self,
            channel_id: &str,
            message_id: &str,
            emoji: &str,
        ) -> anyhow::Result<()> {
            self.reactions_added.lock().await.push((
                channel_id.to_string(),
                message_id.to_string(),
                emoji.to_string(),
            ));
            Ok(())
        }

        async fn remove_reaction(
            &self,
            channel_id: &str,
            message_id: &str,
            emoji: &str,
        ) -> anyhow::Result<()> {
            self.reactions_removed.lock().await.push((
                channel_id.to_string(),
                message_id.to_string(),
                emoji.to_string(),
            ));
            Ok(())
        }
    }

    struct SlowProvider {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Provider for SlowProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            tokio::time::sleep(self.delay).await;
            Ok(format!("echo: {message}"))
        }
    }

    struct ParallelProbeProvider {
        delay: Duration,
        active_calls: Arc<AtomicUsize>,
        max_active_calls: Arc<AtomicUsize>,
    }

    impl ParallelProbeProvider {
        fn new(delay: Duration) -> Self {
            Self {
                delay,
                active_calls: Arc::new(AtomicUsize::new(0)),
                max_active_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn max_active_calls(&self) -> usize {
            self.max_active_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Provider for ParallelProbeProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let current = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = self.max_active_calls.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |previous| Some(previous.max(current)),
            );

            tokio::time::sleep(self.delay).await;
            self.active_calls.fetch_sub(1, Ordering::SeqCst);
            Ok(format!("echo: {message}"))
        }
    }

    struct ToolCallingProvider;

    struct NamedTestTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for NamedTestTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "named test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {}
            })
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::tools::ToolResult> {
            Ok(crate::tools::ToolResult {
                success: true,
                output: String::new(),
                error: None,
            })
        }
    }

    fn tool_call_payload() -> String {
        r#"<tool_call>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</tool_call>"#
            .to_string()
    }

    fn tool_call_payload_with_alias_tag() -> String {
        r#"<toolcall>
{"name":"mock_price","arguments":{"symbol":"BTC"}}
</toolcall>"#
            .to_string()
    }

    #[async_trait::async_trait]
    impl Provider for ToolCallingProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(tool_call_payload())
        }

        async fn chat_with_history(
            &self,
            messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let has_tool_results = messages
                .iter()
                .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
            if has_tool_results {
                Ok("BTC is currently around $65,000 based on latest tool output.".to_string())
            } else {
                Ok(tool_call_payload())
            }
        }
    }

    struct ToolCallingAliasProvider;

    #[async_trait::async_trait]
    impl Provider for ToolCallingAliasProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(tool_call_payload_with_alias_tag())
        }

        async fn chat_with_history(
            &self,
            messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let has_tool_results = messages
                .iter()
                .any(|msg| msg.role == "user" && msg.content.contains("[Tool results]"));
            if has_tool_results {
                Ok("BTC alias-tag flow resolved to final text output.".to_string())
            } else {
                Ok(tool_call_payload_with_alias_tag())
            }
        }
    }

    struct RawToolArtifactProvider;

    #[async_trait::async_trait]
    impl Provider for RawToolArtifactProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("fallback".to_string())
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(r#"{"name":"mock_price","parameters":{"symbol":"BTC"}}
{"result":{"symbol":"BTC","price_usd":65000}}
BTC is currently around $65,000 based on latest tool output."#
                .to_string())
        }
    }

    struct IterativeToolProvider {
        required_tool_iterations: usize,
    }

    impl IterativeToolProvider {
        fn completed_tool_iterations(messages: &[ChatMessage]) -> usize {
            messages
                .iter()
                .filter(|msg| msg.role == "user" && msg.content.contains("[Tool results]"))
                .count()
        }
    }

    #[async_trait::async_trait]
    impl Provider for IterativeToolProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(tool_call_payload())
        }

        async fn chat_with_history(
            &self,
            messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let completed_iterations = Self::completed_tool_iterations(messages);
            if completed_iterations >= self.required_tool_iterations {
                Ok(format!(
                    "Completed after {completed_iterations} tool iterations."
                ))
            } else {
                Ok(tool_call_payload())
            }
        }
    }

    #[derive(Default)]
    struct HistoryCaptureProvider {
        calls: std::sync::Mutex<Vec<Vec<(String, String)>>>,
        system_response: Option<String>,
    }

    #[async_trait::async_trait]
    impl Provider for HistoryCaptureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(self
                .system_response
                .clone()
                .unwrap_or_else(|| "fallback".to_string()))
        }

        async fn chat_with_history(
            &self,
            messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let snapshot = messages
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect::<Vec<_>>();
            let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
            calls.push(snapshot);
            Ok(format!("response-{}", calls.len()))
        }
    }

    struct StructuredRecoveryProvider {
        intent_response: String,
        classifier_response: String,
        history_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for StructuredRecoveryProvider {
        async fn chat_with_system(
            &self,
            system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            if system_prompt.is_some_and(|prompt| prompt.contains("turn-intent classifier")) {
                Ok(self.intent_response.clone())
            } else {
                Ok(self.classifier_response.clone())
            }
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            self.history_calls.fetch_add(1, Ordering::SeqCst);
            Ok("unexpected-main-history-call".to_string())
        }
    }

    struct DelayedHistoryCaptureProvider {
        delay: Duration,
        calls: std::sync::Mutex<Vec<Vec<(String, String)>>>,
    }

    #[async_trait::async_trait]
    impl Provider for DelayedHistoryCaptureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("fallback".to_string())
        }

        async fn chat_with_history(
            &self,
            messages: &[ChatMessage],
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            let snapshot = messages
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect::<Vec<_>>();
            let call_index = {
                let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
                calls.push(snapshot);
                calls.len()
            };
            tokio::time::sleep(self.delay).await;
            Ok(format!("response-{call_index}"))
        }
    }

    struct MockPriceTool;

    #[derive(Default)]
    struct ModelCaptureProvider {
        call_count: AtomicUsize,
        models: std::sync::Mutex<Vec<String>>,
        response: String,
    }

    #[async_trait::async_trait]
    impl Provider for ModelCaptureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(self.response.clone())
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(model.to_string());
            Ok(self.response.clone())
        }
    }

    #[async_trait::async_trait]
    impl Tool for MockPriceTool {
        fn name(&self) -> &str {
            "mock_price"
        }

        fn description(&self) -> &str {
            "Return a mocked BTC price"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string" }
                },
                "required": ["symbol"]
            })
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let symbol = args.get("symbol").and_then(serde_json::Value::as_str);
            if symbol != Some("BTC") {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("unexpected symbol".to_string()),
                });
            }

            Ok(ToolResult {
                success: true,
                output: r#"{"symbol":"BTC","price_usd":65000}"#.to_string(),
                error: None,
            })
        }
    }

    struct MockEchoTool;

    struct SlowMockPriceTool {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Tool for MockEchoTool {
        fn name(&self) -> &str {
            "mock_echo"
        }

        fn description(&self) -> &str {
            "Echo back the input text"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                }
            })
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                success: true,
                output: args
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl Tool for SlowMockPriceTool {
        fn name(&self) -> &str {
            "mock_price"
        }

        fn description(&self) -> &str {
            "Return a mocked BTC price after a delay"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            MockPriceTool.parameters_schema()
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            tokio::time::sleep(self.delay).await;
            MockPriceTool.execute(args).await
        }
    }

    #[test]
    fn build_runtime_tool_visibility_prompt_respects_excluded_snapshot() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(MockPriceTool), Box::new(MockEchoTool)];
        let excluded = vec!["mock_price".to_string()];

        let non_native = build_runtime_tool_visibility_prompt(&tools, &excluded, false);
        assert!(non_native.contains("Runtime Tool Availability (Authoritative)"));
        assert!(non_native.contains("Excluded by runtime policy: mock_price"));
        assert!(non_native.contains("`mock_echo`"));
        assert!(!non_native.contains("**mock_price**:"));
        assert!(non_native.contains("Do not claim tools are unavailable"));
        assert!(non_native.contains("## Tool Use Protocol"));

        let native = build_runtime_tool_visibility_prompt(&tools, &excluded, true);
        assert!(native.contains("Runtime Tool Availability (Authoritative)"));
        assert!(native.contains("Do not claim tools are unavailable"));
        assert!(native.contains("native provider function-calling"));
        assert!(!native.contains("## Tool Use Protocol"));
    }

    fn autonomy_with_mock_price_auto_approve() -> crate::config::AutonomyConfig {
        let mut autonomy = crate::config::AutonomyConfig::default();
        autonomy.auto_approve.push("mock_price".to_string());
        autonomy
    }

    #[tokio::test]
    async fn process_channel_message_injects_runtime_tool_visibility_prompt() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("openrouter".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool), Box::new(MockEchoTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(vec!["mock_price".to_string()])),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-runtime-visibility-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-runtime-visibility".to_string(),
                content: "hello tool visibility".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        {
            let calls = provider_impl
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            assert_eq!(calls.len(), 1);
            let first_call = &calls[0];
            assert!(!first_call.is_empty());
            assert_eq!(first_call[0].0, "system");
            let system_prompt = &first_call[0].1;
            assert!(system_prompt.contains("Runtime Tool Availability (Authoritative)"));
            assert!(system_prompt.contains("Excluded by runtime policy: mock_price"));
            assert!(system_prompt.contains("`mock_echo`"));
            assert!(!system_prompt.contains("**mock_price**:"));
        }

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("response-1"));
    }

    #[tokio::test]
    async fn process_channel_message_executes_tool_calls_instead_of_sending_raw_json() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-42".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-42:"));
        assert!(sent_messages[0].contains("BTC is currently around"));
        assert!(!sent_messages[0].contains("\"tool_calls\""));
        assert!(!sent_messages[0].contains("mock_price"));
    }

    #[tokio::test]
    async fn process_channel_message_telegram_does_not_persist_tool_summary_prefix() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-telegram-tool-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("BTC is currently around"));

        let histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let turns = histories
            .get("telegram_alice")
            .expect("telegram history should be stored");
        let assistant_turn = turns
            .iter()
            .rev()
            .find(|turn| turn.role == "assistant")
            .expect("assistant turn should be present");
        assert!(
            !assistant_turn.content.contains("[Used tools:"),
            "telegram history should not persist tool-summary prefix"
        );
    }

    #[tokio::test]
    async fn process_channel_message_streaming_hides_internal_progress_by_default() {
        let channel_impl = Arc::new(DraftStreamingRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-stream-hide".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-stream".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "draft-streaming-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let updates = channel_impl.draft_updates.lock().await;
        assert!(
            !updates.iter().any(|entry| {
                entry.contains("Got 1 tool call(s)")
                    || entry.contains("Thinking")
                    || entry.contains("⏳")
            }),
            "raw internal tool progress should stay hidden by default, got updates: {updates:?}"
        );
        drop(updates);

        let finalized = channel_impl.finalized_drafts.lock().await;
        assert_eq!(finalized.len(), 1);
        assert!(finalized[0].contains("BTC is currently around"));
    }

    #[tokio::test]
    async fn process_channel_message_streaming_shows_internal_progress_on_explicit_request() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(DraftStreamingRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-stream-show".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-stream".to_string(),
                content: "Please show commands and tool calls you used.".to_string(),
                channel: "draft-streaming-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let updates = channel_impl.draft_updates.lock().await;
        assert!(
            updates
                .iter()
                .any(|entry| entry.contains("Got 1 tool call(s)")),
            "explicit requests should expose internal progress details, got updates: {updates:?}"
        );
        assert!(
            updates.iter().any(|entry| entry.contains("Thinking")),
            "explicit requests should expose internal thinking/progress text, got updates: {updates:?}"
        );
    }

    #[tokio::test]
    async fn process_channel_message_streaming_does_not_seed_placeholder_draft() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(DraftStreamingRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-stream-no-placeholder".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-stream".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "draft-streaming-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert!(
            sent_messages
                .iter()
                .filter(|entry| entry.starts_with("draft:chat-stream:"))
                .all(|entry| !entry.ends_with(":...")),
            "streaming should not seed placeholder drafts, got sent messages: {sent_messages:?}"
        );
    }

    #[tokio::test]
    async fn process_channel_message_strips_unexecuted_tool_json_artifacts_from_reply() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(RawToolArtifactProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-raw-json".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-raw".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 3,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-raw:"));
        assert!(sent_messages[0].contains("BTC is currently around"));
        assert!(!sent_messages[0].contains("\"name\":\"mock_price\""));
        assert!(!sent_messages[0].contains("\"result\""));
    }

    #[tokio::test]
    async fn process_channel_message_executes_tool_calls_with_alias_tags() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingAliasProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-2".to_string(),
                sender: "bob".to_string(),
                reply_target: "chat-84".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-84:"));
        assert!(sent_messages[0].contains("alias-tag flow resolved"));
        assert!(!sent_messages[0].contains("<toolcall>"));
        assert!(!sent_messages[0].contains("mock_price"));
    }

    #[tokio::test]
    async fn process_channel_message_handles_models_command_without_llm_call() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let default_provider_impl = Arc::new(ModelCaptureProvider::default());
        let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
        let fallback_provider_impl = Arc::new(ModelCaptureProvider::default());
        let fallback_provider: Arc<dyn Provider> = fallback_provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
        provider_cache_seed.insert("openrouter".to_string(), fallback_provider);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&default_provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-cmd-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/models openrouter".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Provider switched to `openrouter`"));

        let route_key = "telegram_alice";
        let route = runtime_ctx
            .route_overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(route_key)
            .cloned()
            .expect("route should be stored for sender");
        assert_eq!(route.provider, "openrouter");
        assert_eq!(route.model, "default-model");

        assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_provider_impl.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_channel_message_handles_approve_command_without_llm_call() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm;
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            non_cli_natural_language_approval_mode:
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("openrouter".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });
        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("telegram"),
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm
        );
        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("telegram"),
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm
        );

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-approve-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/approve mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Approved supervised execution for `mock_price`"));
        assert!(sent[0].contains("including after restart"));

        assert!(runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(!runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(
            saved
                .autonomy
                .auto_approve
                .iter()
                .any(|tool| tool == "mock_price"),
            "persisted config should include mock_price in autonomy.auto_approve"
        );
        assert!(
            saved
                .autonomy
                .always_ask
                .iter()
                .all(|tool| tool != "mock_price"),
            "persisted config should remove mock_price from autonomy.always_ask"
        );
    }

    #[tokio::test]
    async fn process_channel_message_denies_approval_management_for_unlisted_sender() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.autonomy.non_cli_approval_approvers = vec!["alice".to_string()];
        persisted
            .autonomy
            .non_cli_natural_language_approval_mode_by_channel
            .insert(
                "telegram".to_string(),
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            );
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            non_cli_approval_approvers: vec!["alice".to_string()],
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });
        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("telegram"),
            crate::config::NonCliNaturalLanguageApprovalMode::Direct
        );

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-approve-denied-1".to_string(),
                sender: "bob".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/approve mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Approval-management command denied"));
        assert!(sent[0].contains("Allowed approvers: alice"));
        assert!(!runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(
            saved
                .autonomy
                .auto_approve
                .iter()
                .all(|tool| tool != "mock_price"),
            "persisted config should not include unauthorized approval changes"
        );
    }

    #[tokio::test]
    async fn process_channel_message_handles_unapprove_command_without_llm_call() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.auto_approve = vec!["mock_price".to_string()];
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            auto_approve: vec!["mock_price".to_string()],
            ..crate::config::AutonomyConfig::default()
        };
        let approval_manager = Arc::new(ApprovalManager::from_config(&autonomy_cfg));
        approval_manager.grant_non_cli_session("mock_price");

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager,
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-unapprove-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/unapprove mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Persistent approval removed for `mock_price`: yes."));
        assert!(sent[0].contains("Runtime session grant removed: yes"));
        assert!(!runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(
            saved
                .autonomy
                .auto_approve
                .iter()
                .all(|tool| tool != "mock_price"),
            "persisted config should remove mock_price from autonomy.auto_approve"
        );
    }

    #[tokio::test]
    async fn process_channel_message_handles_approvals_command_without_llm_call() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.auto_approve = vec!["mock_price".to_string()];
        persisted.autonomy.always_ask = vec!["shell".to_string()];
        persisted.autonomy.non_cli_excluded_tools = vec!["shell".to_string()];
        persisted.save().await.expect("save config");

        let approval_manager = Arc::new(ApprovalManager::from_config(
            &crate::config::AutonomyConfig::default(),
        ));
        approval_manager.grant_non_cli_session("shell");
        approval_manager.grant_non_cli_allow_all_once();

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(vec!["shell".to_string()])),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager,
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-approvals-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/approvals".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Supervised non-CLI tool approvals:"));
        assert!(sent[0].contains("Runtime session grants: shell"));
        assert!(sent[0].contains("Runtime one-time all-tools bypass tokens: 1"));
        assert!(sent[0].contains("Runtime non_cli_approval_approvers:"));
        assert!(sent[0].contains("Runtime non_cli_natural_language_approval_mode:"));
        assert!(sent[0].contains("Runtime non_cli_natural_language_approval_mode_by_channel:"));
        assert!(sent[0].contains("Runtime non_cli_excluded_tools: shell"));
        assert!(sent[0].contains("Persisted autonomy.auto_approve: mock_price"));
        assert!(sent[0].contains("Persisted autonomy.always_ask: shell"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_channel_message_natural_request_then_confirm_approval() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm;
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            non_cli_natural_language_approval_mode:
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-req-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "授权工具 mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let request_id = {
            let sent = channel_impl.sent_messages.lock().await;
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0].contains("Approval request created."),
                "unexpected response: {}",
                sent[0]
            );
            let request_line = sent[0]
                .lines()
                .find(|line| line.starts_with("Request ID: `"))
                .expect("request line");
            request_line
                .trim_start_matches("Request ID: `")
                .trim_end_matches('`')
                .to_string()
        };
        assert!(request_id.starts_with("apr-"));

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-req-2".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: format!("确认授权 {request_id}"),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[1].contains("Approved supervised execution for `mock_price` from request"));
        assert!(runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(!runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert!(runtime_ctx
            .approval_manager
            .list_non_cli_pending_requests(Some("alice"), Some("telegram"), Some("chat-1"))
            .is_empty());
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(saved
            .autonomy
            .auto_approve
            .iter()
            .any(|tool| tool == "mock_price"));
    }

    #[tokio::test]
    async fn process_channel_message_confirm_auto_resumes_original_request() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let provider: Arc<dyn Provider> = Arc::new(ToolCallingProvider);
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir.clone();
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.save().await.expect("save config");

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider,
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(workspace_dir),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["mock_price".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });
        runtime_ctx
            .route_overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                "telegram_alice".to_string(),
                ChannelRouteSelection {
                    provider: "test-provider".to_string(),
                    model: "test-model".to_string(),
                },
            );

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-auto-resume-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let request_id = {
            let sent = channel_impl.sent_messages.lock().await;
            assert_eq!(sent.len(), 1);
            assert!(sent[0].contains("Approval required for current execution plan."));
            assert!(sent[0].contains("`mock_price`"));
            let request_line = sent[0]
                .lines()
                .find(|line| line.starts_with("Request ID: `"))
                .expect("request line");
            request_line
                .trim_start_matches("Request ID: `")
                .trim_end_matches('`')
                .to_string()
        };

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-auto-resume-2".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: format!("/approve-confirm {request_id}"),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let sent = channel_impl.sent_messages.lock().await;
                if sent
                    .iter()
                    .any(|entry| entry.contains("BTC is currently around $65,000"))
                {
                    break;
                }
                drop(sent);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("auto-resumed request should finish");

        let sent = channel_impl.sent_messages.lock().await;
        assert!(sent
            .iter()
            .any(|entry| entry
                .contains("BTC is currently around $65,000 based on latest tool output.")));
    }

    #[tokio::test]
    async fn process_channel_message_all_tools_once_requires_confirm_and_stays_runtime_only() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm;
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            non_cli_natural_language_approval_mode:
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-all-once-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "请一次性允许所有工具和命令".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let request_id = {
            let sent = channel_impl.sent_messages.lock().await;
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0].contains("One-time all-tools approval request created."),
                "unexpected response: {}",
                sent[0]
            );
            let request_line = sent[0]
                .lines()
                .find(|line| line.starts_with("Request ID: `"))
                .expect("request line");
            request_line
                .trim_start_matches("Request ID: `")
                .trim_end_matches('`')
                .to_string()
        };
        assert!(request_id.starts_with("apr-"));

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-all-once-2".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: format!("/approve-confirm {request_id}"),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[1].contains("Approved one-time all-tools bypass from request"));
        assert!(sent[1].contains("does not persist to config"));
        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_allow_all_once_remaining(),
            1
        );
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(
            saved
                .autonomy
                .auto_approve
                .iter()
                .all(|tool| tool != APPROVAL_ALL_TOOLS_ONCE_TOKEN && tool != "mock_price"),
            "persisted config should not persist one-time bypass markers or promote mock_price"
        );
        assert!(
            saved
                .autonomy
                .always_ask
                .iter()
                .any(|tool| tool == "mock_price"),
            "persisted config should keep existing always_ask entries untouched"
        );
    }

    #[tokio::test]
    async fn process_channel_message_natural_approval_direct_mode_grants_immediately() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-direct-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "授权工具 mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("Approved supervised execution for `mock_price`."));
        assert!(sent[0].contains("Runtime pending requests cleared: 0."));
        assert!(runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(!runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert!(runtime_ctx
            .approval_manager
            .list_non_cli_pending_requests(Some("alice"), Some("telegram"), Some("chat-1"))
            .is_empty());
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(saved
            .autonomy
            .auto_approve
            .iter()
            .any(|tool| tool == "mock_price"));
    }

    #[tokio::test]
    async fn process_channel_message_natural_approval_honors_channel_mode_override() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted
            .autonomy
            .non_cli_natural_language_approval_mode_by_channel
            .insert(
                "telegram".to_string(),
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            );
        persisted.save().await.expect("save config");

        let mut autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            ..crate::config::AutonomyConfig::default()
        };
        autonomy_cfg
            .non_cli_natural_language_approval_mode_by_channel
            .insert(
                "telegram".to_string(),
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            );

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-direct-override-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "授权工具 mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].contains("Approval request created."),
            "unexpected response: {}",
            sent[0]
        );
        assert!(!runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_channel_message_natural_approval_can_be_disabled_but_slash_still_works() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let mut persisted = Config::default();
        persisted.config_path = config_path.clone();
        persisted.workspace_dir = workspace_dir;
        persisted.autonomy.always_ask = vec!["mock_price".to_string()];
        persisted.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::Disabled;
        persisted.save().await.expect("save config");

        let autonomy_cfg = crate::config::AutonomyConfig {
            always_ask: vec!["mock_price".to_string()],
            non_cli_natural_language_approval_mode:
                crate::config::NonCliNaturalLanguageApprovalMode::Disabled,
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-nl-disabled-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "授权工具 mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        {
            let sent = channel_impl.sent_messages.lock().await;
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0].contains("Natural-language approval commands are disabled"),
                "unexpected response: {}",
                sent[0]
            );
        }
        assert!(!runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(runtime_ctx.approval_manager.needs_approval("mock_price"));

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-nl-disabled-2".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "/approve mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[1].contains("Approved supervised execution for `mock_price`."));
        assert!(runtime_ctx
            .approval_manager
            .is_non_cli_session_granted("mock_price"));
        assert!(!runtime_ctx.approval_manager.needs_approval("mock_price"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let saved_raw = tokio::fs::read_to_string(&config_path)
            .await
            .expect("read persisted config");
        let saved: Config = toml::from_str(&saved_raw).expect("parse persisted config");
        assert!(saved
            .autonomy
            .auto_approve
            .iter()
            .any(|tool| tool == "mock_price"));
    }

    #[tokio::test]
    async fn process_channel_message_confirm_rejects_sender_mismatch() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let autonomy_cfg = crate::config::AutonomyConfig {
            non_cli_natural_language_approval_mode:
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            ..crate::config::AutonomyConfig::default()
        };

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-mismatch-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "授权工具 mock_price".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let request_id = {
            let sent = channel_impl.sent_messages.lock().await;
            assert_eq!(sent.len(), 1);
            let request_line = sent[0]
                .lines()
                .find(|line| line.starts_with("Request ID: `"))
                .expect("request line");
            request_line
                .trim_start_matches("Request ID: `")
                .trim_end_matches('`')
                .to_string()
        };

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-mismatch-2".to_string(),
                sender: "bob".to_string(),
                reply_target: "chat-1".to_string(),
                content: format!("confirm {request_id}"),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[1].contains("can only be confirmed by the same sender"));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 0);

        let pending = runtime_ctx.approval_manager.list_non_cli_pending_requests(
            Some("alice"),
            Some("telegram"),
            Some("chat-1"),
        );
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id, request_id);
    }

    #[tokio::test]
    async fn process_channel_message_uses_route_override_provider_and_model() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let default_provider_impl = Arc::new(ModelCaptureProvider::default());
        let default_provider: Arc<dyn Provider> = default_provider_impl.clone();
        let routed_provider_impl = Arc::new(ModelCaptureProvider::default());
        let routed_provider: Arc<dyn Provider> = routed_provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&default_provider));
        provider_cache_seed.insert("openrouter".to_string(), routed_provider);

        let route_key = "telegram_alice".to_string();
        let mut route_overrides = HashMap::new();
        route_overrides.insert(
            route_key,
            ChannelRouteSelection {
                provider: "openrouter".to_string(),
                model: "route-model".to_string(),
            },
        );

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&default_provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(route_overrides)),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-routed-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "hello routed provider".to_string(),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
        assert_eq!(routed_provider_impl.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            routed_provider_impl
                .models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_slice(),
            &["route-model".to_string()]
        );
    }

    #[tokio::test]
    async fn process_channel_message_prefers_cached_default_provider_instance() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let startup_provider_impl = Arc::new(ModelCaptureProvider::default());
        let startup_provider: Arc<dyn Provider> = startup_provider_impl.clone();
        let reloaded_provider_impl = Arc::new(ModelCaptureProvider::default());
        let reloaded_provider: Arc<dyn Provider> = reloaded_provider_impl.clone();

        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), reloaded_provider);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&startup_provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-default-provider-cache".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "hello cached default provider".to_string(),
                channel: "telegram".to_string(),
                timestamp: 3,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        assert_eq!(startup_provider_impl.call_count.load(Ordering::SeqCst), 0);
        assert_eq!(reloaded_provider_impl.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_channel_message_uses_runtime_default_model_from_store() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider::default());
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");

        {
            let mut store = runtime_config_store()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            store.insert(
                config_path.clone(),
                RuntimeConfigState {
                    defaults: ChannelRuntimeDefaults {
                        default_provider: "test-provider".to_string(),
                        model: "hot-reloaded-model".to_string(),
                        temperature: 0.5,
                        api_key: None,
                        api_url: None,
                        reliability: crate::config::ReliabilityConfig::default(),
                    },
                    last_applied_stamp: None,
                },
            );
        }

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("startup-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-runtime-store-model".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "hello runtime defaults".to_string(),
                channel: "telegram".to_string(),
                timestamp: 4,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        {
            let mut store = runtime_config_store()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            store.remove(&config_path);
        }

        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            provider_impl
                .models
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_slice(),
            &["hot-reloaded-model".to_string()]
        );
    }

    #[tokio::test]
    async fn load_runtime_defaults_from_config_file_includes_autonomy_policy() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

        let mut cfg = Config::default();
        cfg.config_path = config_path.clone();
        cfg.workspace_dir = workspace_dir;
        cfg.default_provider = Some("test-provider".to_string());
        cfg.default_model = Some("test-model".to_string());
        cfg.autonomy.auto_approve = vec!["mock_price".to_string()];
        cfg.autonomy.always_ask = vec!["shell".to_string()];
        cfg.autonomy.non_cli_excluded_tools = vec!["browser_open".to_string()];
        cfg.autonomy.non_cli_approval_approvers = vec!["telegram:alice".to_string()];
        cfg.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::Direct;
        cfg.autonomy
            .non_cli_natural_language_approval_mode_by_channel
            .insert(
                "telegram".to_string(),
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            );
        cfg.save().await.expect("save config");

        let (_defaults, policy) = load_runtime_defaults_from_config_file(&config_path)
            .await
            .expect("load runtime state");

        assert_eq!(policy.auto_approve, vec!["mock_price".to_string()]);
        assert_eq!(policy.always_ask, vec!["shell".to_string()]);
        assert_eq!(
            policy.non_cli_excluded_tools,
            vec!["browser_open".to_string()]
        );
        assert_eq!(
            policy.non_cli_approval_approvers,
            vec!["telegram:alice".to_string()]
        );
        assert_eq!(
            policy.non_cli_natural_language_approval_mode,
            crate::config::NonCliNaturalLanguageApprovalMode::Direct
        );
        assert_eq!(
            policy
                .non_cli_natural_language_approval_mode_by_channel
                .get("telegram")
                .copied(),
            Some(crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm)
        );
    }

    #[tokio::test]
    async fn maybe_apply_runtime_config_update_refreshes_autonomy_policy_and_excluded_tools() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

        let mut cfg = Config::default();
        cfg.config_path = config_path.clone();
        cfg.workspace_dir = workspace_dir;
        cfg.default_provider = Some("ollama".to_string());
        cfg.default_model = Some("llama3.2".to_string());
        cfg.api_key = Some("http://127.0.0.1:11434".to_string());
        cfg.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::Direct;
        cfg.autonomy.non_cli_excluded_tools = vec!["shell".to_string()];
        cfg.save().await.expect("save initial config");

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(ModelCaptureProvider::default()),
            default_provider: Arc::new("ollama".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("llama3.2".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: Some("http://127.0.0.1:11434".to_string()),
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions {
                topclaw_dir: Some(temp.path().to_path_buf()),
                ..providers::ProviderRuntimeOptions::default()
            },
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        maybe_apply_runtime_config_update(runtime_ctx.as_ref())
            .await
            .expect("apply initial config");

        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("telegram"),
            crate::config::NonCliNaturalLanguageApprovalMode::Direct
        );
        assert_eq!(
            snapshot_non_cli_excluded_tools(runtime_ctx.as_ref()),
            vec!["shell".to_string()]
        );

        cfg.autonomy.non_cli_natural_language_approval_mode =
            crate::config::NonCliNaturalLanguageApprovalMode::Disabled;
        cfg.autonomy
            .non_cli_natural_language_approval_mode_by_channel
            .insert(
                "telegram".to_string(),
                crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm,
            );
        cfg.autonomy.non_cli_excluded_tools =
            vec!["browser_open".to_string(), "mock_price".to_string()];
        cfg.save().await.expect("save updated config");

        maybe_apply_runtime_config_update(runtime_ctx.as_ref())
            .await
            .expect("apply updated config");

        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("telegram"),
            crate::config::NonCliNaturalLanguageApprovalMode::RequestConfirm
        );
        assert_eq!(
            runtime_ctx
                .approval_manager
                .non_cli_natural_language_approval_mode_for_channel("discord"),
            crate::config::NonCliNaturalLanguageApprovalMode::Disabled
        );
        assert_eq!(
            snapshot_non_cli_excluded_tools(runtime_ctx.as_ref()),
            vec!["browser_open".to_string(), "mock_price".to_string()]
        );

        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.remove(&config_path);
    }

    #[tokio::test]
    async fn start_channels_uses_model_routes_when_global_provider_key_is_missing() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");

        let mut cfg = Config::default();
        cfg.workspace_dir = workspace_dir;
        cfg.config_path = temp.path().join("config.toml");
        cfg.default_provider = None;
        cfg.api_key = None;
        cfg.default_model = Some("hint:fast".to_string());
        cfg.model_routes = vec![crate::config::ModelRouteConfig {
            hint: "fast".to_string(),
            provider: "openai-codex".to_string(),
            model: "gpt-5.3-codex".to_string(),
            max_tokens: Some(512),
            api_key: Some("route-specific-key".to_string()),
        }];

        let config_path = cfg.config_path.clone();
        let result = Box::pin(start_channels(cfg)).await;
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.remove(&config_path);

        assert!(
            result.is_ok(),
            "start_channels should support routed providers without global credentials: {result:?}"
        );
    }

    #[tokio::test]
    async fn process_channel_message_respects_configured_max_tool_iterations_above_default() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();
        let mut autonomy_cfg = autonomy_with_mock_price_auto_approve();
        autonomy_cfg.level = crate::security::AutonomyLevel::Full;

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(IterativeToolProvider {
                required_tool_iterations: 11,
            }),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 12,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: 5,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-iter-success".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-iter-success".to_string(),
                content: "Loop until done".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-iter-success:"));
        assert!(sent_messages[0].contains("Completed after 11 tool iterations."));
        assert!(!sent_messages[0].contains("⚠️ Error:"));
    }

    #[tokio::test]
    async fn process_channel_message_reports_configured_max_tool_iterations_limit() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();
        let mut autonomy_cfg = autonomy_with_mock_price_auto_approve();
        autonomy_cfg.level = crate::security::AutonomyLevel::Full;

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(IterativeToolProvider {
                required_tool_iterations: 20,
            }),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 3,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: 5,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(&autonomy_cfg)),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-iter-fail".to_string(),
                sender: "bob".to_string(),
                reply_target: "chat-iter-fail".to_string(),
                content: "Loop forever".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-iter-fail:"));
        assert!(sent_messages[0].contains("⚠️ Reached tool-iteration limit (3)"));
        assert!(sent_messages[0].contains("Context and progress were preserved"));
    }

    #[tokio::test]
    async fn process_channel_message_hides_aggregated_provider_attempt_dump() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(AggregatedFailureProvider),
            default_provider: Arc::new("telegram".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: 5,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-provider-error".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-provider-error".to_string(),
                content: "tell me something useful".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("provider returned an unreadable response"));
        assert!(!sent_messages[0].contains("All providers/models failed"));
        assert!(!sent_messages[0].contains("provider=openrouter"));
        assert!(!sent_messages[0].contains("attempt 1/3"));
    }

    #[tokio::test]
    async fn process_channel_message_answers_model_question_via_provider_when_available() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider {
            response: "I'm using the MiniMax route right now.".to_string(),
            ..Default::default()
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("openrouter".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::from([(
                "telegram_alice".to_string(),
                ChannelRouteSelection {
                    provider: "openrouter".to_string(),
                    model: "minimax/minimax-m2.7".to_string(),
                },
            )]))),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-model-local".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-model-local".to_string(),
                content: "which model specifically?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].ends_with("I'm using the MiniMax route right now."));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_channel_message_answers_skills_question_via_provider_when_available() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(ModelCaptureProvider {
            response: "I can inspect code, browse carefully, and analyze local files.".to_string(),
            ..Default::default()
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();

        let system_prompt = r#"
<available_skills>
  <skill>
    <name>find-skills</name>
    <description>Discover installable skills.</description>
  </skill>
  <skill>
    <name>safe-web-search</name>
    <description>Browse safely.</description>
    <tools>
      <tool>
        <name>web_fetch</name>
      </tool>
    </tools>
  </skill>
</available_skills>
"#;

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new(system_prompt.to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-skills-local".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-skills-local".to_string(),
                content: "which skills do you have?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0]
            .ends_with("I can inspect code, browse carefully, and analyze local files."));
        assert_eq!(provider_impl.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_channel_message_falls_back_to_local_model_answer_on_provider_error() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider: Arc<dyn Provider> = Arc::new(AggregatedFailureProvider);
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("openrouter".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::from([(
                "telegram_alice".to_string(),
                ChannelRouteSelection {
                    provider: "openrouter".to_string(),
                    model: "minimax/minimax-m2.7".to_string(),
                },
            )]))),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-model-fallback".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-model-fallback".to_string(),
                content: "which model are you using now?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].ends_with(
            "I'm currently using provider `openrouter` with model `minimax/minimax-m2.7`."
        ));
    }

    #[tokio::test]
    async fn process_channel_message_falls_back_to_local_skills_answer_on_provider_error() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider: Arc<dyn Provider> = Arc::new(AggregatedFailureProvider);

        let system_prompt = r#"
<available_skills>
  <skill>
    <name>find-skills</name>
    <description>Discover installable skills.</description>
  </skill>
  <skill>
    <name>safe-web-search</name>
    <description>Browse safely.</description>
    <tools>
      <tool>
        <name>web_fetch</name>
      </tool>
    </tools>
  </skill>
</available_skills>
"#;

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new(system_prompt.to_string()),
            model: Arc::new("default-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-skills-fallback".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-skills-fallback".to_string(),
                content: "which skills do you have?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("I currently have 2 advertised skills loaded:"));
        assert!(sent_messages[0].contains("`find-skills`"));
        assert!(sent_messages[0].contains("`safe-web-search`"));
    }

    #[tokio::test]
    async fn process_channel_message_model_judged_direct_reply_suppresses_tools_for_turn() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider {
            system_response: Some(
                r#"{"intent":"direct_reply","reason":"exploratory discussion, not an execution request"}"#
                    .to_string(),
            ),
            ..Default::default()
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(NamedTestTool("shell"))]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["shell".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-direct-reply".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-direct-reply".to_string(),
                content: "I'm trying to make you smarter by changing your codebase.".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("response-1"));
        assert!(!sent_messages[0].contains("approve-confirm"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 1);
        let system_prompt = &calls[0][0].1;
        assert!(system_prompt
            .contains("Turn-intent policy: This turn is best handled as a direct reply"));
        assert!(system_prompt.contains("Allowed tools: (none)"));
        assert!(system_prompt.contains("No tool calls are allowed in this turn"));
        assert!(!system_prompt.contains("`shell`"));
    }

    #[tokio::test]
    async fn process_channel_message_model_judged_clarification_suppresses_tools_for_turn() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider {
            system_response: Some(
                r#"{"intent":"needs_clarification","reason":"missing target repo and desired change"}"#
                    .to_string(),
            ),
            ..Default::default()
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(NamedTestTool("shell"))]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["shell".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-needs-clarification".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-needs-clarification".to_string(),
                content: "Help me update my repo so it is smarter.".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("response-1"));
        assert!(!sent_messages[0].contains("approve-confirm"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 1);
        let system_prompt = &calls[0][0].1;
        assert!(system_prompt
            .contains("The user likely wants action, but this request is underspecified"));
        assert!(system_prompt.contains("Allowed tools: (none)"));
        assert!(system_prompt.contains("No tool calls are allowed in this turn"));
        assert!(!system_prompt.contains("`shell`"));
    }

    #[tokio::test]
    async fn process_channel_message_model_judged_needs_tools_keeps_approval_flow_active() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider {
            system_response: Some(
                r#"{"intent":"needs_tools","reason":"explicit shell execution request"}"#
                    .to_string(),
            ),
            ..Default::default()
        });
        let provider: Arc<dyn Provider> = provider_impl.clone();
        let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        provider_cache_seed.insert("test-provider".to_string(), Arc::clone(&provider));

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::clone(&provider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(NamedTestTool("shell"))]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["shell".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-needs-tools".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-needs-tools".to_string(),
                content: "please run cargo test".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].contains("supervised access to `shell`"));
        assert!(sent_messages[0].contains("/approve-confirm"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            calls.is_empty(),
            "main history should not run before approval"
        );
    }

    struct NoopMemory;

    #[async_trait::async_trait]
    impl Memory for NoopMemory {
        fn name(&self) -> &str {
            "noop"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: crate::memory::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::memory::MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<crate::memory::MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&crate::memory::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::memory::MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    struct RecallMemory;

    #[async_trait::async_trait]
    impl Memory for RecallMemory {
        fn name(&self) -> &str {
            "recall-memory"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: crate::memory::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::memory::MemoryEntry>> {
            Ok(vec![crate::memory::MemoryEntry {
                id: "entry-1".to_string(),
                key: "memory_key_1".to_string(),
                content: "Age is 45".to_string(),
                category: crate::memory::MemoryCategory::Conversation,
                timestamp: "2026-02-20T00:00:00Z".to_string(),
                session_id: None,
                score: Some(0.9),
            }])
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<crate::memory::MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&crate::memory::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::memory::MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(1)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn message_dispatch_processes_messages_in_parallel() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();
        let provider_impl = Arc::new(ParallelProbeProvider::new(Duration::from_millis(250)));

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(4);
        tx.send(traits::ChannelMessage {
            id: "1".to_string(),
            sender: "alice".to_string(),
            reply_target: "alice".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
        })
        .await
        .unwrap();
        tx.send(traits::ChannelMessage {
            id: "2".to_string(),
            sender: "bob".to_string(),
            reply_target: "bob".to_string(),
            content: "world".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
        })
        .await
        .unwrap();
        drop(tx);

        run_message_dispatch_loop(rx, runtime_ctx, 2).await;

        assert!(
            provider_impl.max_active_calls() >= 2,
            "expected overlapping provider calls, observed max concurrency {}",
            provider_impl.max_active_calls()
        );

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 2);
    }

    #[tokio::test]
    async fn message_dispatch_interrupts_in_flight_telegram_request_and_preserves_context() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(DelayedHistoryCaptureProvider {
            delay: Duration::from_millis(250),
            calls: std::sync::Mutex::new(Vec::new()),
        });

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: true,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(8);
        let send_task = tokio::spawn(async move {
            tx.send(traits::ChannelMessage {
                id: "msg-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "forwarded content".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
            tx.send(traits::ChannelMessage {
                id: "msg-2".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "summarize this".to_string(),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            })
            .await
            .unwrap();
        });

        run_message_dispatch_loop(rx, runtime_ctx, 4).await;
        send_task.await.unwrap();

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 1);
        assert!(sent_messages[0].starts_with("chat-1:"));
        assert!(sent_messages[0].contains("response-2"));
        drop(sent_messages);

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 2);
        let second_call = &calls[1];
        assert!(second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("forwarded content") }));
        assert!(second_call
            .iter()
            .any(|(role, content)| { role == "user" && content.contains("summarize this") }));
        assert!(
            !second_call.iter().any(|(role, _)| role == "assistant"),
            "cancelled turn should not persist an assistant response"
        );
    }

    #[tokio::test]
    async fn message_dispatch_interrupt_scope_is_same_sender_same_chat() {
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(SlowProvider {
                delay: Duration::from_millis(180),
            }),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: true,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(8);
        let send_task = tokio::spawn(async move {
            tx.send(traits::ChannelMessage {
                id: "msg-a".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "first chat".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            tx.send(traits::ChannelMessage {
                id: "msg-b".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-2".to_string(),
                content: "second chat".to_string(),
                channel: "telegram".to_string(),
                timestamp: 2,
                thread_ts: None,
            })
            .await
            .unwrap();
        });

        run_message_dispatch_loop(rx, runtime_ctx, 4).await;
        send_task.await.unwrap();

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert_eq!(sent_messages.len(), 2);
        assert!(sent_messages.iter().any(|msg| msg.starts_with("chat-1:")));
        assert!(sent_messages.iter().any(|msg| msg.starts_with("chat-2:")));
    }

    #[tokio::test]
    async fn process_channel_message_cancels_scoped_typing_task() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(SlowProvider {
                delay: Duration::from_millis(20),
            }),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "typing-msg".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-typing".to_string(),
                content: "hello".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let starts = channel_impl.start_typing_calls.load(Ordering::SeqCst);
        let stops = channel_impl.stop_typing_calls.load(Ordering::SeqCst);
        assert_eq!(starts, 1, "start_typing should be called once");
        assert_eq!(stops, 1, "stop_typing should be called once");
    }

    #[tokio::test]
    async fn process_channel_message_adds_and_swaps_reactions() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(SlowProvider {
                delay: Duration::from_millis(5),
            }),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "react-msg".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-react".to_string(),
                content: "hello".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let added = channel_impl.reactions_added.lock().await;
        assert!(
            added.len() >= 2,
            "expected at least 2 reactions added (\u{1F440} then \u{2705}), got {}",
            added.len()
        );
        assert_eq!(added[0].2, "\u{1F440}", "first reaction should be eyes");
        assert_eq!(
            added.last().unwrap().2,
            "\u{2705}",
            "last reaction should be checkmark"
        );

        let removed = channel_impl.reactions_removed.lock().await;
        assert_eq!(removed.len(), 1, "eyes reaction should be removed once");
        assert_eq!(removed[0].2, "\u{1F440}");
    }

    #[tokio::test]
    async fn process_channel_message_suppresses_heartbeat_ok_sentinel() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(HeartbeatOkProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "heartbeat-msg".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-heartbeat".to_string(),
                content: "hello".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        assert!(
            sent_messages.is_empty(),
            "HEARTBEAT_OK sentinel should not be sent as a channel reply"
        );
        drop(sent_messages);

        let histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let history_key = "test-channel_alice";
        let turns = histories
            .get(history_key)
            .expect("user turn should still be retained");
        assert_eq!(turns.len(), 1, "assistant sentinel should not be persisted");
        assert_eq!(turns[0].role, "user");
        assert!(
            turns[0].content.contains("hello"),
            "expected user content to retain original message"
        );
    }

    #[test]
    fn prompt_contains_all_sections() {
        let ws = make_workspace();
        let tools = vec![("shell", "Run commands"), ("file_read", "Read files")];
        let prompt = build_system_prompt(ws.path(), "test-model", &tools, &[], None, None);

        // Section headers
        assert!(prompt.contains("## Tools"), "missing Tools section");
        assert!(prompt.contains("## Safety"), "missing Safety section");
        assert!(prompt.contains("## Workspace"), "missing Workspace section");
        assert!(
            prompt.contains("## Project Context"),
            "missing Project Context"
        );
        assert!(
            prompt.contains("## Current Date & Time"),
            "missing Date/Time"
        );
        assert!(prompt.contains("## Runtime"), "missing Runtime section");
    }

    #[test]
    fn prompt_injects_tools() {
        let ws = make_workspace();
        let tools = vec![
            ("shell", "Run commands"),
            ("memory_recall", "Search memory"),
        ];
        let prompt = build_system_prompt(ws.path(), "gpt-4o", &tools, &[], None, None);

        assert!(prompt.contains("**shell**"));
        assert!(prompt.contains("Run commands"));
        assert!(prompt.contains("**memory_recall**"));
    }

    #[test]
    fn prompt_includes_single_tool_protocol_block_after_append() {
        let ws = make_workspace();
        let tools = vec![("shell", "Run commands")];
        let mut prompt = build_system_prompt(ws.path(), "gpt-4o", &tools, &[], None, None);

        assert!(
            !prompt.contains("## Tool Use Protocol"),
            "build_system_prompt should not emit protocol block directly"
        );

        prompt.push_str(&crate::agent::loop_::build_tool_instructions(&[]));

        assert_eq!(
            prompt.matches("## Tool Use Protocol").count(),
            1,
            "protocol block should appear exactly once in the final prompt"
        );
    }

    #[test]
    fn prompt_injects_safety() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(prompt.contains("Do not exfiltrate private data"));
        assert!(prompt.contains("Do not run destructive commands"));
        assert!(prompt.contains("Prefer `trash` over `rm`"));
    }

    #[test]
    fn prompt_injects_workspace_files() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(prompt.contains("### SOUL.md"), "missing SOUL.md header");
        assert!(prompt.contains("Be helpful"), "missing SOUL content");
        assert!(prompt.contains("### IDENTITY.md"), "missing IDENTITY.md");
        assert!(prompt.contains("Name: TopClaw"), "missing IDENTITY content");
        assert!(prompt.contains("### USER.md"), "missing USER.md");
        assert!(prompt.contains("### AGENTS.md"), "missing AGENTS.md");
        assert!(prompt.contains("### TOOLS.md"), "missing TOOLS.md");
        // HEARTBEAT.md is intentionally excluded from channel prompts — it's only
        // relevant to the heartbeat worker and causes LLMs to emit spurious
        // "HEARTBEAT_OK" acknowledgments in channel conversations.
        assert!(
            !prompt.contains("### HEARTBEAT.md"),
            "HEARTBEAT.md should not be in channel prompt"
        );
        assert!(prompt.contains("### MEMORY.md"), "missing MEMORY.md");
        assert!(prompt.contains("User likes Rust"), "missing MEMORY content");
    }

    #[test]
    fn prompt_missing_file_markers() {
        let tmp = TempDir::new().unwrap();
        // Empty workspace — no files at all
        let prompt = build_system_prompt(tmp.path(), "model", &[], &[], None, None);

        assert!(prompt.contains("[File not found: SOUL.md]"));
        assert!(prompt.contains("[File not found: AGENTS.md]"));
        assert!(prompt.contains("[File not found: IDENTITY.md]"));
    }

    #[test]
    fn prompt_bootstrap_only_if_exists() {
        let ws = make_workspace();
        // No BOOTSTRAP.md — should not appear
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);
        assert!(
            !prompt.contains("### BOOTSTRAP.md"),
            "BOOTSTRAP.md should not appear when missing"
        );

        // Create BOOTSTRAP.md — should appear
        std::fs::write(ws.path().join("BOOTSTRAP.md"), "# Bootstrap\nFirst run.").unwrap();
        let prompt2 = build_system_prompt(ws.path(), "model", &[], &[], None, None);
        assert!(
            prompt2.contains("### BOOTSTRAP.md"),
            "BOOTSTRAP.md should appear when present"
        );
        assert!(prompt2.contains("First run"));
    }

    #[test]
    fn prompt_no_daily_memory_injection() {
        let ws = make_workspace();
        let memory_dir = ws.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        std::fs::write(
            memory_dir.join(format!("{today}.md")),
            "# Daily\nSome note.",
        )
        .unwrap();

        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        // Daily notes should NOT be in the system prompt (on-demand via tools)
        assert!(
            !prompt.contains("Daily Notes"),
            "daily notes should not be auto-injected"
        );
        assert!(
            !prompt.contains("Some note"),
            "daily content should not be in prompt"
        );
    }

    #[test]
    fn prompt_runtime_metadata() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "claude-sonnet-4", &[], &[], None, None);

        assert!(prompt.contains("Model: claude-sonnet-4"));
        assert!(prompt.contains(&format!("OS: {}", std::env::consts::OS)));
        assert!(prompt.contains("Host:"));
    }

    #[test]
    fn prompt_skills_include_instructions_and_tools() {
        let ws = make_workspace();
        let skills = vec![crate::skills::Skill {
            name: "code-review".into(),
            description: "Review code for bugs".into(),
            version: "1.0.0".into(),
            author: None,
            tags: vec![],
            tools: vec![crate::skills::SkillTool {
                name: "lint".into(),
                description: "Run static checks".into(),
                kind: "shell".into(),
                command: "cargo clippy".into(),
                args: HashMap::new(),
            }],
            prompts: vec!["Always run cargo test before final response.".into()],
            location: None,
        }];

        let prompt = build_system_prompt(ws.path(), "model", &[], &skills, None, None);

        assert!(prompt.contains("<available_skills>"), "missing skills XML");
        assert!(prompt.contains("<name>code-review</name>"));
        assert!(prompt.contains("<description>Review code for bugs</description>"));
        assert!(prompt.contains("SKILL.md</location>"));
        assert!(prompt.contains("<instructions>"));
        assert!(prompt
            .contains("<instruction>Always run cargo test before final response.</instruction>"));
        assert!(prompt.contains("<tools>"));
        assert!(prompt.contains("<name>lint</name>"));
        assert!(prompt.contains("<kind>shell</kind>"));
        assert!(!prompt.contains("loaded on demand"));
    }

    #[test]
    fn prompt_skills_compact_mode_omits_instructions_and_tools() {
        let ws = make_workspace();
        let skills = vec![crate::skills::Skill {
            name: "code-review".into(),
            description: "Review code for bugs".into(),
            version: "1.0.0".into(),
            author: None,
            tags: vec![],
            tools: vec![crate::skills::SkillTool {
                name: "lint".into(),
                description: "Run static checks".into(),
                kind: "shell".into(),
                command: "cargo clippy".into(),
                args: HashMap::new(),
            }],
            prompts: vec!["Always run cargo test before final response.".into()],
            location: None,
        }];

        let prompt = build_system_prompt_with_mode(
            ws.path(),
            "model",
            &[],
            &skills,
            None,
            None,
            false,
            crate::config::SkillsPromptInjectionMode::Compact,
        );

        assert!(prompt.contains("<available_skills>"), "missing skills XML");
        assert!(prompt.contains("<name>code-review</name>"));
        assert!(prompt.contains("<location>skills/code-review/SKILL.md</location>"));
        assert!(prompt.contains("loaded on demand"));
        assert!(!prompt.contains("<instructions>"));
        assert!(!prompt
            .contains("<instruction>Always run cargo test before final response.</instruction>"));
        assert!(!prompt.contains("<tools>"));
    }

    #[test]
    fn prompt_skills_escape_reserved_xml_chars() {
        let ws = make_workspace();
        let skills = vec![crate::skills::Skill {
            name: "code<review>&".into(),
            description: "Review \"unsafe\" and 'risky' bits".into(),
            version: "1.0.0".into(),
            author: None,
            tags: vec![],
            tools: vec![crate::skills::SkillTool {
                name: "run\"linter\"".into(),
                description: "Run <lint> & report".into(),
                kind: "shell&exec".into(),
                command: "cargo clippy".into(),
                args: HashMap::new(),
            }],
            prompts: vec!["Use <tool_call> and & keep output \"safe\"".into()],
            location: None,
        }];

        let prompt = build_system_prompt(ws.path(), "model", &[], &skills, None, None);

        assert!(prompt.contains("<name>code&lt;review&gt;&amp;</name>"));
        assert!(prompt.contains(
            "<description>Review &quot;unsafe&quot; and &apos;risky&apos; bits</description>"
        ));
        assert!(prompt.contains("<name>run&quot;linter&quot;</name>"));
        assert!(prompt.contains("<description>Run &lt;lint&gt; &amp; report</description>"));
        assert!(prompt.contains("<kind>shell&amp;exec</kind>"));
        assert!(prompt.contains(
            "<instruction>Use &lt;tool_call&gt; and &amp; keep output &quot;safe&quot;</instruction>"
        ));
    }

    #[test]
    fn prompt_truncation() {
        let ws = make_workspace();
        // Write a file larger than BOOTSTRAP_MAX_CHARS
        let big_content = "x".repeat(BOOTSTRAP_MAX_CHARS + 1000);
        std::fs::write(ws.path().join("AGENTS.md"), &big_content).unwrap();

        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(
            prompt.contains("truncated at"),
            "large files should be truncated"
        );
        assert!(
            !prompt.contains(&big_content),
            "full content should not appear"
        );
    }

    #[test]
    fn prompt_empty_files_skipped() {
        let ws = make_workspace();
        std::fs::write(ws.path().join("TOOLS.md"), "").unwrap();

        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        // Empty file should not produce a header
        assert!(
            !prompt.contains("### TOOLS.md"),
            "empty files should be skipped"
        );
    }

    #[test]
    fn channel_log_truncation_is_utf8_safe_for_multibyte_text() {
        let msg = "Hello from TopClaw 🌍. Current status is healthy, and café-style UTF-8 text stays safe in logs.";

        // Reproduces the production crash path where channel logs truncate at 80 chars.
        let result = std::panic::catch_unwind(|| crate::util::truncate_with_ellipsis(msg, 80));
        assert!(
            result.is_ok(),
            "truncate_with_ellipsis should never panic on UTF-8"
        );

        let truncated = result.unwrap();
        assert!(!truncated.is_empty());
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn prompt_contains_channel_capabilities() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(
            prompt.contains("## Channel Capabilities"),
            "missing Channel Capabilities section"
        );
        assert!(
            prompt.contains("running as a messaging bot"),
            "missing channel context"
        );
        assert!(
            prompt.contains("NEVER repeat, describe, or echo credentials"),
            "missing security instruction"
        );
        assert!(
            prompt.contains("If the user asks what you can do"),
            "missing capability explanation guidance"
        );
        assert!(
            prompt.contains("Read-only investigation tools may be approved once"),
            "missing scoped approval guidance"
        );
    }

    #[test]
    fn runtime_tool_visibility_prompt_mentions_operator_controlled_workflows() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(MockPriceTool), Box::new(MockEchoTool)];
        let prompt = build_runtime_tool_visibility_prompt(&tools, &[], false);

        assert!(prompt.contains("If the user asks what you can do"));
        assert!(prompt.contains("actions that still require approval"));
        assert!(prompt.contains("Self-improvement is not automatic by default"));
    }

    #[test]
    fn prompt_workspace_path() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(prompt.contains(&format!("Working directory: `{}`", ws.path().display())));
    }

    #[test]
    fn conversation_memory_key_uses_message_id() {
        let msg = traits::ChannelMessage {
            id: "msg_abc123".into(),
            sender: "U123".into(),
            reply_target: "C456".into(),
            content: "hello".into(),
            channel: "slack".into(),
            timestamp: 1,
            thread_ts: None,
        };

        assert_eq!(conversation_memory_key(&msg), "slack_U123_msg_abc123");
    }

    #[test]
    fn conversation_memory_key_is_unique_per_message() {
        let msg1 = traits::ChannelMessage {
            id: "msg_1".into(),
            sender: "U123".into(),
            reply_target: "C456".into(),
            content: "first".into(),
            channel: "slack".into(),
            timestamp: 1,
            thread_ts: None,
        };
        let msg2 = traits::ChannelMessage {
            id: "msg_2".into(),
            sender: "U123".into(),
            reply_target: "C456".into(),
            content: "second".into(),
            channel: "slack".into(),
            timestamp: 2,
            thread_ts: None,
        };

        assert_ne!(
            conversation_memory_key(&msg1),
            conversation_memory_key(&msg2)
        );
    }

    #[tokio::test]
    async fn autosave_keys_preserve_multiple_conversation_facts() {
        let tmp = TempDir::new().unwrap();
        let mem = SqliteMemory::new(tmp.path()).unwrap();

        let msg1 = traits::ChannelMessage {
            id: "msg_1".into(),
            sender: "U123".into(),
            reply_target: "C456".into(),
            content: "I'm Paul".into(),
            channel: "slack".into(),
            timestamp: 1,
            thread_ts: None,
        };
        let msg2 = traits::ChannelMessage {
            id: "msg_2".into(),
            sender: "U123".into(),
            reply_target: "C456".into(),
            content: "I'm 45".into(),
            channel: "slack".into(),
            timestamp: 2,
            thread_ts: None,
        };

        mem.store(
            &conversation_memory_key(&msg1),
            &msg1.content,
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();
        mem.store(
            &conversation_memory_key(&msg2),
            &msg2.content,
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();

        assert_eq!(mem.count().await.unwrap(), 2);

        let recalled = mem.recall("45", 5, None).await.unwrap();
        assert!(recalled.iter().any(|entry| entry.content.contains("45")));
    }

    #[tokio::test]
    async fn build_memory_context_includes_recalled_entries() {
        let tmp = TempDir::new().unwrap();
        let mem = SqliteMemory::new(tmp.path()).unwrap();
        mem.store("age_fact", "Age is 45", MemoryCategory::Conversation, None)
            .await
            .unwrap();

        let context = build_memory_context(&mem, "age", 0.0).await;
        assert!(context.contains("[Memory context]"));
        assert!(context.contains("Age is 45"));
    }

    #[tokio::test]
    async fn process_channel_message_restores_per_sender_history_on_follow_ups() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-a".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "hello".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-b".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-1".to_string(),
                content: "follow up".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].len(), 2);
        assert_eq!(calls[0][0].0, "system");
        assert_eq!(calls[0][1].0, "user");
        assert_eq!(calls[1].len(), 4);
        assert_eq!(calls[1][0].0, "system");
        assert_eq!(calls[1][1].0, "user");
        assert_eq!(calls[1][2].0, "assistant");
        assert_eq!(calls[1][3].0, "user");
        assert!(calls[1][1].1.contains("hello"));
        assert!(calls[1][2].1.contains("response-1"));
        assert!(calls[1][3].1.contains("follow up"));
    }

    #[tokio::test]
    async fn process_channel_message_enriches_current_turn_without_persisting_context() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(RecallMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "msg-ctx-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-ctx".to_string(),
                content: "hello".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 2);
        assert_eq!(calls[0][1].0, "user");
        assert!(calls[0][1].1.contains("[Memory context]"));
        assert!(calls[0][1].1.contains("Age is 45"));
        assert!(calls[0][1].1.contains("hello"));

        let histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let turns = histories
            .get("test-channel_alice")
            .expect("history should be stored for sender");
        assert_eq!(turns[0].role, "user");
        assert!(turns[0].content.contains("hello"));
        assert!(!turns[0].content.contains("[Memory context]"));
    }

    #[tokio::test]
    async fn process_channel_message_telegram_keeps_system_instruction_at_top_only() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let mut histories = HashMap::new();
        histories.insert(
            "telegram_alice".to_string(),
            vec![
                ChatMessage::assistant("stale assistant"),
                ChatMessage::user("earlier user question"),
                ChatMessage::assistant("earlier assistant reply"),
            ],
        );

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(histories)),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "tg-msg-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "hello".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 2);

        let roles = calls[0]
            .iter()
            .map(|(role, _)| role.as_str())
            .collect::<Vec<_>>();
        assert_eq!(roles, vec!["system", "user"]);
        assert!(
            calls[0][0].1.contains("When responding on Telegram:"),
            "telegram channel instructions should be embedded into the system prompt"
        );
        assert!(
            calls[0][0].1.contains("For media attachments use markers:"),
            "telegram media marker guidance should live in the system prompt"
        );
        assert!(!calls[0].iter().skip(1).any(|(role, _)| role == "system"));
    }

    #[test]
    fn discord_delivery_instructions_include_codex_style_guidance() {
        let instructions = prompt::channel_delivery_instructions("discord")
            .expect("discord instructions should exist");
        assert!(instructions.contains("When responding on Discord:"));
        assert!(instructions.contains("Match the TopClaw CLI / Codex-style voice"));
        assert!(instructions.contains("Use tool results silently"));
    }

    #[test]
    fn extract_tool_context_summary_collects_alias_and_native_tool_calls() {
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::assistant(
                r#"<toolcall>
{"name":"shell","arguments":{"command":"date"}}
</toolcall>"#,
            ),
            ChatMessage::assistant(
                r#"{"content":null,"tool_calls":[{"id":"1","name":"web_search","arguments":"{}"}]}"#,
            ),
        ];

        let summary = extract_tool_context_summary(&history, 1);
        assert_eq!(summary, "[Used tools: shell, web_search]");
    }

    #[test]
    fn extract_tool_context_summary_collects_prompt_mode_tool_result_names() {
        let history = vec![
            ChatMessage::system("sys"),
            ChatMessage::assistant("Using markdown tool call fence"),
            ChatMessage::user(
                r#"[Tool results]
<tool_result name="http_request">
{"status":200}
</tool_result>
<tool_result name="shell">
Mon Feb 20
</tool_result>"#,
            ),
        ];

        let summary = extract_tool_context_summary(&history, 1);
        assert_eq!(summary, "[Used tools: http_request, shell]");
    }

    #[test]
    fn extract_tool_context_summary_respects_start_index() {
        let history = vec![
            ChatMessage::assistant(
                r#"<tool_call>
{"name":"stale_tool","arguments":{}}
</tool_call>"#,
            ),
            ChatMessage::assistant(
                r#"<tool_call>
{"name":"fresh_tool","arguments":{}}
</tool_call>"#,
            ),
        ];

        let summary = extract_tool_context_summary(&history, 1);
        assert_eq!(summary, "[Used tools: fresh_tool]");
    }

    #[test]
    fn strip_isolated_tool_json_artifacts_removes_tool_calls_and_results() {
        let mut known_tools = HashSet::new();
        known_tools.insert("schedule".to_string());

        let input = r#"{"name":"schedule","parameters":{"action":"create","message":"test"}}
{"name":"schedule","parameters":{"action":"cancel","task_id":"test"}}
Let me create the reminder properly:
{"name":"schedule","parameters":{"action":"create","message":"Go to sleep"}}
{"result":{"task_id":"abc","status":"scheduled"}}
Done reminder set for 1:38 AM."#;

        let result = strip_isolated_tool_json_artifacts(input, &known_tools);
        let normalized = result
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            normalized,
            "Let me create the reminder properly:\nDone reminder set for 1:38 AM."
        );
    }

    #[test]
    fn should_expose_internal_tool_details_matches_explicit_requests() {
        assert!(should_expose_internal_tool_details(
            "Please show commands and tool calls you used."
        ));
        assert!(should_expose_internal_tool_details(
            "请输出命令和工具调用过程"
        ));
        assert!(!should_expose_internal_tool_details(
            "帮我直接给最终结论，不要过程。"
        ));
    }

    #[test]
    fn should_expose_internal_tool_details_respects_negative_requests() {
        assert!(!should_expose_internal_tool_details(
            "Please do not show commands or tool calls, only final answer."
        ));
        assert!(!should_expose_internal_tool_details(
            "不要显示命令和工具调用，直接给最终结论。"
        ));
    }

    #[test]
    fn split_internal_progress_delta_detects_sentinel_prefix() {
        let payload = format!(
            "{}⏳ shell: ls -la\n",
            crate::agent::loop_::DRAFT_PROGRESS_SENTINEL
        );
        let (is_internal, visible) = split_internal_progress_delta(&payload);
        assert!(is_internal);
        assert_eq!(visible, "⏳ shell: ls -la\n");

        let (is_internal_plain, plain) = split_internal_progress_delta("final answer");
        assert!(!is_internal_plain);
        assert_eq!(plain, "final answer");
    }

    #[test]
    fn summarize_internal_progress_delta_sanitizes_internal_updates() {
        assert_eq!(
            summarize_internal_progress_delta("🤔 Thinking...\n"),
            Some("Analyzing the request...\n".to_string())
        );
        assert_eq!(
            summarize_internal_progress_delta("🤔 Thinking (round 2)...\n"),
            Some("Analyzing the request (round 2)...\n".to_string())
        );
        assert_eq!(
            summarize_internal_progress_delta("⏳ shell: ls -la\n"),
            Some("Using `shell`: ls -la\n".to_string())
        );
        assert_eq!(
            summarize_internal_progress_delta("💬 Got 1 tool call(s) (2s)\n"),
            Some("Using tools...\n".to_string())
        );
        assert_eq!(
            summarize_internal_progress_delta("✅ shell (1s)\n"),
            Some("Finished `shell`.\n".to_string())
        );
        assert_eq!(
            summarize_internal_progress_delta("↻ Retrying: response implied action\n"),
            Some("Retrying the previous step...\n".to_string())
        );
        assert_eq!(summarize_internal_progress_delta("plain text"), None);
    }

    #[test]
    fn summarize_internal_progress_delta_preserves_visible_heartbeat_updates() {
        assert_eq!(
            summarize_internal_progress_delta("⏳ Still working (15s)...\n"),
            Some("⏳ Still working (15s)...\n".to_string())
        );
    }

    #[test]
    fn contextualize_progress_heartbeat_reuses_last_meaningful_summary() {
        assert_eq!(
            contextualize_progress_heartbeat(
                "⏳ Still working (15s)...\n",
                Some("Using `task_plan`: fetch repo metadata\n"),
            ),
            "⏳ Still working: Using `task_plan`: fetch repo metadata\n".to_string()
        );
    }

    #[test]
    fn contextualize_progress_heartbeat_leaves_generic_update_without_context() {
        assert_eq!(
            contextualize_progress_heartbeat("⏳ Still working (15s)...\n", None),
            "⏳ Still working: Analyzing the request...\n".to_string()
        );
    }

    #[test]
    fn looks_like_remote_repo_review_request_matches_repo_audit_prompts() {
        assert!(looks_like_remote_repo_review_request(
            "review this repo https://github.com/topway-ai/topclaw"
        ));
        assert!(looks_like_remote_repo_review_request(
            "你的所有代码都在这里，看看有啥明显缺陷么？https://github.com/topway-ai/topclaw"
        ));
        assert!(!looks_like_remote_repo_review_request(
            "https://github.com/topway-ai/topclaw"
        ));
        assert!(!looks_like_remote_repo_review_request(
            "show me local file /home/frank/claw_projects/topclaw/README.md"
        ));
    }

    #[test]
    fn capability_tool_state_accounts_for_presence_exclusion_and_approval() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(NamedTestTool("web_fetch"))];
        let approval_manager =
            ApprovalManager::from_config(&crate::config::AutonomyConfig::default());
        assert_eq!(
            capability_tool_state(&tools, &[], &approval_manager, "web_fetch"),
            CapabilityState::NeedsApproval
        );
        assert_eq!(
            capability_tool_state(
                &tools,
                &["web_fetch".to_string()],
                &approval_manager,
                "web_fetch"
            ),
            CapabilityState::Excluded
        );

        let gated_approval = ApprovalManager::from_config(&crate::config::AutonomyConfig {
            always_ask: vec!["web_fetch".to_string()],
            ..crate::config::AutonomyConfig::default()
        });
        assert_eq!(
            capability_tool_state(&tools, &[], &gated_approval, "web_fetch"),
            CapabilityState::NeedsApproval
        );
        gated_approval.grant_non_cli_session("web_fetch");
        assert_eq!(
            capability_tool_state(&tools, &[], &gated_approval, "web_fetch"),
            CapabilityState::Available
        );
        assert_eq!(
            capability_tool_state(&[], &[], &approval_manager, "web_fetch"),
            CapabilityState::Missing
        );
    }

    #[test]
    fn infer_capability_recovery_plan_flags_shell_requests_for_approval() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(NamedTestTool("shell"))];
        let approval_manager = Arc::new(ApprovalManager::from_config(
            &crate::config::AutonomyConfig {
                always_ask: vec!["shell".to_string()],
                ..crate::config::AutonomyConfig::default()
            },
        ));
        let runtime_ctx = ChannelRuntimeContext {
            channels_by_name: Arc::new(HashMap::new()),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(tools),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("system".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager,
        };
        let msg = traits::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "topclaw_user".to_string(),
            reply_target: "chat-1".to_string(),
            content: "run cargo test in this repo".to_string(),
            channel: "telegram".to_string(),
            timestamp: 0,
            thread_ts: None,
        };

        let plan = infer_capability_recovery_plan(&runtime_ctx, &msg, &[])
            .expect("shell requests should produce a capability recovery plan");

        assert_eq!(plan.tool_name, "shell");
        assert_eq!(plan.state, CapabilityState::NeedsApproval);
        assert!(plan.reason.contains("shell or terminal execution"));
        assert!(plan.message.contains("/approve-confirm"));
    }

    #[tokio::test]
    async fn process_channel_message_remote_repo_url_without_web_tools_uses_local_agent_path() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "repo-link-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content:
                    "你的所有代码都在这里，看看有啥明显缺陷么？https://github.com/topway-ai/topclaw"
                        .to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("response-1"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            !calls.is_empty(),
            "provider should still be called so the local workspace path can handle repo review prompts"
        );
    }

    #[tokio::test]
    async fn process_channel_message_exported_github_improvement_prompt_does_not_fall_back_to_shell(
    ) {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(vec!["shell".to_string()])),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "repo-link-exported-1".to_string(),
                sender: "topclaw_user".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "https://github.com/topway-ai/topclaw This is your codebase, tell me what improvements you can do make yourself better and smarter?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(
            !sent[0].contains("I can’t use `shell`"),
            "prompt should stay on the normal agent path instead of shell recovery"
        );
        assert!(sent[0].contains("response-1"));
    }

    #[tokio::test]
    async fn process_channel_message_web_url_without_web_tools_uses_normal_agent_path() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "web-link-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "check https://example.com and tell me the key issues".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("response-1"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            !calls.is_empty(),
            "provider should still be called so the normal tool-selection path can decide what to do"
        );
    }

    #[tokio::test]
    async fn process_channel_message_streaming_surfaces_useful_progress_for_telegram_channel() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramDraftStreamingRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-telegram-no-thinking".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent_messages = channel_impl.sent_messages.lock().await;
        let draft_updates = channel_impl.draft_updates.lock().await;
        assert!(
            sent_messages
                .iter()
                .chain(draft_updates.iter())
                .all(|entry| !entry.contains("Thinking") && !entry.contains("Working...\n")),
            "telegram progress should suppress generic placeholder updates: sent={sent_messages:?} updates={draft_updates:?}"
        );
        assert!(
            sent_messages
                .iter()
                .chain(draft_updates.iter())
                .any(|entry| entry.contains("Using `mock_price`")),
            "telegram progress should surface the tool being used: sent={sent_messages:?} updates={draft_updates:?}"
        );
    }

    #[tokio::test]
    async fn process_channel_message_streaming_contextualizes_heartbeat_for_telegram_channel() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramDraftStreamingRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(ToolCallingProvider),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(SlowMockPriceTool {
                delay: Duration::from_secs(CHANNEL_PROGRESS_HEARTBEAT_SECS + 1),
            })]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 10,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS + CHANNEL_PROGRESS_HEARTBEAT_SECS,
            interrupt_on_new_message: false,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &autonomy_with_mock_price_auto_approve(),
            )),
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-telegram-contextual-heartbeat".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "What is the BTC price now?".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let draft_updates = channel_impl.draft_updates.lock().await;
        assert!(
            draft_updates
                .iter()
                .any(|entry| entry.contains("⏳ Still working: Using `mock_price`")),
            "telegram heartbeat should reuse the latest meaningful progress instead of a generic timer: updates={draft_updates:?}"
        );
        assert!(
            draft_updates
                .iter()
                .all(|entry| !entry.contains("⏳ Still working (")),
            "telegram heartbeat should avoid generic timer-only progress once a concrete step is known: updates={draft_updates:?}"
        );
    }

    #[tokio::test]
    async fn process_channel_message_web_request_with_gated_tool_creates_approval_request() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(HistoryCaptureProvider::default());
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(NamedTestTool("web_fetch"))]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["web_fetch".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx.clone(),
            traits::ChannelMessage {
                id: "web-link-approval-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "please inspect https://example.com/docs".to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0]
            .contains("I can finish this, but I need supervised access to `web_fetch` first."));
        assert!(sent[0].contains("/approve-confirm apr-"));

        let calls = provider_impl
            .calls
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        assert!(
            calls.is_empty(),
            "provider should not be called before approval"
        );
        assert_eq!(
            runtime_ctx
                .approval_manager
                .list_non_cli_pending_requests(
                    Some("alice"),
                    Some("telegram"),
                    Some("chat-telegram")
                )
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn process_channel_message_ambiguous_request_uses_llm_recovery_classifier() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(TelegramRecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let provider_impl = Arc::new(StructuredRecoveryProvider {
            intent_response:
                r#"{"intent":"needs_tools","reason":"the user is explicitly asking what capability is needed"}"#
                    .to_string(),
            classifier_response: r#"{"need_recovery":true,"tool_name":"web_fetch","reason":"needs remote docs access"}"#.to_string(),
            history_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: provider_impl.clone(),
            default_provider: Arc::new("test-provider".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![Box::new(NamedTestTool("web_fetch"))]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("test-system-prompt".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig {
                    always_ask: vec!["web_fetch".to_string()],
                    ..crate::config::AutonomyConfig::default()
                },
            )),
        });

        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "ambiguous-recovery-1".to_string(),
                sender: "alice".to_string(),
                reply_target: "chat-telegram".to_string(),
                content: "I can't solve this upstream docs problem from chat; figure out what capability you need"
                    .to_string(),
                channel: "telegram".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("supervised access to `web_fetch`"));
        assert_eq!(provider_impl.history_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn build_channel_system_prompt_includes_visibility_policy() {
        let hidden = build_channel_system_prompt("base", "telegram", "chat", false);
        assert!(hidden.contains("run tools/functions in the background"));
        assert!(hidden.contains("Do not reveal raw tool names"));

        let exposed = build_channel_system_prompt("base", "telegram", "chat", true);
        assert!(exposed.contains("user explicitly requested command/tool details"));
    }

    #[test]
    fn strip_isolated_tool_json_artifacts_preserves_non_tool_json() {
        let mut known_tools = HashSet::new();
        known_tools.insert("shell".to_string());

        let input = r#"{"name":"profile","parameters":{"timezone":"UTC"}}
This is an example JSON object for profile settings."#;

        let result = strip_isolated_tool_json_artifacts(input, &known_tools);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_channel_response_removes_tool_call_tags_and_tool_json_artifacts() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(MockPriceTool)];

        let input = r#"Let me check.
<tool_call>
{"name":"debug_trace","arguments":{"foo":"bar"}}
</tool_call>
{"name":"mock_price","parameters":{"symbol":"BTC"}}
{"result":{"symbol":"BTC","price_usd":65000}}
BTC is currently around $65,000 based on latest tool output."#;

        let result = sanitize_channel_response(input, &tools);
        let normalized = result
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            normalized,
            "Let me check.\nBTC is currently around $65,000 based on latest tool output."
        );
        assert!(!result.contains("<tool_call>"));
        assert!(!result.contains("\"name\":\"mock_price\""));
        assert!(!result.contains("\"result\""));
    }

    #[test]
    fn none_identity_config_uses_bootstrap() {
        let ws = make_workspace();
        let prompt = build_system_prompt(ws.path(), "model", &[], &[], None, None);

        assert!(prompt.contains("### SOUL.md"));
        assert!(prompt.contains("Be helpful"));
    }

    #[test]
    fn classify_health_ok_true() {
        let state = classify_health_result(&Ok(true));
        assert_eq!(state, ChannelHealthState::Healthy);
    }

    #[test]
    fn classify_health_ok_false() {
        let state = classify_health_result(&Ok(false));
        assert_eq!(state, ChannelHealthState::Unhealthy);
    }

    #[tokio::test]
    async fn classify_health_timeout() {
        let result = tokio::time::timeout(Duration::from_millis(1), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            true
        })
        .await;
        let state = classify_health_result(&result);
        assert_eq!(state, ChannelHealthState::Timeout);
    }

    struct AlwaysFailChannel {
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    struct BlockUntilClosedChannel {
        name: String,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Channel for AlwaysFailChannel {
        fn name(&self) -> &str {
            self.name
        }

        async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
            Ok(())
        }

        async fn listen(
            &self,
            _tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("listen boom")
        }
    }

    #[async_trait::async_trait]
    impl Channel for BlockUntilClosedChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn send(&self, _message: &SendMessage) -> anyhow::Result<()> {
            Ok(())
        }

        async fn listen(
            &self,
            tx: tokio::sync::mpsc::Sender<traits::ChannelMessage>,
        ) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tx.closed().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn supervised_listener_marks_error_and_restarts_on_failures() {
        let calls = Arc::new(AtomicUsize::new(0));
        let channel: Arc<dyn Channel> = Arc::new(AlwaysFailChannel {
            name: "test-supervised-fail",
            calls: Arc::clone(&calls),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(1);
        let handle = spawn_supervised_listener(channel, tx, 1, 1);

        tokio::time::sleep(Duration::from_millis(80)).await;
        drop(rx);
        handle.abort();
        let _ = handle.await;

        let snapshot = crate::health::snapshot_json();
        let component = &snapshot["components"]["channel:test-supervised-fail"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(component["last_error"]
            .as_str()
            .unwrap_or("")
            .contains("listen boom"));
        assert!(calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn supervised_listener_refreshes_health_while_running() {
        let calls = Arc::new(AtomicUsize::new(0));
        let channel_name = format!("test-supervised-heartbeat-{}", uuid::Uuid::new_v4());
        let component_name = format!("channel:{channel_name}");
        let channel: Arc<dyn Channel> = Arc::new(BlockUntilClosedChannel {
            name: channel_name,
            calls: Arc::clone(&calls),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(1);
        let handle = spawn_supervised_listener_with_health_interval(
            channel,
            tx,
            1,
            1,
            Duration::from_millis(20),
        );

        tokio::time::sleep(Duration::from_millis(35)).await;
        let first_last_ok = crate::health::snapshot_json()["components"][&component_name]
            ["last_ok"]
            .as_str()
            .unwrap_or("")
            .to_string();
        assert!(!first_last_ok.is_empty());

        tokio::time::sleep(Duration::from_millis(70)).await;
        let second_last_ok = crate::health::snapshot_json()["components"][&component_name]
            ["last_ok"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let first = chrono::DateTime::parse_from_rfc3339(&first_last_ok)
            .expect("last_ok should be valid RFC3339");
        let second = chrono::DateTime::parse_from_rfc3339(&second_last_ok)
            .expect("last_ok should be valid RFC3339");
        assert!(second > first, "expected periodic health heartbeat refresh");

        drop(rx);
        let join = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(join.is_ok(), "listener should stop after channel shutdown");
        assert!(calls.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn maybe_restart_daemon_systemd_args_regression() {
        assert_eq!(
            SYSTEMD_STATUS_ARGS,
            ["--user", "is-active", "topclaw.service"]
        );
        assert_eq!(
            SYSTEMD_RESTART_ARGS,
            ["--user", "restart", "topclaw.service"]
        );
    }

    #[test]
    fn maybe_restart_daemon_openrc_args_regression() {
        assert_eq!(OPENRC_STATUS_ARGS, ["topclaw", "status"]);
        assert_eq!(OPENRC_RESTART_ARGS, ["topclaw", "restart"]);
    }

    #[test]
    fn normalize_merges_consecutive_user_turns() {
        let turns = vec![ChatMessage::user("hello"), ChatMessage::user("world")];
        let result = normalize_cached_channel_turns(turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content, "hello\n\nworld");
    }

    #[test]
    fn normalize_preserves_strict_alternation() {
        let turns = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
            ChatMessage::user("bye"),
        ];
        let result = normalize_cached_channel_turns(turns);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].content, "hello");
        assert_eq!(result[1].content, "hi");
        assert_eq!(result[2].content, "bye");
    }

    #[test]
    fn normalize_merges_multiple_consecutive_user_turns() {
        let turns = vec![
            ChatMessage::user("a"),
            ChatMessage::user("b"),
            ChatMessage::user("c"),
        ];
        let result = normalize_cached_channel_turns(turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content, "a\n\nb\n\nc");
    }

    #[test]
    fn normalize_empty_input() {
        let result = normalize_cached_channel_turns(vec![]);
        assert!(result.is_empty());
    }

    // ── E2E: photo [IMAGE:] marker rejected by non-vision provider ───

    /// End-to-end test: a photo attachment message (containing `[IMAGE:]`
    /// marker) sent through `process_channel_message` with a non-vision
    /// provider must produce a `"⚠️ Error: …does not support vision"` reply
    /// on the recording channel — no real Telegram or LLM API required.
    #[tokio::test]
    async fn e2e_photo_attachment_rejected_by_non_vision_provider() {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        // DummyProvider has default capabilities (vision: false).
        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("dummy".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("You are a helpful assistant.".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        // Simulate a photo attachment message with [IMAGE:] marker.
        Box::pin(process_channel_message(
            runtime_ctx,
            traits::ChannelMessage {
                id: "msg-photo-1".to_string(),
                sender: "topclaw_user".to_string(),
                reply_target: "chat-photo".to_string(),
                content: "[IMAGE:/tmp/workspace/photo_99_1.jpg]\n\nWhat is this?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 1, "expected exactly one reply message");
        assert!(
            sent[0].contains("does not support vision"),
            "reply must mention vision capability error, got: {}",
            sent[0]
        );
        assert!(
            sent[0].contains("⚠️ Error"),
            "reply must start with error prefix, got: {}",
            sent[0]
        );
    }

    #[tokio::test]
    async fn e2e_failed_vision_turn_does_not_poison_follow_up_text_turn() {
        let temp = TempDir::new().unwrap();
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel: Arc<dyn Channel> = channel_impl.clone();

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            channels_by_name: Arc::new(channels_by_name),
            provider: Arc::new(DummyProvider),
            default_provider: Arc::new("dummy".to_string()),
            memory: Arc::new(NoopMemory),
            tools_registry: Arc::new(vec![]),
            observer: Arc::new(NoopObserver),
            system_prompt: Arc::new("You are a helpful assistant.".to_string()),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            auto_save_memory: false,
            max_tool_iterations: 5,
            min_relevance_score: 0.0,
            conversation_histories: Arc::new(Mutex::new(HashMap::new())),
            provider_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_key: None,
            api_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            provider_runtime_options: providers::ProviderRuntimeOptions::default(),
            workspace_dir: Arc::new(temp.path().to_path_buf()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            interrupt_on_new_message: false,
            multimodal: crate::config::MultimodalConfig::default(),
            hooks: None,
            non_cli_excluded_tools: Arc::new(Mutex::new(Vec::new())),
            query_classification: crate::config::QueryClassificationConfig::default(),
            model_routes: Vec::new(),
            approval_manager: Arc::new(ApprovalManager::from_config(
                &crate::config::AutonomyConfig::default(),
            )),
        });

        Box::pin(process_channel_message(
            Arc::clone(&runtime_ctx),
            traits::ChannelMessage {
                id: "msg-photo-1".to_string(),
                sender: "topclaw_user".to_string(),
                reply_target: "chat-photo".to_string(),
                content: "[IMAGE:/tmp/workspace/photo_99_1.jpg]\n\nWhat is this?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 1,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        Box::pin(process_channel_message(
            Arc::clone(&runtime_ctx),
            traits::ChannelMessage {
                id: "msg-text-2".to_string(),
                sender: "topclaw_user".to_string(),
                reply_target: "chat-photo".to_string(),
                content: "What is WAL?".to_string(),
                channel: "test-channel".to_string(),
                timestamp: 2,
                thread_ts: None,
            },
            CancellationToken::new(),
        ))
        .await;

        let sent = channel_impl.sent_messages.lock().await;
        assert_eq!(sent.len(), 2, "expected one error and one successful reply");
        assert!(
            sent[0].contains("does not support vision"),
            "first reply must mention vision capability error, got: {}",
            sent[0]
        );
        assert!(
            sent[1].ends_with(":ok"),
            "second reply should succeed for text-only turn, got: {}",
            sent[1]
        );
        drop(sent);

        let histories = runtime_ctx
            .conversation_histories
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let turns = histories
            .get("test-channel_topclaw_user")
            .expect("history should exist for sender");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert!(turns[0].content.contains("What is WAL?"));
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].content, "ok");
        assert!(
            turns.iter().all(|turn| !turn.content.contains("[IMAGE:")),
            "failed vision turn must not persist image marker content"
        );
    }
}
