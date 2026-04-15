#![forbid(unsafe_code)]
//! TopClaw is a trait-driven Rust runtime for agentic CLI, channel, optional
//! gateway, and hardware workflows.
//!
//! The crate is organized around a small set of extension traits:
//!
//! - [`providers::traits::Provider`] for model backends
//! - [`channels::traits::Channel`] for messaging transports
//! - [`tools::traits::Tool`] for agent-callable capabilities
//! - [`memory::traits::Memory`] for persistence and recall
//! - [`observability::traits::Observer`] for telemetry sinks
//! - [`runtime::traits::RuntimeAdapter`] for execution environments
//! - optional peripheral integration for hardware boards that expose tools
//!
//! # Execution Flow
//!
//! A typical TopClaw request goes through these stages:
//!
//! 1. load and normalize [`Config`]
//! 2. build runtime, security, memory, provider, observer, and tool instances
//! 3. receive input from the CLI, a channel, or the optional gateway
//! 4. construct the system prompt and conversation history
//! 5. call the active provider
//! 6. execute any requested tool calls, subject to runtime and security policy
//! 7. return the final response and record memory and telemetry side effects
//!
//! # Top-Level Modules
//!
//! The most commonly used modules are:
//!
//! - [`agent`] for the orchestration loop
//! - [`config`] for config loading, defaults, and schema-backed types
//! - [`providers`] for model backends and routing
//! - [`channels`] for external message transports
//! - [`tools`] for agent-callable capabilities
//! - [`memory`] for memory backends and retrieval
//! - [`security`] for policy, pairing, secrets, and sandboxing
//! - optional gateway support for HTTP and OpenAI-compatible endpoints
//!
//! # Example
//!
//! ```no_run
//! use topclaw::{agent::Agent, Config};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let config = Config::load_or_init().await?;
//! let mut agent = Agent::from_config(&config)?;
//! let reply = agent.turn("Summarize the current workspace state").await?;
//! println!("{reply}");
//! # Ok(())
//! # }
//! ```
//!
//! `Config::load_or_init` may create first-run state on disk. Use the CLI if
//! you want the guided onboarding flow instead of direct library wiring.
// Lint configuration is in [lints] section of Cargo.toml.

use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod agent;
pub(crate) mod approval;
pub mod auth;
pub mod backup;
pub mod channels;
pub mod config;
pub mod coordination;
#[cfg(feature = "gateway")]
pub(crate) mod cost;
pub mod cron;
pub mod daemon;
pub mod doctor;
#[cfg(feature = "gateway")]
pub mod gateway;
pub(crate) mod health;
pub(crate) mod heartbeat;
pub mod hooks;
pub mod memory;
pub(crate) mod multimodal;
pub mod observability;
pub mod onboard;
pub mod providers;
pub mod runtime;
pub mod security;
pub mod service;
#[cfg(feature = "computer-use-sidecar")]
pub mod sidecar;
pub mod skills;
#[doc(hidden)]
pub mod test_capabilities;
pub mod tools;
#[cfg(feature = "tunnel")]
pub(crate) mod tunnel;
pub mod update;
pub(crate) mod util;
pub mod workspace;

pub use config::Config;

/// Backup-management subcommands used by the CLI and any embedding wrapper
/// that wants to delegate archive lifecycle operations to TopClaw.
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackupCommands {
    /// Create a portable backup bundle of the current TopClaw state
    Create {
        /// Destination directory for the backup bundle
        destination: PathBuf,
        /// Include runtime logs in the backup bundle
        #[arg(long)]
        include_logs: bool,
    },
    /// Inspect a backup bundle and verify its integrity
    Inspect {
        /// Source backup bundle directory
        source: PathBuf,
    },
    /// Restore a previously created TopClaw backup bundle
    Restore {
        /// Source backup bundle directory
        source: PathBuf,
        /// Replace an existing non-empty target config directory
        #[arg(long)]
        force: bool,
    },
}

/// Service-management subcommands for installing and controlling the
/// long-running background runtime.
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceCommands {
    /// Install daemon service unit for auto-start and restart
    Install,
    /// Start daemon service
    Start,
    /// Stop daemon service
    Stop,
    /// Restart daemon service to apply latest config
    Restart,
    /// Check daemon service status
    Status,
    /// Uninstall daemon service unit
    Uninstall,
}

/// Channel-management subcommands for configuring, diagnosing, and starting
/// external messaging transports.
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelCommands {
    /// List all configured channels
    List,
    /// Start all configured channels (handled in main.rs for async)
    Start,
    /// Run health checks for configured channels (handled in main.rs for async)
    Doctor,
    /// Add a new channel configuration
    #[command(long_about = "\
Add a new channel configuration.

Provide the channel type and a JSON object with the required \
configuration keys for that channel type.

If you want a guided setup flow instead of hand-writing JSON, use:
  topclaw bootstrap --channels-only

Supported types: telegram, discord.

Examples:
  topclaw channel add telegram '{\"bot_token\":\"123456:ABC...\",\"name\":\"my-bot\",\"allowed_users\":[\"topclaw_user\"]}'
  topclaw channel add discord '{\"bot_token\":\"MTIz...\",\"name\":\"my-discord\",\"allowed_users\":[\"topclaw_user\"]}'")]
    Add {
        /// Channel type (telegram, discord)
        channel_type: String,
        /// Optional configuration as JSON
        config: String,
    },
    /// Remove a channel configuration
    Remove {
        /// Channel name to remove
        name: String,
    },
    /// Bind a Telegram identity (username or numeric user ID) into allowlist
    #[command(long_about = "\
Bind a Telegram identity into the allowlist.

Adds a Telegram username (without the '@' prefix) or numeric user \
ID to the channel allowlist so the agent will respond to messages \
from that identity.

Examples:
  topclaw channel bind-telegram topclaw_user
  topclaw channel bind-telegram 123456789")]
    BindTelegram {
        /// Telegram identity to allow (username without '@' or numeric user ID)
        identity: String,
    },
}

/// Skill-management subcommands for installing, vetting, auditing, and
/// removing reusable capability bundles.
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillCommands {
    /// List installed skills and whether each one is enabled or disabled
    List,
    /// Run the full skill vetter against a source directory or installed skill
    Vet {
        /// Skill path or installed skill name
        source: String,
        /// Emit the full report as JSON
        #[arg(long)]
        json: bool,
        /// Optional sandbox probe mode (`docker`)
        #[arg(long, value_parser = ["docker"])]
        sandbox: Option<String>,
    },
    /// Audit a skill source directory or installed skill name
    Audit {
        /// Skill path or installed skill name
        source: String,
    },
    /// Install a new skill from a URL or local path
    Install {
        /// Source URL or local path
        source: String,
    },
    /// Enable a disabled installed skill
    Enable {
        /// Skill name to enable
        name: String,
    },
    /// Disable an enabled installed skill without deleting it
    Disable {
        /// Skill name to disable
        name: String,
    },
    /// Remove an installed skill
    Remove {
        /// Skill name to remove
        name: String,
    },
}

/// Scheduler subcommands for recurring and one-shot tasks backed by the cron
/// subsystem.
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CronCommands {
    /// List all scheduled tasks
    List,
    /// Add a new scheduled task
    #[command(long_about = "\
Add a new recurring scheduled task.

Uses standard 5-field cron syntax: 'min hour day month weekday'. \
Times are evaluated in UTC by default; use --tz with an IANA \
timezone name to override.

Examples:
  topclaw cron add '0 9 * * 1-5' 'Good morning' --tz America/New_York
  topclaw cron add '*/30 * * * *' 'Check system health'")]
    Add {
        /// Cron expression
        expression: String,
        /// Optional IANA timezone (e.g. America/Los_Angeles)
        #[arg(long)]
        tz: Option<String>,
        /// Command to run
        command: String,
    },
    /// Add a one-shot scheduled task at an RFC3339 timestamp
    #[command(long_about = "\
Add a one-shot task that fires at a specific UTC timestamp.

The timestamp must be in RFC 3339 format (e.g. 2025-01-15T14:00:00Z).

Examples:
  topclaw cron add-at 2025-01-15T14:00:00Z 'Send reminder'
  topclaw cron add-at 2025-12-31T23:59:00Z 'Happy New Year!'")]
    AddAt {
        /// One-shot timestamp in RFC3339 format
        at: String,
        /// Command to run
        command: String,
    },
    /// Add a fixed-interval scheduled task
    #[command(long_about = "\
Add a task that repeats at a fixed interval.

Interval is specified in milliseconds. For example, 60000 = 1 minute.

Examples:
  topclaw cron add-every 60000 'Ping heartbeat'     # every minute
  topclaw cron add-every 3600000 'Hourly report'    # every hour")]
    AddEvery {
        /// Interval in milliseconds
        every_ms: u64,
        /// Command to run
        command: String,
    },
    /// Add a one-shot delayed task (e.g. "30m", "2h", "1d")
    #[command(long_about = "\
Add a one-shot task that fires after a delay from now.

Accepts human-readable durations: s (seconds), m (minutes), \
h (hours), d (days).

Examples:
  topclaw cron once 30m 'Run backup in 30 minutes'
  topclaw cron once 2h 'Follow up on deployment'
  topclaw cron once 1d 'Daily check'")]
    Once {
        /// Delay duration
        delay: String,
        /// Command to run
        command: String,
    },
    /// Remove a scheduled task
    Remove {
        /// Task ID
        id: String,
    },
    /// Update a scheduled task
    #[command(long_about = "\
Update one or more fields of an existing scheduled task.

Only the fields you specify are changed; others remain unchanged.

Examples:
  topclaw cron update <task-id> --expression '0 8 * * *'
  topclaw cron update <task-id> --tz Europe/London --name 'Morning check'
  topclaw cron update <task-id> --command 'Updated message'")]
    Update {
        /// Task ID
        id: String,
        /// New cron expression
        #[arg(long)]
        expression: Option<String>,
        /// New IANA timezone
        #[arg(long)]
        tz: Option<String>,
        /// New command to run
        #[arg(long)]
        command: Option<String>,
        /// New job name
        #[arg(long)]
        name: Option<String>,
    },
    /// Pause a scheduled task
    Pause {
        /// Task ID
        id: String,
    },
    /// Resume a paused task
    Resume {
        /// Task ID
        id: String,
    },
}

/// Memory management subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryCommands {
    /// List memory entries with optional filters
    List {
        /// Filter by category (core, daily, conversation, or custom name)
        #[arg(long)]
        category: Option<String>,
        /// Filter by session ID
        #[arg(long)]
        session: Option<String>,
        /// Maximum number of entries to display
        #[arg(long, default_value = "50")]
        limit: usize,
        /// Number of entries to skip (for pagination)
        #[arg(long, default_value = "0")]
        offset: usize,
    },
    /// Get a specific memory entry by key
    Get {
        /// Memory key to look up
        key: String,
    },
    /// Show memory backend statistics and health
    Stats,
    /// Clear memories by category, by key, or clear all
    Clear {
        /// Delete a single entry by key (supports prefix match)
        #[arg(long)]
        key: Option<String>,
        /// Only clear entries in this category
        #[arg(long)]
        category: Option<String>,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}
