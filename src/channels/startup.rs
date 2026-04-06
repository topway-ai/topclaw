//! Channel startup, health checks, and CLI command handling.

use crate::agent::loop_::{build_shell_policy_instructions, build_tool_instructions_from_specs};
use crate::agent::wiring;
use crate::approval::ApprovalManager;
use crate::config::Config;
use crate::memory;
use crate::providers::{self, Provider};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::context::*;
use super::dispatch::{run_message_dispatch_loop, spawn_supervised_listener};
use super::factory::collect_configured_channels;
use super::helpers::*;
use super::prompt::{
    build_channel_tool_descriptions, build_self_config_instructions, build_system_prompt_with_mode,
};
use super::runtime_config::{
    config_file_stamp, runtime_config_store, runtime_defaults_from_config, RuntimeConfigState,
};
use super::runtime_helpers::{
    filtered_tool_specs_for_runtime, resolved_default_model, resolved_default_provider,
};
use super::traits::Channel;

fn normalize_telegram_identity(value: &str) -> String {
    value.trim().trim_start_matches('@').to_string()
}

async fn bind_telegram_identity(config: &Config, identity: &str) -> Result<()> {
    let normalized = normalize_telegram_identity(identity);
    if normalized.is_empty() {
        anyhow::bail!("Telegram identity cannot be empty");
    }

    let mut updated = config.clone();
    let Some(telegram) = updated.channels_config.telegram.as_mut() else {
        anyhow::bail!(
            "Telegram channel is not configured. Run `topclaw bootstrap --channels-only` first"
        );
    };

    if telegram.allowed_users.iter().any(|u| u == "*") {
        println!(
            "\u{26A0}\u{FE0F} Telegram allowlist is currently wildcard (`*`) \u{2014} binding is unnecessary until you remove '*'."
        );
    }

    if telegram
        .allowed_users
        .iter()
        .map(|entry| normalize_telegram_identity(entry))
        .any(|entry| entry == normalized)
    {
        println!("\u{2705} Telegram identity already bound: {normalized}");
        return Ok(());
    }

    telegram.allowed_users.push(normalized.clone());
    updated.save().await?;
    println!("\u{2705} Bound Telegram identity: {normalized}");
    println!("   Saved to {}", updated.config_path.display());
    match maybe_restart_managed_daemon_service() {
        Ok(true) => {
            println!("\u{1F504} Detected running managed daemon service; reloaded automatically.");
        }
        Ok(false) => {
            println!(
                "\u{2139}\u{FE0F} No managed daemon service detected. If `topclaw daemon`/`channel start` is already running, restart it to load the updated allowlist."
            );
        }
        Err(e) => {
            eprintln!(
                "\u{26A0}\u{FE0F} Allowlist saved, but failed to reload daemon service automatically: {e}\n\
                 Restart service manually with `topclaw service stop && topclaw service start`."
            );
        }
    }
    Ok(())
}

fn maybe_restart_managed_daemon_service() -> Result<bool> {
    if cfg!(target_os = "macos") {
        let home = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("Could not find home directory")?;
        let plist = home
            .join("Library")
            .join("LaunchAgents")
            .join("com.topclaw.daemon.plist");
        if !plist.exists() {
            return Ok(false);
        }

        let list_output = Command::new("launchctl")
            .arg("list")
            .output()
            .context("Failed to query launchctl list")?;
        let listed = String::from_utf8_lossy(&list_output.stdout);
        if !listed.contains("com.topclaw.daemon") {
            return Ok(false);
        }

        let _ = Command::new("launchctl")
            .args(["stop", "com.topclaw.daemon"])
            .output();
        let start_output = Command::new("launchctl")
            .args(["start", "com.topclaw.daemon"])
            .output()
            .context("Failed to start launchd daemon service")?;
        if !start_output.status.success() {
            let stderr = String::from_utf8_lossy(&start_output.stderr);
            anyhow::bail!("launchctl start failed: {}", stderr.trim());
        }

        return Ok(true);
    }

    if cfg!(target_os = "linux") {
        // OpenRC (system-wide) takes precedence over systemd (user-level)
        let openrc_init_script = PathBuf::from("/etc/init.d/topclaw");
        if openrc_init_script.exists() {
            if let Ok(status_output) = Command::new("rc-service").args(OPENRC_STATUS_ARGS).output()
            {
                // rc-service exits 0 if running, non-zero otherwise
                if status_output.status.success() {
                    let restart_output = Command::new("rc-service")
                        .args(OPENRC_RESTART_ARGS)
                        .output()
                        .context("Failed to restart OpenRC daemon service")?;
                    if !restart_output.status.success() {
                        let stderr = String::from_utf8_lossy(&restart_output.stderr);
                        anyhow::bail!("rc-service restart failed: {}", stderr.trim());
                    }
                    return Ok(true);
                }
            }
        }

        // Systemd (user-level)
        let home = directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .context("Could not find home directory")?;
        let unit_path: PathBuf = home
            .join(".config")
            .join("systemd")
            .join("user")
            .join("topclaw.service");
        if !unit_path.exists() {
            return Ok(false);
        }

        let active_output = Command::new("systemctl")
            .args(SYSTEMD_STATUS_ARGS)
            .output()
            .context("Failed to query systemd service state")?;
        let state = String::from_utf8_lossy(&active_output.stdout);
        if !state.trim().eq_ignore_ascii_case("active") {
            return Ok(false);
        }

        let restart_output = Command::new("systemctl")
            .args(SYSTEMD_RESTART_ARGS)
            .output()
            .context("Failed to restart systemd daemon service")?;
        if !restart_output.status.success() {
            let stderr = String::from_utf8_lossy(&restart_output.stderr);
            anyhow::bail!("systemctl restart failed: {}", stderr.trim());
        }

        return Ok(true);
    }

    Ok(false)
}

// ============================================================================
// CLI Commands and Channel Startup
// ============================================================================

pub async fn handle_command(command: crate::ChannelCommands, config: &Config) -> Result<()> {
    match command {
        crate::ChannelCommands::Start => {
            anyhow::bail!("Start must be handled in main.rs (requires async runtime)")
        }
        crate::ChannelCommands::Doctor => {
            anyhow::bail!("Doctor must be handled in main.rs (requires async runtime)")
        }
        crate::ChannelCommands::List => {
            println!("Runtime channels:");
            println!("  \u{2705} CLI (always available)");
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
                    "  {} {} ({priority})",
                    if configured { "\u{2705}" } else { "\u{274C}" },
                    channel.name()
                );
            }
            let auxiliary = config.channels_config.auxiliary_channels();
            if auxiliary.iter().any(|(_, configured)| *configured) {
                println!();
                println!("Auxiliary surfaces:");
                for (channel, configured) in auxiliary {
                    println!(
                        "  {} {}",
                        if configured { "\u{2705}" } else { "\u{274C}" },
                        channel.name()
                    );
                }
                println!("  \u{2139}\u{FE0F} Auxiliary surfaces ride on the daemon/gateway path when configured.");
            }
            println!("\nTo start channels: topclaw channel start");
            println!("To check health:    topclaw channel doctor");
            println!("To configure:      topclaw bootstrap");
            Ok(())
        }
        crate::ChannelCommands::Add {
            channel_type,
            config: _,
        } => {
            anyhow::bail!(
                "Channel type '{channel_type}' \u{2014} use `topclaw bootstrap` to configure channels"
            );
        }
        crate::ChannelCommands::Remove { name } => {
            anyhow::bail!("Remove channel '{name}' \u{2014} edit ~/.topclaw/config.toml directly");
        }
        crate::ChannelCommands::BindTelegram { identity } => {
            bind_telegram_identity(config, &identity).await
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChannelHealthState {
    Healthy,
    Unhealthy,
    Timeout,
}

pub(super) fn classify_health_result(
    result: &std::result::Result<bool, tokio::time::error::Elapsed>,
) -> ChannelHealthState {
    match result {
        Ok(true) => ChannelHealthState::Healthy,
        Ok(false) => ChannelHealthState::Unhealthy,
        Err(_) => ChannelHealthState::Timeout,
    }
}

/// Run health checks for configured channels.
pub async fn doctor_channels(config: Config) -> Result<()> {
    let channels = collect_configured_channels(&config);
    let init_failures: Vec<String> = Vec::new();

    if channels.is_empty() && init_failures.is_empty() {
        println!("No real-time channels configured. Run `topclaw bootstrap` first.");
        return Ok(());
    }

    println!("\u{1FA7A} TopClaw Channel Doctor");
    println!();

    let mut healthy = 0_u32;
    let mut unhealthy = u32::try_from(init_failures.len()).unwrap_or(u32::MAX);
    let mut timeout = 0_u32;
    let has_runtime_channels = !channels.is_empty();

    for failure in &init_failures {
        println!("  \u{274C} {failure}");
    }

    for configured in channels {
        let result =
            tokio::time::timeout(Duration::from_secs(10), configured.channel.health_check()).await;
        let state = classify_health_result(&result);

        match state {
            ChannelHealthState::Healthy => {
                healthy += 1;
                println!("  \u{2705} {:<9} healthy", configured.display_name);
            }
            ChannelHealthState::Unhealthy => {
                unhealthy += 1;
                println!(
                    "  \u{274C} {:<9} unhealthy (auth/config/network)",
                    configured.display_name
                );
            }
            ChannelHealthState::Timeout => {
                timeout += 1;
                println!(
                    "  \u{23F1}\u{FE0F}  {:<9} timed out (>10s)",
                    configured.display_name
                );
            }
        }
    }

    if config.channels_config.webhook.is_some() {
        println!("  \u{2139}\u{FE0F}  Webhook   check via `topclaw gateway` then GET /health");
    }

    if !has_runtime_channels && !init_failures.is_empty() {
        println!();
        anyhow::bail!("All configured channels failed during initialization.");
    }

    println!();
    println!("Summary: {healthy} healthy, {unhealthy} unhealthy, {timeout} timed out");
    Ok(())
}

/// Start all configured channels and route messages to the agent
#[allow(clippy::too_many_lines)]
pub async fn start_channels(config: Config) -> Result<()> {
    let provider_name = resolved_default_provider(&config);
    let model = resolved_default_model(&config);
    let provider_runtime_options = providers::ProviderRuntimeOptions::from_config(&config);
    let provider: Arc<dyn Provider> = Arc::from(
        create_routed_provider_nonblocking(
            &provider_name,
            config.api_key.clone(),
            config.api_url.clone(),
            config.reliability.clone(),
            config.model_routes.clone(),
            model.clone(),
            provider_runtime_options.clone(),
        )
        .await?,
    );

    // Warm up the provider connection pool (TLS handshake, DNS, HTTP/2 setup)
    // so the first real message doesn't hit a cold-start timeout.
    if let Err(e) = provider.warmup().await {
        tracing::warn!("Provider warmup failed (non-fatal): {e}");
    }

    let initial_stamp = config_file_stamp(&config.config_path).await;
    {
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(
            config.config_path.clone(),
            RuntimeConfigState {
                defaults: runtime_defaults_from_config(&config),
                last_applied_stamp: initial_stamp,
            },
        );
    }

    let observer = wiring::build_observer(&config);
    let temperature = config.default_temperature;
    let wiring::ExecutionSupport {
        memory: mem, tools, ..
    } = wiring::build_channel_execution_support(&config, &[])?;
    // Build system prompt from workspace identity files + skills
    let workspace = config.workspace_dir.clone();
    let tools_registry = Arc::new(tools);
    let skills = wiring::load_skills(&config);

    // Collect tool descriptions for the prompt
    let mut tool_descs = build_channel_tool_descriptions(&config);
    let loaded_tool_names: HashSet<&str> = tools_registry.iter().map(|tool| tool.name()).collect();

    // Filter out tools excluded for non-CLI channels so the system prompt
    // does not advertise them for channel-driven runs.
    let excluded = &config.autonomy.non_cli_excluded_tools;
    tool_descs.retain(|(name, _)| {
        loaded_tool_names.contains(name) && !excluded.iter().any(|ex| ex.eq_ignore_ascii_case(name))
    });

    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    let native_tools = provider.supports_native_tools();
    let mut system_prompt = build_system_prompt_with_mode(
        &workspace,
        &model,
        &tool_descs,
        &skills,
        Some(&config.identity),
        bootstrap_max_chars,
        native_tools,
        config.skills.prompt_injection_mode,
    );
    if !native_tools {
        let filtered_specs = filtered_tool_specs_for_runtime(tools_registry.as_ref(), excluded);
        system_prompt.push_str(&build_tool_instructions_from_specs(&filtered_specs));
    }
    system_prompt.push_str(&build_shell_policy_instructions(&config.autonomy));
    system_prompt.push_str(&build_self_config_instructions(&config.config_path));

    if !skills.is_empty() {
        println!(
            "  \u{1F9E9} Skills:   {}",
            skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Collect active channels from a shared builder to keep startup and doctor parity.
    let configured_channels = collect_configured_channels(&config);
    let init_failures: Vec<String> = Vec::new();

    if configured_channels.is_empty() && init_failures.is_empty() {
        println!("No channels configured. Run `topclaw bootstrap` to set up channels.");
        return Ok(());
    }

    if configured_channels.is_empty() && !init_failures.is_empty() {
        for failure in &init_failures {
            println!("  \u{274C} {failure}");
        }
        anyhow::bail!("All configured channels failed during initialization.");
    }

    if !init_failures.is_empty() {
        for failure in &init_failures {
            println!("  \u{26A0}\u{FE0F}  {failure}");
        }
        println!();
    }

    let channels: Vec<Arc<dyn Channel>> = configured_channels
        .into_iter()
        .map(|configured| configured.channel)
        .collect();

    println!("\u{1F980} TopClaw Channel Server");
    println!("  \u{1F916} Model:    {model}");
    let effective_backend = memory::effective_memory_backend_name(
        &config.memory.backend,
        Some(&config.storage.provider.config),
    );
    println!(
        "  \u{1F9E0} Memory:   {} (auto-save: {})",
        effective_backend,
        if config.memory.auto_save { "on" } else { "off" }
    );
    println!(
        "  \u{1F4E1} Channels: {}",
        channels
            .iter()
            .map(|c| c.name())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();
    println!("  Listening for messages... (Ctrl+C to stop)");
    println!();

    crate::health::mark_component_ok("channels");

    let initial_backoff_secs = config
        .reliability
        .channel_initial_backoff_secs
        .max(DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS);
    let max_backoff_secs = config
        .reliability
        .channel_max_backoff_secs
        .max(DEFAULT_CHANNEL_MAX_BACKOFF_SECS);

    // Single message bus -- all channels send messages here
    let (tx, rx) = tokio::sync::mpsc::channel::<super::traits::ChannelMessage>(100);

    // Spawn a listener for each channel
    let mut handles = Vec::new();
    for ch in &channels {
        handles.push(spawn_supervised_listener(
            ch.clone(),
            tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        ));
    }
    drop(tx); // Drop our copy so rx closes when all channels stop

    let channels_by_name = Arc::new(
        channels
            .iter()
            .map(|ch| (ch.name().to_string(), Arc::clone(ch)))
            .collect::<HashMap<_, _>>(),
    );
    let max_in_flight_messages = compute_max_in_flight_messages(channels.len());

    println!("  \u{1F6A6} In-flight message limit: {max_in_flight_messages}");

    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert(provider_name.clone(), Arc::clone(&provider));
    let message_timeout_secs =
        effective_channel_message_timeout_secs(config.channels_config.message_timeout_secs);
    let interrupt_on_new_message = config
        .channels_config
        .telegram
        .as_ref()
        .is_some_and(|tg| tg.interrupt_on_new_message);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name,
        provider: Arc::clone(&provider),
        default_provider: Arc::new(provider_name),
        memory: Arc::clone(&mem),
        tools_registry: Arc::clone(&tools_registry),
        observer,
        system_prompt: Arc::new(system_prompt),
        model: Arc::new(model.clone()),
        temperature,
        auto_save_memory: config.memory.auto_save,
        max_tool_iterations: config.agent.max_tool_iterations,
        min_relevance_score: config.memory.min_relevance_score,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: config.api_key.clone(),
        api_url: config.api_url.clone(),
        reliability: Arc::new(config.reliability.clone()),
        provider_runtime_options,
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs,
        interrupt_on_new_message,
        multimodal: config.multimodal.clone(),
        hooks: if config.hooks.enabled {
            let mut runner = crate::hooks::HookRunner::new();
            if config.hooks.builtin.command_logger {
                runner.register(Box::new(crate::hooks::builtin::CommandLoggerHook::new()));
            }
            Some(Arc::new(runner))
        } else {
            None
        },
        non_cli_excluded_tools: Arc::new(Mutex::new(
            config.autonomy.non_cli_excluded_tools.clone(),
        )),
        query_classification: config.query_classification.clone(),
        model_routes: config.model_routes.clone(),
        approval_manager: Arc::new(ApprovalManager::from_config(&config.autonomy)),
    });

    run_message_dispatch_loop(rx, runtime_ctx, max_in_flight_messages).await;

    // Wait for all channel tasks
    for h in handles {
        let _ = h.await;
    }

    Ok(())
}
