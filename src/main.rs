#![warn(clippy::all, clippy::pedantic)]
#![forbid(unsafe_code)]
#![allow(
    clippy::assigning_clones,
    clippy::bool_to_int_with_if,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::field_reassign_with_default,
    clippy::float_cmp,
    clippy::implicit_clone,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::manual_let_else,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::needless_pass_by_value,
    clippy::needless_raw_string_hashes,
    clippy::redundant_closure_for_method_calls,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_field_names,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unused_self,
    clippy::cast_precision_loss,
    clippy::unnecessary_cast,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_literal_bound,
    clippy::unnecessary_map_or,
    clippy::unnecessary_wraps,
    dead_code
)]

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use topclaw::{
    agent, auth, backup, channels, config, cron, daemon, doctor, gateway, hardware, integrations,
    memory, observability, onboard, peripherals, providers, security, self_improvement,
    service, skills, update, Config,
};
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};

fn parse_temperature(s: &str) -> std::result::Result<f64, String> {
    let t: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if !(0.0..=2.0).contains(&t) {
        return Err("temperature must be between 0.0 and 2.0".to_string());
    }
    Ok(t)
}

fn provider_ready(config: &Config) -> bool {
    config
        .default_provider
        .as_deref()
        .filter(|provider| !provider.trim().is_empty())
        .is_some_and(|provider| {
            config.api_key.is_some()
                || auth::has_saved_profile_for_provider(config, provider)
                || doctor::provider_supports_keyless_usage(provider)
        })
}

fn channels_configured(config: &Config) -> bool {
    config.channels_config.channels().iter().any(|(_, ok)| *ok)
}

fn daemon_ready(diag_results: &[doctor::DiagResult]) -> bool {
    !diag_results.iter().any(|item| {
        item.category == "daemon"
            && item.severity == doctor::Severity::Error
            && (item.message.starts_with("state file not found:")
                || item.message.starts_with("heartbeat stale")
                || item.message.starts_with("invalid daemon timestamp:"))
    })
}

mod main_handlers;
use main_handlers::{
    handle_auth_command, handle_estop_command, handle_security_command, handle_uninstall_command,
    handle_workspace_command, write_shell_completion,
};

// Re-export so binary modules can use crate::<CommandEnum> while keeping a single source of truth.
pub use topclaw::{
    BackupCommands, ChannelCommands, CronCommands, HardwareCommands, IntegrationCommands,
    MemoryCommands, PeripheralCommands, ServiceCommands, SkillCommands,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "fish")]
    Fish,
    #[value(name = "zsh")]
    Zsh,
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "elvish")]
    Elvish,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum EstopLevelArg {
    #[value(name = "kill-all")]
    KillAll,
    #[value(name = "network-kill")]
    NetworkKill,
    #[value(name = "domain-block")]
    DomainBlock,
    #[value(name = "tool-freeze")]
    ToolFreeze,
}

/// `TopClaw` - Zero overhead. Zero compromise. 100% Rust.
#[derive(Parser, Debug)]
#[command(name = "topclaw")]
#[command(author = "theonlyhennygod")]
#[command(version)]
#[command(about = "TopClaw command-line interface", long_about = None)]
#[command(after_help = "\
Start here:
  topclaw bootstrap                # first-time setup
  topclaw agent                    # interactive chat
  topclaw daemon                   # always-on runtime

Most common tasks:
  topclaw status                   # quick readiness summary
  topclaw status --diagnose        # include full diagnostic report
  topclaw doctor                   # run diagnostics
  topclaw update --check           # check for a new release
  topclaw update                   # install latest release
  topclaw service restart          # restart background service after update

Background operation:
  topclaw service install
  topclaw service start
  topclaw service stop
  topclaw service restart
  topclaw service uninstall

Extensions and setup:
  topclaw channel list
  topclaw models refresh
  topclaw skills list
  topclaw integrations info <name>

Full uninstall:
  topclaw uninstall              # remove the installed binary, keep ~/.topclaw data
  topclaw uninstall --purge      # remove the binary and erase ~/.topclaw data too")]
struct Cli {
    #[arg(long, global = true)]
    config_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Set up TopClaw for first use
    #[command(name = "bootstrap")]
    Onboard {
        /// Run the full interactive wizard (default is quick setup)
        #[arg(long)]
        interactive: bool,

        /// Overwrite existing config without confirmation
        #[arg(long)]
        force: bool,

        /// Reconfigure or add channels only using the guided wizard
        #[arg(long)]
        channels_only: bool,

        /// API key (used in quick mode, ignored with --interactive)
        #[arg(long)]
        api_key: Option<String>,

        /// Provider name (used in quick mode, default: openrouter)
        #[arg(long)]
        provider: Option<String>,
        /// Model ID override (used in quick mode)
        #[arg(long)]
        model: Option<String>,
        /// Memory backend (sqlite, lucid, markdown, none) - used in quick mode, default: sqlite
        #[arg(long)]
        memory: Option<String>,
    },

    /// Chat with TopClaw
    #[command(long_about = "\
Start the AI agent loop.

Launches an interactive chat session with the configured AI provider. \
Use --message for single-shot queries without entering interactive mode.

Examples:
  topclaw agent                              # interactive session
  topclaw agent -m \"Summarize today's logs\"  # single message
  topclaw agent -p anthropic --model claude-sonnet-4-20250514
  topclaw agent --peripheral nucleo-f401re:/dev/ttyACM0
  topclaw agent --autonomy-level full --max-actions-per-hour 100
  topclaw agent -m \"quick task\" --memory-backend none --compact-context")]
    Agent {
        /// Single message mode (don't enter interactive mode)
        #[arg(short, long)]
        message: Option<String>,

        /// Provider to use (openrouter, anthropic, openai, openai-codex)
        #[arg(short, long)]
        provider: Option<String>,

        /// Model to use
        #[arg(long)]
        model: Option<String>,

        /// Temperature (0.0 - 2.0)
        #[arg(short, long, default_value = "0.7", value_parser = parse_temperature)]
        temperature: f64,

        /// Attach a peripheral (board:path, e.g. nucleo-f401re:/dev/ttyACM0)
        #[arg(long)]
        peripheral: Vec<String>,

        /// Autonomy level (read_only, supervised, full)
        #[arg(long, value_parser = clap::value_parser!(security::AutonomyLevel))]
        autonomy_level: Option<security::AutonomyLevel>,

        /// Maximum shell/tool actions per hour
        #[arg(long)]
        max_actions_per_hour: Option<u32>,

        /// Maximum tool-call iterations per message
        #[arg(long)]
        max_tool_iterations: Option<usize>,

        /// Maximum conversation history messages
        #[arg(long)]
        max_history_messages: Option<usize>,

        /// Enable compact context mode (smaller prompts for limited models)
        #[arg(long)]
        compact_context: bool,

        /// Memory backend (sqlite, markdown, none)
        #[arg(long)]
        memory_backend: Option<String>,
    },

    /// Run the HTTP/WebSocket gateway
    #[command(long_about = "\
Start the gateway server (webhooks, websockets).

Runs the HTTP/WebSocket gateway that accepts incoming webhook events \
and WebSocket connections. Bind address defaults to the values in \
your config file (gateway.host / gateway.port).

Examples:
  topclaw gateway                  # use config defaults
  topclaw gateway -p 8080          # listen on port 8080
  topclaw gateway --host 0.0.0.0   # bind to all interfaces
  topclaw gateway -p 0             # random available port
  topclaw gateway --new-pairing    # clear tokens and generate fresh pairing code")]
    Gateway {
        /// Port to listen on (use 0 for random available port); defaults to config gateway.port
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to; defaults to config gateway.host
        #[arg(long)]
        host: Option<String>,

        /// Clear all paired tokens and generate a fresh pairing code
        #[arg(long)]
        new_pairing: bool,
    },

    /// Run TopClaw continuously in the background
    #[command(long_about = "\
Start the long-running autonomous daemon.

Launches the full TopClaw runtime: gateway server, all configured \
channels (Telegram, Discord, Slack, etc.), heartbeat monitor, and \
the cron scheduler. This is the recommended way to run TopClaw in \
production or as an always-on assistant.

Use 'topclaw service install' to register the daemon as an OS \
service (systemd/launchd) for auto-start on boot.

Examples:
  topclaw daemon                   # use config defaults
  topclaw daemon -p 9090           # gateway on port 9090
  topclaw daemon --host 127.0.0.1  # localhost only")]
    Daemon {
        /// Port to listen on (use 0 for random available port); defaults to config gateway.port
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to; defaults to config gateway.host
        #[arg(long)]
        host: Option<String>,
    },

    /// Manage the background service
    #[command(long_about = "\
Manage OS service lifecycle for the TopClaw daemon.

Use this command group when you want TopClaw to stay active in the background.

Examples:
  topclaw service install
  topclaw service start
  topclaw service stop
  topclaw service restart
  topclaw service status
  topclaw service uninstall")]
    Service {
        /// Init system to use: auto (detect), systemd, or openrc
        #[arg(long, default_value = "auto", value_parser = ["auto", "systemd", "openrc"])]
        service_init: String,

        #[command(subcommand)]
        service_command: ServiceCommands,
    },

    /// Run diagnostics and health checks
    Doctor {
        #[command(subcommand)]
        doctor_command: Option<DoctorCommands>,
    },

    /// Show current status and configuration summary
    Status {
        /// Include the full diagnostic report after the status summary
        #[arg(long)]
        diagnose: bool,
    },

    /// Update TopClaw to the latest release
    #[command(long_about = "\
Self-update TopClaw to the latest release from GitHub.

Downloads the appropriate pre-built binary for your platform and
replaces the current executable. Requires write permissions to
the binary location.

Safe update flow:
  1. Run `topclaw update --check` to see whether a newer version exists.
  2. Run `topclaw update` to install it.
  3. If TopClaw is running as a background service, run `topclaw service restart`.
  4. Verify with `topclaw --version`.

If the binary location is not writable, TopClaw prints a recovery path
instead of replacing the binary unsafely.

Examples:
  topclaw update              # Update to latest version
  topclaw update --check      # Check for updates without installing
  topclaw update --force      # Reinstall even if already up to date")]
    Update {
        /// Check for updates without installing
        #[arg(long)]
        check: bool,

        /// Force update even if already at latest version
        #[arg(long)]
        force: bool,
    },

    /// Create or restore a portable TopClaw backup bundle
    #[command(long_about = "\
Create or restore a portable TopClaw backup bundle.

Backups capture the resolved TopClaw config root, which includes your \
config.toml, auth state, secrets, workspace data, memories, preferences, \
and installed skills. Use this before risky upgrades or to move a working \
TopClaw setup to another machine.

Examples:
  topclaw backup create ./topclaw-backup
  topclaw backup create ~/Backups/topclaw-2026-03-08 --include-logs
  topclaw backup inspect ./topclaw-backup
  topclaw backup restore ./topclaw-backup
  topclaw backup restore ./topclaw-backup --force")]
    Backup {
        #[command(subcommand)]
        backup_command: BackupCommands,
    },

    /// Remove TopClaw from this machine
    #[command(long_about = "\
Uninstall TopClaw from this machine.

Removes background service artifacts and the installed TopClaw binary.
Use --purge to also remove ~/.topclaw config, logs, auth profiles, and workspace data.

Examples:
  topclaw uninstall
  topclaw uninstall --purge")]
    Uninstall {
        /// Remove ~/.topclaw config, logs, auth profiles, and workspace data
        #[arg(long)]
        purge: bool,
    },

    /// Engage or inspect emergency-stop protection.
    ///
    /// Examples:
    /// - `topclaw estop`
    /// - `topclaw estop --level network-kill`
    /// - `topclaw estop --level domain-block --domain "*.chase.com"`
    /// - `topclaw estop --level tool-freeze --tool shell --tool browser`
    /// - `topclaw estop status`
    /// - `topclaw estop resume --network`
    /// - `topclaw estop resume --domain "*.chase.com"`
    /// - `topclaw estop resume --tool shell`
    Estop {
        #[command(subcommand)]
        estop_command: Option<EstopSubcommands>,

        /// Level used when engaging estop from `topclaw estop`.
        #[arg(long, value_enum)]
        level: Option<EstopLevelArg>,

        /// Domain pattern(s) for `domain-block` (repeatable).
        #[arg(long = "domain")]
        domains: Vec<String>,

        /// Tool name(s) for `tool-freeze` (repeatable).
        #[arg(long = "tool")]
        tools: Vec<String>,
    },

    /// Manage security maintenance tasks
    #[command(long_about = "\
Manage security maintenance tasks.

Commands in this group maintain security-related data stores used at runtime.

Examples:
  topclaw security update-guard-corpus
  topclaw security update-guard-corpus --source builtin
  topclaw security update-guard-corpus --source ./data/security/attack-corpus-v1.jsonl
  topclaw security update-guard-corpus --source https://example.com/guard-corpus.jsonl --checksum <sha256>")]
    Security {
        #[command(subcommand)]
        security_command: SecurityCommands,
    },

    /// Manage scheduled tasks
    #[command(long_about = "\
Configure and manage scheduled tasks.

Schedule recurring, one-shot, or interval-based tasks using cron \
expressions, RFC 3339 timestamps, durations, or fixed intervals.

Cron expressions use the standard 5-field format: \
'min hour day month weekday'. Timezones default to UTC; \
override with --tz and an IANA timezone name.

Examples:
  topclaw cron list
  topclaw cron add '0 9 * * 1-5' 'Good morning' --tz America/New_York
  topclaw cron add '*/30 * * * *' 'Check system health'
  topclaw cron add-at 2025-01-15T14:00:00Z 'Send reminder'
  topclaw cron add-every 60000 'Ping heartbeat'
  topclaw cron once 30m 'Run backup in 30 minutes'
  topclaw cron pause <task-id>
  topclaw cron update <task-id> --expression '0 8 * * *' --tz Europe/London")]
    Cron {
        #[command(subcommand)]
        cron_command: CronCommands,
    },

    /// Manage model lists and defaults
    Models {
        #[command(subcommand)]
        model_command: ModelCommands,
    },

    /// List available AI providers
    Providers,

    /// Manage messaging and chat channels
    #[command(long_about = "\
Manage communication channels.

Add, remove, list, and health-check channels that connect TopClaw \
to messaging platforms. Supported channel types: telegram, discord, \
slack, whatsapp, matrix, imessage, email.

Examples:
  topclaw bootstrap --channels-only # guided channel setup without touching provider settings
  topclaw channel list
  topclaw channel doctor
  topclaw channel add telegram '{\"bot_token\":\"123456:ABC...\",\"name\":\"my-bot\",\"allowed_users\":[\"topclaw_user\"]}'
  topclaw channel remove my-bot
  topclaw channel bind-telegram topclaw_user")]
    Channel {
        #[command(subcommand)]
        channel_command: ChannelCommands,
    },

    /// Inspect available integrations
    Integrations {
        #[command(subcommand)]
        integration_command: IntegrationCommands,
    },

    /// Manage skills and skill installation
    Skills {
        #[command(subcommand)]
        skill_command: SkillCommands,
    },

    /// Manage provider authentication profiles
    Auth {
        #[command(subcommand)]
        auth_command: AuthCommands,
    },

    /// Discover connected hardware
    #[command(long_about = "\
Discover and introspect USB hardware.

Enumerate connected USB devices, identify known development boards \
(STM32 Nucleo, Arduino, ESP32), and retrieve chip information via \
probe-rs / ST-Link.

Examples:
  topclaw hardware discover
  topclaw hardware introspect /dev/ttyACM0
  topclaw hardware info --chip STM32F401RETx")]
    Hardware {
        #[command(subcommand)]
        hardware_command: topclaw::HardwareCommands,
    },

    /// Manage hardware peripherals and boards
    #[command(long_about = "\
Manage hardware peripherals.

Add, list, flash, and configure hardware boards that expose tools \
to the agent (GPIO, sensors, actuators). Supported boards: \
nucleo-f401re, rpi-gpio, esp32, arduino-uno.

Examples:
  topclaw peripheral list
  topclaw peripheral add nucleo-f401re /dev/ttyACM0
  topclaw peripheral add rpi-gpio native
  topclaw peripheral flash --port /dev/cu.usbmodem12345
  topclaw peripheral flash-nucleo")]
    Peripheral {
        #[command(subcommand)]
        peripheral_command: topclaw::PeripheralCommands,
    },

    /// Inspect and manage stored memory
    #[command(long_about = "\
Manage agent memory entries.

List, inspect, and clear memory entries stored by the agent. \
Supports filtering by category and session, pagination, and \
batch clearing with confirmation.

Examples:
  topclaw memory stats
  topclaw memory list
  topclaw memory list --category core --limit 10
  topclaw memory get <key>
  topclaw memory clear --category conversation --yes")]
    Memory {
        #[command(subcommand)]
        memory_command: MemoryCommands,
    },

    /// Inspect configuration schema
    #[command(long_about = "\
Manage TopClaw configuration.

Inspect and export configuration settings. Use 'schema' to dump \
the full JSON Schema for the config file, which documents every \
available key, type, and default value.

Examples:
  topclaw config schema              # print JSON Schema to stdout
  topclaw config schema > schema.json")]
    Config {
        #[command(subcommand)]
        config_command: ConfigCommands,
    },

    /// Manage registered workspaces
    #[command(long_about = "\
Manage workspace registry entries for multi-workspace deployments.

The workspace registry is opt-in and controlled by `[workspaces].enabled = true`
in your config.toml. Commands operate on the local filesystem registry root.

Examples:
  topclaw workspace create --name team-a
  topclaw workspace list
  topclaw workspace disable <workspace-id>
  topclaw workspace token rotate <workspace-id>
  topclaw workspace delete <workspace-id> --confirm")]
    Workspace {
        #[command(subcommand)]
        workspace_command: WorkspaceCommands,
    },

    /// Generate shell completions
    #[command(long_about = "\
Generate shell completion scripts for `topclaw`.

The script is printed to stdout so it can be sourced directly:

Examples:
  source <(topclaw completions bash)
  topclaw completions zsh > ~/.zfunc/_topclaw
  topclaw completions fish > ~/.config/fish/completions/topclaw.fish")]
    Completions {
        /// Target shell
        #[arg(value_enum)]
        shell: CompletionShell,
    },

    /// Self-improvement maintenance commands
    #[command(name = "self-improvement")]
    SelfImprovement {
        #[command(subcommand)]
        self_improvement_command: SelfImprovementCommands,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Dump the full configuration JSON Schema to stdout
    Schema,
}

#[derive(Subcommand, Debug)]
enum SelfImprovementCommands {
    /// Quarantine corrupt task state and recreate a default empty file
    RepairState,
}

#[derive(Subcommand, Debug)]
enum WorkspaceCommands {
    /// Create a workspace and print its bearer token once
    Create {
        /// Optional display name shown in `workspace list`
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered workspaces
    List,
    /// Disable an existing workspace (data preserved)
    Disable {
        /// Workspace UUID
        workspace_id: String,
    },
    /// Delete workspace data recursively (requires --confirm)
    Delete {
        /// Workspace UUID
        workspace_id: String,
        /// Acknowledge destructive deletion
        #[arg(long)]
        confirm: bool,
    },
    /// Manage workspace bearer tokens
    Token {
        #[command(subcommand)]
        token_command: WorkspaceTokenCommands,
    },
}

#[derive(Subcommand, Debug)]
enum WorkspaceTokenCommands {
    /// Rotate bearer token for a workspace and print new token once
    Rotate {
        /// Workspace UUID
        workspace_id: String,
    },
}

#[derive(Subcommand, Debug)]
enum EstopSubcommands {
    /// Print current estop status.
    Status,
    /// Resume from an engaged estop level.
    Resume {
        /// Resume only network kill.
        #[arg(long)]
        network: bool,
        /// Resume one or more blocked domain patterns.
        #[arg(long = "domain")]
        domains: Vec<String>,
        /// Resume one or more frozen tools.
        #[arg(long = "tool")]
        tools: Vec<String>,
        /// OTP code. If omitted and OTP is required, a prompt is shown.
        #[arg(long)]
        otp: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SecurityCommands {
    /// Upsert semantic prompt-injection corpus records into the configured vector collection
    UpdateGuardCorpus {
        /// Corpus source: `builtin`, filesystem path, or HTTP(S) URL
        #[arg(long)]
        source: Option<String>,
        /// Expected SHA-256 checksum (hex) for source payload verification
        #[arg(long)]
        checksum: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum AuthCommands {
    /// Login with OAuth (OpenAI Codex or Gemini)
    Login {
        /// Provider (`openai-codex` or `gemini`)
        #[arg(long)]
        provider: String,
        /// Profile name (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
        /// Use OAuth device-code flow
        #[arg(long)]
        device_code: bool,
    },
    /// Complete OAuth by pasting redirect URL or auth code
    PasteRedirect {
        /// Provider (`openai-codex`)
        #[arg(long)]
        provider: String,
        /// Profile name (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
        /// Full redirect URL or raw OAuth code
        #[arg(long)]
        input: Option<String>,
    },
    /// Paste setup token / auth token (for Anthropic subscription auth)
    PasteToken {
        /// Provider (`anthropic`)
        #[arg(long)]
        provider: String,
        /// Profile name (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
        /// Token value (if omitted, read interactively)
        #[arg(long)]
        token: Option<String>,
        /// Auth kind override (`authorization` or `api-key`)
        #[arg(long)]
        auth_kind: Option<String>,
    },
    /// Alias for `paste-token` (interactive by default)
    SetupToken {
        /// Provider (`anthropic`)
        #[arg(long)]
        provider: String,
        /// Profile name (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
    },
    /// Refresh OpenAI Codex access token using refresh token
    Refresh {
        /// Provider (`openai-codex`)
        #[arg(long)]
        provider: String,
        /// Profile name or profile id
        #[arg(long)]
        profile: Option<String>,
    },
    /// Remove auth profile
    Logout {
        /// Provider
        #[arg(long)]
        provider: String,
        /// Profile name (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
    },
    /// Set active profile for a provider
    Use {
        /// Provider
        #[arg(long)]
        provider: String,
        /// Profile name or full profile id
        #[arg(long)]
        profile: String,
    },
    /// List auth profiles
    List,
    /// Show auth status with active profile and token expiry info
    Status,
}

#[derive(Subcommand, Debug)]
enum ModelCommands {
    /// Refresh and cache provider models
    Refresh {
        /// Provider name (defaults to configured default provider)
        #[arg(long)]
        provider: Option<String>,

        /// Refresh all providers that support live model discovery
        #[arg(long)]
        all: bool,

        /// Force live refresh and ignore fresh cache
        #[arg(long)]
        force: bool,
    },
    /// List cached models for a provider
    List {
        /// Provider name (defaults to configured default provider)
        #[arg(long)]
        provider: Option<String>,
    },
    /// Set the default model in config
    Set {
        /// Model name to set as default
        model: String,
    },
    /// Show current model configuration and cache status
    Status,
}

#[derive(Subcommand, Debug)]
enum DoctorCommands {
    /// Probe model catalogs across providers and report availability
    Models {
        /// Probe a specific provider only (default: all known providers)
        #[arg(long)]
        provider: Option<String>,

        /// Prefer cached catalogs when available (skip forced live refresh)
        #[arg(long)]
        use_cache: bool,
    },
    /// Query runtime trace events (tool diagnostics and model replies)
    Traces {
        /// Show a specific trace event by id
        #[arg(long)]
        id: Option<String>,
        /// Filter list output by event type
        #[arg(long)]
        event: Option<String>,
        /// Case-insensitive text match across message/payload
        #[arg(long)]
        contains: Option<String>,
        /// Maximum number of events to display
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    // Install default crypto provider for Rustls TLS.
    // This prevents the error: "could not automatically determine the process-level CryptoProvider"
    // when both aws-lc-rs and ring features are available (or neither is explicitly selected).
    if let Err(e) = rustls::crypto::ring::default_provider().install_default() {
        eprintln!("Warning: Failed to install default crypto provider: {e:?}");
    }

    let cli = Cli::parse();

    if let Some(config_dir) = &cli.config_dir {
        if config_dir.trim().is_empty() {
            bail!("--config-dir cannot be empty");
        }
        std::env::set_var("TOPCLAW_CONFIG_DIR", config_dir);
    }

    // Completions must remain stdout-only and should not load config or initialize logging.
    // This avoids warnings/log lines corrupting sourced completion scripts.
    if let Commands::Completions { shell } = &cli.command {
        let mut stdout = std::io::stdout().lock();
        write_shell_completion(*shell, &mut stdout)?;
        return Ok(());
    }

    // Initialize logging - respects RUST_LOG env var, defaults to INFO
    let subscriber = fmt::Subscriber::builder()
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::rfc_3339())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    // Onboard runs quick setup by default, or the interactive wizard with --interactive.
    // The onboard wizard uses reqwest::blocking internally, which creates its own
    // Tokio runtime. To avoid "Cannot drop a runtime in a context where blocking is
    // not allowed", we run the wizard on a blocking thread via spawn_blocking.
    if let Commands::Onboard {
        interactive,
        force,
        channels_only,
        api_key,
        provider,
        model,
        memory,
    } = &cli.command
    {
        let interactive = *interactive;
        let force = *force;
        let channels_only = *channels_only;
        let api_key = api_key.clone();
        let provider = provider.clone();
        let model = model.clone();
        let memory = memory.clone();

        if interactive && channels_only {
            bail!("Use either --interactive or --channels-only, not both");
        }
        if channels_only
            && (api_key.is_some() || provider.is_some() || model.is_some() || memory.is_some())
        {
            bail!("--channels-only does not accept --api-key, --provider, --model, or --memory");
        }
        if channels_only && force {
            bail!("--channels-only does not accept --force");
        }
        let config = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("failed to create onboarding runtime")?;

            if channels_only {
                runtime.block_on(onboard::run_channels_repair_wizard())
            } else if interactive {
                runtime.block_on(onboard::run_wizard(force))
            } else {
                runtime.block_on(onboard::run_quick_setup(
                    api_key.as_deref(),
                    provider.as_deref(),
                    model.as_deref(),
                    memory.as_deref(),
                    force,
                ))
            }
        })
        .await
        .context("onboarding task panicked")??;
        let _ = config;
        return Ok(());
    }

    match &cli.command {
        Commands::Uninstall { purge } => return handle_uninstall_command(*purge),
        Commands::Backup { backup_command } => {
            return backup::handle_command(backup_command.clone()).await;
        }
        _ => {}
    }

    // All other commands need config loaded first
    let mut config = Config::load_or_init().await?;
    config.apply_env_overrides();
    observability::runtime_trace::init_from_config(&config.observability, &config.workspace_dir);
    if config.security.otp.enabled {
        let config_dir = config
            .config_path
            .parent()
            .context("Config path must have a parent directory")?;
        let store = security::SecretStore::new(config_dir, config.secrets.encrypt);
        let (_validator, enrollment_uri) =
            security::OtpValidator::from_config(&config.security.otp, config_dir, &store)?;
        if let Some(uri) = enrollment_uri {
            println!("Initialized OTP secret for TopClaw.");
            println!("Enrollment URI: {uri}");
        }
    }

    match cli.command {
        Commands::Onboard { .. }
        | Commands::Completions { .. }
        | Commands::Uninstall { .. }
        | Commands::Backup { .. } => unreachable!(),

        Commands::Agent {
            message,
            provider,
            model,
            temperature,
            peripheral,
            autonomy_level,
            max_actions_per_hour,
            max_tool_iterations,
            max_history_messages,
            compact_context,
            memory_backend,
        } => {
            if let Some(level) = autonomy_level {
                config.autonomy.level = level;
            }
            if let Some(n) = max_actions_per_hour {
                config.autonomy.max_actions_per_hour = n;
            }
            if let Some(n) = max_tool_iterations {
                config.agent.max_tool_iterations = n;
            }
            if let Some(n) = max_history_messages {
                config.agent.max_history_messages = n;
            }
            if compact_context {
                config.agent.compact_context = true;
            }
            if let Some(ref backend) = memory_backend {
                config.memory.backend = backend.clone();
            }
            Box::pin(agent::run(
                config,
                message,
                provider,
                model,
                temperature,
                peripheral,
                true,
            ))
            .await
            .map(|_| ())
        }

        Commands::Gateway {
            port,
            host,
            new_pairing,
        } => {
            if new_pairing {
                // Persist token reset from raw config so env-derived overrides are not written to disk.
                let mut persisted_config = Config::load_or_init().await?;
                persisted_config.gateway.paired_tokens.clear();
                persisted_config.save().await?;
                config.gateway.paired_tokens.clear();
                info!("🔐 Cleared paired tokens — a fresh pairing code will be generated");
            }
            let port = port.unwrap_or(config.gateway.port);
            let host = host.unwrap_or_else(|| config.gateway.host.clone());
            if port == 0 {
                info!("🚀 Starting TopClaw Gateway on {host} (random port)");
            } else {
                info!("🚀 Starting TopClaw Gateway on {host}:{port}");
            }
            gateway::run_gateway(&host, port, config).await
        }

        Commands::Daemon { port, host } => {
            let port = port.unwrap_or(config.gateway.port);
            let host = host.unwrap_or_else(|| config.gateway.host.clone());
            if port == 0 {
                info!("🧠 Starting TopClaw Daemon on {host} (random port)");
            } else {
                info!("🧠 Starting TopClaw Daemon on {host}:{port}");
            }
            daemon::run(config, host, port).await
        }

        Commands::Status { diagnose } => {
            let diag_results = doctor::diagnose(&config);
            let provider_ready = provider_ready(&config);
            let channels_configured = channels_configured(&config);
            let daemon_ready = daemon_ready(&diag_results);
            let overall_ready = provider_ready && (!channels_configured || daemon_ready);
            println!("🦀 TopClaw Status");
            println!();
            println!("Version:     {}", env!("CARGO_PKG_VERSION"));
            println!("Workspace:   {}", config.workspace_dir.display());
            println!("Config:      {}", config.config_path.display());
            println!();
            println!("Readiness:");
            println!(
                "  Provider:   {}",
                if provider_ready {
                    "✅ ready"
                } else {
                    "⚠️  auth or API key needed"
                }
            );
            println!(
                "  Channels:   {}",
                if channels_configured {
                    "✅ configured"
                } else {
                    "ℹ️  CLI only"
                }
            );
            println!(
                "  Runtime:    {}",
                if channels_configured {
                    if daemon_ready {
                        "✅ background runtime looks healthy"
                    } else {
                        "⚠️  channels configured but background runtime is not healthy"
                    }
                } else {
                    "ℹ️  background runtime not required"
                }
            );
            println!(
                "  Overall:    {}",
                if overall_ready {
                    "✅ ready"
                } else {
                    "⚠️  action needed"
                }
            );
            println!();
            println!(
                "🤖 Provider:      {}",
                config.default_provider.as_deref().unwrap_or("openrouter")
            );
            println!(
                "   Model:         {}",
                config.default_model.as_deref().unwrap_or("(default)")
            );
            println!("📊 Observability:  {}", config.observability.backend);
            println!(
                "🧾 Trace storage:  {} ({})",
                config.observability.runtime_trace_mode, config.observability.runtime_trace_path
            );
            println!("🛡️  Autonomy:      {:?}", config.autonomy.level);
            println!("⚙️  Runtime:       {}", config.runtime.kind);
            let effective_memory_backend = memory::effective_memory_backend_name(
                &config.memory.backend,
                Some(&config.storage.provider.config),
            );
            println!(
                "💓 Heartbeat:      {}",
                if config.heartbeat.enabled {
                    format!("every {}min", config.heartbeat.interval_minutes)
                } else {
                    "disabled".into()
                }
            );
            println!(
                "🧠 Memory:         {} (auto-save: {})",
                effective_memory_backend,
                if config.memory.auto_save { "on" } else { "off" }
            );

            println!();
            println!("Security:");
            println!("  Workspace only:    {}", config.autonomy.workspace_only);
            println!(
                "  Allowed roots:     {}",
                if config.autonomy.allowed_roots.is_empty() {
                    "(none)".to_string()
                } else {
                    config.autonomy.allowed_roots.join(", ")
                }
            );
            println!(
                "  Allowed commands:  {}",
                config.autonomy.allowed_commands.join(", ")
            );
            println!(
                "  Max actions/hour:  {}",
                config.autonomy.max_actions_per_hour
            );
            println!(
                "  Max cost/day:      ${:.2}",
                f64::from(config.autonomy.max_cost_per_day_cents) / 100.0
            );
            println!("  OTP enabled:       {}", config.security.otp.enabled);
            println!("  E-stop enabled:    {}", config.security.estop.enabled);
            println!();
            println!("Channels:");
            println!("  CLI:      ✅ always");
            for (channel, configured) in config.channels_config.channels() {
                println!(
                    "  {:9} {}",
                    channel.name(),
                    if configured {
                        "✅ configured"
                    } else {
                        "❌ not configured"
                    }
                );
            }
            println!();
            println!("Peripherals:");
            println!(
                "  Enabled:   {}",
                if config.peripherals.enabled {
                    "yes"
                } else {
                    "no"
                }
            );
            println!("  Boards:    {}", config.peripherals.boards.len());

            if diagnose {
                println!();
                doctor::print_report(&diag_results);
            }
            doctor::print_next_step_suggestions(&config, &diag_results);

            Ok(())
        }

        Commands::Update { check, force } => {
            update::self_update(force, check).await?;
            Ok(())
        }

        Commands::Estop {
            estop_command,
            level,
            domains,
            tools,
        } => handle_estop_command(&config, estop_command, level, domains, tools),

        Commands::Security { security_command } => {
            handle_security_command(&config, security_command).await
        }

        Commands::Cron { cron_command } => cron::handle_command(cron_command, &config),

        Commands::Models { model_command } => match model_command {
            ModelCommands::Refresh {
                provider,
                all,
                force,
            } => {
                if all {
                    if provider.is_some() {
                        bail!("`models refresh --all` cannot be combined with --provider");
                    }
                    onboard::run_models_refresh_all(&config, force).await
                } else {
                    onboard::run_models_refresh(&config, provider.as_deref(), force).await
                }
            }
            ModelCommands::List { provider } => {
                onboard::run_models_list(&config, provider.as_deref()).await
            }
            ModelCommands::Set { model } => onboard::run_models_set(&config, &model).await,
            ModelCommands::Status => onboard::run_models_status(&config).await,
        },

        Commands::Providers => {
            let providers = providers::list_providers();
            let current = config
                .default_provider
                .as_deref()
                .unwrap_or("openrouter")
                .trim()
                .to_ascii_lowercase();
            println!("Supported providers ({} total):\n", providers.len());
            println!("  ID (use in config)  DESCRIPTION");
            println!("  ─────────────────── ───────────");
            for p in &providers {
                let is_active = p.name.eq_ignore_ascii_case(&current)
                    || p.aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(&current));
                let marker = if is_active { " (active)" } else { "" };
                let local_tag = if p.local { " [local]" } else { "" };
                let aliases = if p.aliases.is_empty() {
                    String::new()
                } else {
                    format!("  (aliases: {})", p.aliases.join(", "))
                };
                println!(
                    "  {:<19} {}{}{}{}",
                    p.name, p.display_name, local_tag, marker, aliases
                );
            }
            println!("\n  custom:<URL>   Any OpenAI-compatible endpoint");
            println!("  anthropic-custom:<URL>  Any Anthropic-compatible endpoint");
            Ok(())
        }

        Commands::Service {
            service_command,
            service_init,
        } => {
            let init_system = service_init.parse()?;
            service::handle_command(&service_command, &config, init_system)
        }

        Commands::Doctor { doctor_command } => match doctor_command {
            Some(DoctorCommands::Models {
                provider,
                use_cache,
            }) => doctor::run_models(&config, provider.as_deref(), use_cache).await,
            Some(DoctorCommands::Traces {
                id,
                event,
                contains,
                limit,
            }) => doctor::run_traces(
                &config,
                id.as_deref(),
                event.as_deref(),
                contains.as_deref(),
                limit,
            ),
            None => doctor::run(&config),
        },

        Commands::Channel { channel_command } => match channel_command {
            ChannelCommands::Start => Box::pin(channels::start_channels(config)).await,
            ChannelCommands::Doctor => channels::doctor_channels(config).await,
            other => channels::handle_command(other, &config).await,
        },

        Commands::Integrations {
            integration_command,
        } => integrations::handle_command(integration_command, &config),

        Commands::Skills { skill_command } => skills::handle_command(skill_command, &config),

        Commands::Memory { memory_command } => {
            memory::cli::handle_command(memory_command, &config).await
        }

        Commands::Auth { auth_command } => handle_auth_command(auth_command, &config).await,

        Commands::Hardware { hardware_command } => {
            hardware::handle_command(hardware_command.clone(), &config)
        }

        Commands::Peripheral { peripheral_command } => {
            peripherals::handle_command(peripheral_command.clone(), &config).await
        }

        Commands::Config { config_command } => match config_command {
            ConfigCommands::Schema => {
                let schema = schemars::schema_for!(config::Config);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&schema).expect("failed to serialize JSON Schema")
                );
                Ok(())
            }
        },

        Commands::Workspace { workspace_command } => {
            handle_workspace_command(workspace_command, &config)
        }

        Commands::SelfImprovement {
            self_improvement_command,
        } => match self_improvement_command {
            SelfImprovementCommands::RepairState => {
                let manager = self_improvement::SelfImprovementManager::new(&config.workspace_dir);
                let outcome = manager.repair_state_file().await?;
                println!("{}", serde_json::to_string_pretty(&outcome)?);
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn cli_definition_has_no_flag_conflicts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn bootstrap_help_includes_model_flag() {
        let cmd = Cli::command();
        let bootstrap = cmd
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "bootstrap")
            .expect("bootstrap subcommand must exist");

        let has_model_flag = bootstrap
            .get_arguments()
            .any(|arg| arg.get_id().as_str() == "model" && arg.get_long() == Some("model"));

        assert!(
            has_model_flag,
            "bootstrap help should include --model for quick setup overrides"
        );
    }

    #[test]
    fn bootstrap_cli_accepts_model_provider_and_api_key_in_quick_mode() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "bootstrap",
            "--provider",
            "openrouter",
            "--model",
            "custom-model-946",
            "--api-key",
            "sk-issue946",
        ])
        .expect("quick onboard invocation should parse");

        match cli.command {
            Commands::Onboard {
                interactive,
                force,
                channels_only,
                api_key,
                provider,
                model,
                ..
            } => {
                assert!(!interactive);
                assert!(!force);
                assert!(!channels_only);
                assert_eq!(provider.as_deref(), Some("openrouter"));
                assert_eq!(model.as_deref(), Some("custom-model-946"));
                assert_eq!(api_key.as_deref(), Some("sk-issue946"));
            }
            other => panic!("expected bootstrap command, got {other:?}"),
        }
    }

    #[test]
    fn completions_cli_parses_supported_shells() {
        for shell in ["bash", "fish", "zsh", "powershell", "elvish"] {
            let cli = Cli::try_parse_from(["topclaw", "completions", shell])
                .expect("completions invocation should parse");
            match cli.command {
                Commands::Completions { .. } => {}
                other => panic!("expected completions command, got {other:?}"),
            }
        }
    }

    #[test]
    fn gateway_help_includes_new_pairing_flag() {
        let cmd = Cli::command();
        let gateway = cmd
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "gateway")
            .expect("gateway subcommand must exist");

        let has_new_pairing_flag = gateway.get_arguments().any(|arg| {
            arg.get_id().as_str() == "new_pairing" && arg.get_long() == Some("new-pairing")
        });

        assert!(
            has_new_pairing_flag,
            "gateway help should include --new-pairing"
        );
    }

    #[test]
    fn gateway_cli_accepts_new_pairing_flag() {
        let cli = Cli::try_parse_from(["topclaw", "gateway", "--new-pairing"])
            .expect("gateway --new-pairing should parse");

        match cli.command {
            Commands::Gateway { new_pairing, .. } => assert!(new_pairing),
            other => panic!("expected gateway command, got {other:?}"),
        }
    }

    #[test]
    fn gateway_cli_defaults_new_pairing_to_false() {
        let cli = Cli::try_parse_from(["topclaw", "gateway"]).expect("gateway should parse");

        match cli.command {
            Commands::Gateway { new_pairing, .. } => assert!(!new_pairing),
            other => panic!("expected gateway command, got {other:?}"),
        }
    }

    #[test]
    fn completion_generation_mentions_binary_name() {
        let mut output = Vec::new();
        write_shell_completion(CompletionShell::Bash, &mut output)
            .expect("completion generation should succeed");
        let script = String::from_utf8(output).expect("completion output should be valid utf-8");
        assert!(
            script.contains("topclaw"),
            "completion script should reference binary name"
        );
    }

    #[test]
    fn bootstrap_cli_accepts_force_flag() {
        let cli = Cli::try_parse_from(["topclaw", "bootstrap", "--force"])
            .expect("bootstrap --force should parse");

        match cli.command {
            Commands::Onboard { force, .. } => assert!(force),
            other => panic!("expected bootstrap command, got {other:?}"),
        }
    }

    #[test]
    fn status_cli_accepts_diagnose_flag() {
        let cli =
            Cli::try_parse_from(["topclaw", "status", "--diagnose"]).expect("status should parse");

        match cli.command {
            Commands::Status { diagnose } => assert!(diagnose),
            other => panic!("expected status command, got {other:?}"),
        }
    }

    #[test]
    fn doctor_cli_parses_default_command() {
        let cli = Cli::try_parse_from(["topclaw", "doctor"]).expect("doctor should parse");

        match cli.command {
            Commands::Doctor { .. } => {}
            other => panic!("expected doctor command, got {other:?}"),
        }
    }

    #[test]
    fn uninstall_cli_accepts_purge_flag() {
        let cli = Cli::try_parse_from(["topclaw", "uninstall", "--purge"])
            .expect("uninstall --purge should parse");

        match cli.command {
            Commands::Uninstall { purge } => assert!(purge),
            other => panic!("expected uninstall command, got {other:?}"),
        }
    }

    #[test]
    fn backup_create_cli_accepts_destination_and_logs_flag() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "backup",
            "create",
            "./topclaw-backup",
            "--include-logs",
        ])
        .expect("backup create should parse");

        match cli.command {
            Commands::Backup {
                backup_command:
                    topclaw::BackupCommands::Create {
                        destination,
                        include_logs,
                    },
            } => {
                assert_eq!(destination, std::path::PathBuf::from("./topclaw-backup"));
                assert!(include_logs);
            }
            other => panic!("expected backup create command, got {other:?}"),
        }
    }

    #[test]
    fn backup_restore_cli_accepts_force_flag() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "backup",
            "restore",
            "./topclaw-backup",
            "--force",
        ])
        .expect("backup restore should parse");

        match cli.command {
            Commands::Backup {
                backup_command: topclaw::BackupCommands::Restore { source, force },
            } => {
                assert_eq!(source, std::path::PathBuf::from("./topclaw-backup"));
                assert!(force);
            }
            other => panic!("expected backup restore command, got {other:?}"),
        }
    }

    #[test]
    fn backup_inspect_cli_accepts_source() {
        let cli = Cli::try_parse_from(["topclaw", "backup", "inspect", "./topclaw-backup"])
            .expect("backup inspect should parse");

        match cli.command {
            Commands::Backup {
                backup_command: topclaw::BackupCommands::Inspect { source },
            } => {
                assert_eq!(source, std::path::PathBuf::from("./topclaw-backup"));
            }
            other => panic!("expected backup inspect command, got {other:?}"),
        }
    }

    #[test]
    fn skills_vet_cli_accepts_json_and_sandbox_flags() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "skills",
            "vet",
            "find-skills",
            "--json",
            "--sandbox",
            "docker",
        ])
        .expect("skills vet flags should parse");

        match cli.command {
            Commands::Skills {
                skill_command:
                    topclaw::SkillCommands::Vet {
                        source,
                        json,
                        sandbox,
                    },
            } => {
                assert_eq!(source, "find-skills");
                assert!(json);
                assert_eq!(sandbox.as_deref(), Some("docker"));
            }
            other => panic!("expected skills vet command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_estop_default_engage() {
        let cli = Cli::try_parse_from(["topclaw", "estop"]).expect("estop command should parse");

        match cli.command {
            Commands::Estop {
                estop_command,
                level,
                domains,
                tools,
            } => {
                assert!(estop_command.is_none());
                assert!(level.is_none());
                assert!(domains.is_empty());
                assert!(tools.is_empty());
            }
            other => panic!("expected estop command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_estop_resume_domain() {
        let cli = Cli::try_parse_from(["topclaw", "estop", "resume", "--domain", "*.chase.com"])
            .expect("estop resume command should parse");

        match cli.command {
            Commands::Estop {
                estop_command: Some(EstopSubcommands::Resume { domains, .. }),
                ..
            } => assert_eq!(domains, vec!["*.chase.com".to_string()]),
            other => panic!("expected estop resume command, got {other:?}"),
        }
    }

    #[test]
    fn workspace_cli_parses_create_with_name() {
        let cli = Cli::try_parse_from(["topclaw", "workspace", "create", "--name", "team-a"])
            .expect("workspace create should parse");

        match cli.command {
            Commands::Workspace {
                workspace_command: WorkspaceCommands::Create { name },
            } => assert_eq!(name.as_deref(), Some("team-a")),
            other => panic!("expected workspace create command, got {other:?}"),
        }
    }

    #[test]
    fn workspace_cli_parses_token_rotate() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "workspace",
            "token",
            "rotate",
            "550e8400-e29b-41d4-a716-446655440000",
        ])
        .expect("workspace token rotate should parse");

        match cli.command {
            Commands::Workspace {
                workspace_command:
                    WorkspaceCommands::Token {
                        token_command: WorkspaceTokenCommands::Rotate { workspace_id },
                    },
            } => assert_eq!(workspace_id, "550e8400-e29b-41d4-a716-446655440000"),
            other => panic!("expected workspace token rotate command, got {other:?}"),
        }
    }

    #[test]
    fn workspace_cli_delete_requires_explicit_confirm_flag_value() {
        let cli = Cli::try_parse_from([
            "topclaw",
            "workspace",
            "delete",
            "550e8400-e29b-41d4-a716-446655440000",
            "--confirm",
        ])
        .expect("workspace delete should parse");

        match cli.command {
            Commands::Workspace {
                workspace_command:
                    WorkspaceCommands::Delete {
                        workspace_id,
                        confirm,
                    },
            } => {
                assert_eq!(workspace_id, "550e8400-e29b-41d4-a716-446655440000");
                assert!(confirm);
            }
            other => panic!("expected workspace delete command, got {other:?}"),
        }
    }

    #[test]
    fn self_improvement_cli_parses_repair_state() {
        let cli = Cli::try_parse_from(["topclaw", "self-improvement", "repair-state"])
            .expect("self-improvement repair-state should parse");

        match cli.command {
            Commands::SelfImprovement {
                self_improvement_command: SelfImprovementCommands::RepairState,
            } => {}
            other => panic!("expected self-improvement repair-state command, got {other:?}"),
        }
    }
}
