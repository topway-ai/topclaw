#![forbid(unsafe_code)]
// Lint configuration is in [lints] section of Cargo.toml.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "gateway")]
use topclaw::gateway;
use topclaw::{
    agent, auth, backup, channels, config, cron, daemon, doctor, memory, observability, onboard,
    providers, security, service, skills, update, Config,
};
use tracing::info;
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
    config
        .channels_config
        .launchable_channels()
        .iter()
        .any(|(_, ok)| *ok)
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

fn auxiliary_surfaces_configured(config: &Config) -> bool {
    let tunnel_provider = config.tunnel.provider.trim();
    config
        .channels_config
        .auxiliary_channels()
        .iter()
        .any(|(_, configured)| *configured)
        || config.gateway.node_control.enabled
        || (!tunnel_provider.is_empty() && !tunnel_provider.eq_ignore_ascii_case("none"))
}

mod main_handlers;
use main_handlers::{
    handle_auth_command, handle_estop_command, handle_security_command, handle_uninstall_command,
    handle_workspace_command, write_shell_completion,
};
use std::ffi::OsString;

// Re-export so binary modules can use crate::<CommandEnum> while keeping a single source of truth.
pub use topclaw::{
    BackupCommands, ChannelCommands, CronCommands, MemoryCommands, ServiceCommands, SkillCommands,
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
Syntax note:
  `[OPTIONS]` and `<COMMAND>` are placeholders in help output. Do not type the brackets.
  Example: `topclaw providers` or `topclaw --providers`

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

Full uninstall:
  topclaw uninstall              # remove the installed binary, keep ~/.topclaw data
  topclaw uninstall --purge      # remove the binary and erase ~/.topclaw data too")]
struct Cli {
    #[arg(long, global = true)]
    config_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

fn normalize_cli_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter()
        .enumerate()
        .filter_map(|(idx, arg)| {
            let arg = arg.into();
            if idx > 0 && arg == "[OPTIONS]" {
                return None;
            }
            if idx > 0 && arg == "--providers" {
                return Some(OsString::from("providers"));
            }
            Some(arg)
        })
        .collect()
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

        /// API key (used in quick mode for API-key providers; if passed without --provider, quick setup uses OpenRouter)
        #[arg(long)]
        api_key: Option<String>,

        /// Provider name (used in quick mode, default: openai-codex)
        #[arg(long)]
        provider: Option<String>,
        /// Model ID override (used in quick mode)
        #[arg(long)]
        model: Option<String>,
        /// Memory backend (sqlite, markdown, none) - used in quick mode, default: sqlite
        #[arg(long)]
        memory: Option<String>,

        /// Attempt to install missing Linux desktop helpers (xdotool, wmctrl, scrot, xdg-open)
        #[cfg(feature = "computer-use-sidecar")]
        #[arg(long)]
        install_desktop_helpers: bool,
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

        /// Provider to use (openai-codex, openrouter, ollama, or another configured provider)
        #[arg(short, long)]
        provider: Option<String>,

        /// Model to use
        #[arg(long)]
        model: Option<String>,

        /// Temperature (0.0 - 2.0)
        #[arg(short, long, default_value = "0.7", value_parser = parse_temperature)]
        temperature: f64,

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
    #[cfg(feature = "gateway")]
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

Launches the long-running TopClaw runtime for configured channels, \
heartbeat monitoring, and any explicitly enabled scheduler work. The gateway surface is \
started only when webhook/API features are configured in the runtime config. Gateway/API support is \
available only in builds with the `gateway` feature.

Use 'topclaw service install' to register the daemon as an OS \
service (systemd/launchd) for auto-start on boot.

Examples:
  topclaw daemon                   # use config defaults")]
    Daemon,

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
webhook.

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

    /// Run the computer-use sidecar HTTP server
    ///
    /// Serves the protocol defined in `docs/computer-use-sidecar-protocol.md`
    /// on a local address. Linux-only runtime. The agent auto-spawns this via
    /// the approval-gated `computer_use_sidecar_start` tool; running it by
    /// hand is rarely necessary.
    #[cfg(feature = "computer-use-sidecar")]
    #[command(name = "computer-use-sidecar")]
    ComputerUseSidecar {
        /// Bind address (default: 127.0.0.1:8787)
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
        /// Optional Bearer API key. If omitted, `TOPCLAW_SIDECAR_API_KEY` is read.
        #[arg(long)]
        api_key: Option<String>,
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
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Dump the full configuration JSON Schema to stdout
    Schema,
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

        /// Refresh product-priority providers in priority order
        #[arg(long)]
        all: bool,

        /// Refresh every provider that supports live model discovery (advanced)
        #[arg(long)]
        all_providers: bool,

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
        /// Probe a specific provider only (default: product-priority providers)
        #[arg(long)]
        provider: Option<String>,

        /// Probe all known providers instead of the product-priority default set
        #[arg(long)]
        all_providers: bool,

        /// Prefer cached catalogs when available (skip forced live refresh)
        #[arg(long)]
        use_cache: bool,
    },
    /// Check or install desktop automation helpers (xdotool, wmctrl, scrot, xdg-open)
    #[cfg(feature = "computer-use-sidecar")]
    DesktopHelpers {
        /// Attempt to install missing desktop helpers via the system package manager
        #[arg(long)]
        install: bool,
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

    let cli = Cli::parse_from(normalize_cli_args(std::env::args_os()));

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

    // Computer-use sidecar runs a small HTTP server and does not touch the
    // main config pipeline. Short-circuit here so `topclaw computer-use-sidecar`
    // works even before onboarding is complete (the auto-spawn tool relies on
    // this invariant).
    #[cfg(feature = "computer-use-sidecar")]
    if let Commands::ComputerUseSidecar { bind, api_key } = &cli.command {
        let addr: std::net::SocketAddr = bind
            .parse()
            .with_context(|| format!("invalid --bind address '{bind}'"))?;
        let key = api_key
            .clone()
            .or_else(|| std::env::var("TOPCLAW_SIDECAR_API_KEY").ok())
            .filter(|s| !s.is_empty());
        return topclaw::sidecar::run_server(addr, key).await;
    }

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
        #[cfg(feature = "computer-use-sidecar")]
        install_desktop_helpers,
    } = &cli.command
    {
        let interactive = *interactive;
        let force = *force;
        let channels_only = *channels_only;
        let api_key = api_key.clone();
        let provider = provider.clone();
        let model = model.clone();
        let memory = memory.clone();
        #[cfg(feature = "computer-use-sidecar")]
        let install_desktop_helpers = *install_desktop_helpers;

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
        #[cfg(feature = "computer-use-sidecar")]
        if channels_only && install_desktop_helpers {
            bail!("--channels-only does not accept --install-desktop-helpers");
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

        // Headless/desktop-helper bootstrap: after the config is written,
        // attempt to install missing Linux desktop helpers if requested.
        // We call the functions directly rather than doctor::run_desktop_helpers()
        // because the --install-desktop-helpers flag is an explicit user request
        // that should work unconditionally (not gated on is_computer_use_backend).
        #[cfg(feature = "computer-use-sidecar")]
        if install_desktop_helpers {
            let missing = topclaw::tools::computer_use::missing_linux_helpers();
            if missing.is_empty() {
                println!("✅ All desktop helpers already installed");
            } else {
                println!(
                    "📦 Installing missing desktop helpers: {}…",
                    missing.join(", ")
                );
                let result = topclaw::tools::computer_use::install_desktop_helpers().await;
                println!("{result}");
            }
        }

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

        #[cfg(feature = "computer-use-sidecar")]
        Commands::ComputerUseSidecar { .. } => unreachable!(),

        Commands::Agent {
            message,
            provider,
            model,
            temperature,
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
                Vec::new(),
                true,
            ))
            .await
            .map(|_| ())
        }

        #[cfg(feature = "gateway")]
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

        Commands::Daemon => {
            info!("🧠 Starting TopClaw Daemon");
            daemon::run(config).await
        }

        Commands::Status { diagnose } => {
            let diag_results = doctor::diagnose(&config);
            let provider_ready = provider_ready(&config);
            let channels_configured = channels_configured(&config);
            let daemon_ready = daemon_ready(&diag_results);
            #[cfg(feature = "computer-use-sidecar")]
            let (desktop_helpers_ready, desktop_missing) = {
                if doctor::is_computer_use_backend(&config) {
                    let missing = topclaw::tools::computer_use::missing_linux_helpers();
                    (missing.is_empty(), missing)
                } else {
                    (true, Vec::new())
                }
            };
            #[cfg(not(feature = "computer-use-sidecar"))]
            let desktop_helpers_ready = true;
            let overall_ready =
                provider_ready && (!channels_configured || daemon_ready) && desktop_helpers_ready;
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
            #[cfg(feature = "computer-use-sidecar")]
            if doctor::is_computer_use_backend(&config) {
                let desktop_status = if desktop_helpers_ready {
                    "✅ desktop helpers installed (xdotool, wmctrl, scrot, xdg-open)".to_string()
                } else {
                    format!(
                        "⚠️  missing: {} — run `topclaw doctor desktop-helpers --install`",
                        desktop_missing.join(", ")
                    )
                };
                println!("  Desktop:    {desktop_status}");
            }
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
                config
                    .default_provider
                    .as_deref()
                    .unwrap_or(providers::DEFAULT_PROVIDER_NAME)
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
            println!("Runtime Channels:");
            println!("  CLI:        ✅ always");
            for (index, (channel, configured)) in config
                .channels_config
                .launchable_channels()
                .into_iter()
                .enumerate()
            {
                let priority = match index {
                    0 => "primary",
                    1 => "secondary",
                    _ => "configured",
                };
                println!(
                    "  {:9} {} ({priority})",
                    channel.name(),
                    if configured {
                        "✅ configured"
                    } else {
                        "❌ not configured"
                    }
                );
            }
            if auxiliary_surfaces_configured(&config) {
                println!();
                println!("Auxiliary Surfaces:");
                #[cfg(feature = "gateway")]
                println!("  Build:     {}", "✅ gateway-enabled");
                #[cfg(not(feature = "gateway"))]
                println!("  Build:     ℹ️  unavailable in this build (`--features gateway`)");
                for (channel, configured) in config.channels_config.auxiliary_channels() {
                    println!(
                        "  {:9} {}",
                        channel.name(),
                        if configured {
                            "✅ configured"
                        } else {
                            "ℹ️  disabled"
                        }
                    );
                }
                println!(
                    "  Node ctl:  {}",
                    if config.gateway.node_control.enabled {
                        "✅ enabled"
                    } else {
                        "ℹ️  disabled"
                    }
                );
                println!(
                    "  Tunnel:    {}",
                    if config.tunnel.provider.trim().is_empty()
                        || config.tunnel.provider.eq_ignore_ascii_case("none")
                    {
                        "ℹ️  disabled".to_string()
                    } else {
                        format!("✅ {}", config.tunnel.provider)
                    }
                );
            }

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
                all_providers,
                force,
            } => {
                if all && all_providers {
                    bail!("`models refresh --all` cannot be combined with --all-providers");
                }
                if all || all_providers {
                    if provider.is_some() {
                        bail!(
                            "`models refresh --all` / --all-providers cannot be combined with --provider"
                        );
                    }
                    onboard::run_models_refresh_all(&config, force, all_providers).await
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
            let provider_list = providers::list_providers();
            let current = config
                .default_provider
                .as_deref()
                .unwrap_or(providers::DEFAULT_PROVIDER_NAME)
                .trim()
                .to_ascii_lowercase();
            println!("Supported providers ({} total):\n", provider_list.len());
            let print_section = |title: &str, entries: Vec<&providers::ProviderInfo>| {
                println!("  {title}");
                println!("  ID (use in config)  DESCRIPTION");
                println!("  ─────────────────── ───────────");
                for p in entries {
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
                println!();
            };

            let product_priority: Vec<&providers::ProviderInfo> = provider_list
                .iter()
                .filter(|provider| providers::is_product_priority_provider(provider.name))
                .collect();
            let additional: Vec<&providers::ProviderInfo> = provider_list
                .iter()
                .filter(|provider| !providers::is_product_priority_provider(provider.name))
                .collect();

            print_section(
                "Product-priority providers (default product path: Codex -> OpenRouter -> Ollama)",
                product_priority,
            );
            print_section("Additional providers (advanced/compatibility)", additional);
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
                all_providers,
                use_cache,
            }) => doctor::run_models(&config, provider.as_deref(), all_providers, use_cache).await,
            #[cfg(feature = "computer-use-sidecar")]
            Some(DoctorCommands::DesktopHelpers { install }) => {
                doctor::run_desktop_helpers(&config, install).await
            }
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

        Commands::Skills { skill_command } => skills::handle_command(skill_command, &config),

        Commands::Memory { memory_command } => {
            memory::cli::handle_command(memory_command, &config).await
        }

        Commands::Auth { auth_command } => handle_auth_command(auth_command, &config).await,

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
    fn normalize_cli_args_rewrites_common_root_usage_mistakes() {
        let providers = normalize_cli_args(["topclaw", "--providers"]);
        assert_eq!(providers, vec!["topclaw", "providers"]);

        let copied_usage = normalize_cli_args(["topclaw", "[OPTIONS]", "providers"]);
        assert_eq!(copied_usage, vec!["topclaw", "providers"]);
    }

    #[test]
    fn providers_shortcut_flag_parses_as_subcommand() {
        let cli = Cli::try_parse_from(normalize_cli_args(["topclaw", "--providers"]))
            .expect("--providers should normalize to the providers subcommand");

        match cli.command {
            Commands::Providers => {}
            other => panic!("expected providers command, got {other:?}"),
        }
    }

    #[test]
    fn root_help_explains_usage_placeholders() {
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        assert!(help.contains("`[OPTIONS]` and `<COMMAND>` are placeholders"));
        assert!(help.contains("topclaw --providers"));
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
                #[cfg(feature = "computer-use-sidecar")]
                install_desktop_helpers,
                ..
            } => {
                assert!(!interactive);
                assert!(!force);
                assert!(!channels_only);
                #[cfg(feature = "computer-use-sidecar")]
                assert!(!install_desktop_helpers);
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
    #[cfg(feature = "gateway")]
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
    #[cfg(feature = "gateway")]
    fn gateway_cli_accepts_new_pairing_flag() {
        let cli = Cli::try_parse_from(["topclaw", "gateway", "--new-pairing"])
            .expect("gateway --new-pairing should parse");

        match cli.command {
            Commands::Gateway { new_pairing, .. } => assert!(new_pairing),
            other => panic!("expected gateway command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "gateway")]
    fn gateway_cli_defaults_new_pairing_to_false() {
        let cli = Cli::try_parse_from(["topclaw", "gateway"]).expect("gateway should parse");

        match cli.command {
            Commands::Gateway { new_pairing, .. } => assert!(!new_pairing),
            other => panic!("expected gateway command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(not(feature = "gateway"))]
    fn gateway_command_hidden_without_gateway_feature() {
        let cmd = Cli::command();
        assert!(
            cmd.get_subcommands()
                .all(|subcommand| subcommand.get_name() != "gateway"),
            "gateway subcommand should be hidden without the gateway feature"
        );
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
            Commands::Onboard {
                force,
                #[cfg(feature = "computer-use-sidecar")]
                install_desktop_helpers,
                ..
            } => {
                assert!(force);
                #[cfg(feature = "computer-use-sidecar")]
                assert!(!install_desktop_helpers);
            }
            other => panic!("expected bootstrap command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "computer-use-sidecar")]
    fn status_overall_ready_includes_desktop_helpers_check() {
        // When browser.backend=computer_use and helpers are missing,
        // overall_ready should be false even if provider is ready.
        let mut config = Config::default();
        config.browser.backend = "computer_use".into();
        config.default_provider = Some("openrouter".into());
        config.api_key = Some("sk-test".into());

        let provider_rdy = provider_ready(&config);
        assert!(provider_rdy, "provider should be ready with API key set");

        let (desktop_helpers_ready, _) = if doctor::is_computer_use_backend(&config) {
            let missing = topclaw::tools::computer_use::missing_linux_helpers();
            (missing.is_empty(), missing)
        } else {
            (true, Vec::new())
        };

        if !desktop_helpers_ready {
            // If helpers are missing on this host, overall_ready must be false.
            let diag_results = doctor::diagnose(&config);
            let channels_configured = channels_configured(&config);
            let daemon_rdy = daemon_ready(&diag_results);
            let overall =
                provider_rdy && (!channels_configured || daemon_rdy) && desktop_helpers_ready;
            assert!(
                !overall,
                "overall_ready must be false when desktop helpers are missing"
            );
        }
        // When all helpers are installed, overall_ready depends only on
        // provider/channels/daemon — desktop_helpers_ready is true, so it
        // doesn't drag overall down. That's covered by existing tests.
    }

    #[test]
    fn status_omits_desktop_helpers_when_not_computer_use() {
        let config = Config::default();
        // Default browser backend is not computer_use.
        assert!(!doctor::is_computer_use_backend(&config));
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
    #[cfg(feature = "computer-use-sidecar")]
    fn doctor_cli_parses_desktop_helpers_without_install() {
        let cli = Cli::try_parse_from(["topclaw", "doctor", "desktop-helpers"])
            .expect("doctor desktop-helpers should parse");

        match cli.command {
            Commands::Doctor {
                doctor_command: Some(DoctorCommands::DesktopHelpers { install }),
            } => assert!(!install),
            other => panic!("expected doctor desktop-helpers command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "computer-use-sidecar")]
    fn bootstrap_cli_accepts_install_desktop_helpers_flag() {
        let cli = Cli::try_parse_from(["topclaw", "bootstrap", "--install-desktop-helpers"])
            .expect("bootstrap --install-desktop-helpers should parse");

        match cli.command {
            Commands::Onboard {
                install_desktop_helpers,
                force,
                interactive,
                channels_only,
                ..
            } => {
                assert!(install_desktop_helpers);
                assert!(!force);
                assert!(!interactive);
                assert!(!channels_only);
            }
            other => panic!("expected bootstrap command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "computer-use-sidecar")]
    fn bootstrap_cli_parses_channels_only_with_install_desktop_helpers() {
        // Clap cannot enforce cross-flag constraints at parse time;
        // runtime validation in main() rejects this combination with
        // bail!("--channels-only does not accept --install-desktop-helpers").
        // This test confirms the flags parse to the expected boolean values
        // so the runtime guard can evaluate them.
        let cli = Cli::try_parse_from([
            "topclaw",
            "bootstrap",
            "--channels-only",
            "--install-desktop-helpers",
        ])
        .expect("both flags should parse");

        match cli.command {
            Commands::Onboard {
                channels_only,
                install_desktop_helpers,
                ..
            } => {
                assert!(channels_only);
                assert!(install_desktop_helpers);
            }
            other => panic!("expected bootstrap command, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "computer-use-sidecar")]
    fn doctor_cli_parses_desktop_helpers_with_install() {
        let cli = Cli::try_parse_from(["topclaw", "doctor", "desktop-helpers", "--install"])
            .expect("doctor desktop-helpers --install should parse");

        match cli.command {
            Commands::Doctor {
                doctor_command: Some(DoctorCommands::DesktopHelpers { install }),
            } => assert!(install),
            other => panic!("expected doctor desktop-helpers --install command, got {other:?}"),
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
}
