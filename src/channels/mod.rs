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
pub mod channel_runtime_context;
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
pub(crate) mod runtime_config;
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
#[cfg(feature = "gateway")]
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
mod tests;
