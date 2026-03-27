use crate::config::schema::StreamMode;
use crate::config::{
    AutonomyConfig, BrowserConfig, ChannelsConfig, ComposioConfig, Config, DiscordConfig,
    HeartbeatConfig, HttpRequestConfig, MemoryConfig, ObservabilityConfig, RuntimeConfig,
    SecretsConfig, StorageConfig, TelegramConfig, WebFetchConfig, WebSearchConfig, WebhookConfig,
};
use crate::hardware::{self, HardwareConfig};
use crate::memory::{default_memory_backend_key, memory_backend_profile};
use crate::providers::{
    canonical_china_provider_name, is_glm_alias, is_glm_cn_alias, is_minimax_alias,
    is_moonshot_alias, is_qianfan_alias, is_qwen_alias, is_qwen_oauth_alias, is_zai_alias,
    is_zai_cn_alias, list_providers,
};
use anyhow::{bail, Context, Result};
use console::style;
#[cfg(test)]
use console::Key;
use dialoguer::{Confirm, Input, Select};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

#[path = "wizard_channel_flows.rs"]
mod channel_flows;
#[path = "wizard_channels.rs"]
mod channel_menu;
#[path = "wizard_model_catalog.rs"]
mod model_catalog;
#[path = "wizard_provider_setup.rs"]
mod provider_setup;
#[path = "wizard_skill_selection.rs"]
mod skill_selection;

use channel_flows::{setup_discord_channel, setup_telegram_channel, setup_webhook_channel};
use channel_menu::{
    channel_choice_is_configured, channel_menu_choices, channel_menu_option_labels,
    default_channel_menu_index, default_other_channel_menu_index, other_channel_menu_choices,
    other_channel_menu_option_labels, ChannelMenuChoice,
};
use model_catalog::{
    build_model_options, cache_live_models_for_provider, fetch_live_models_for_provider,
    fetch_openrouter_top_onboarding_models, humanize_age, interactive_model_labels,
    load_any_cached_models_for_provider, load_cached_models_for_provider,
    normalize_ollama_endpoint_url, ollama_uses_remote_endpoint, supports_live_model_fetch,
    MODEL_CACHE_TTL_SECS,
};
pub use model_catalog::{
    cached_model_catalog_stats, run_models_list, run_models_refresh, run_models_refresh_all,
    run_models_set, run_models_status,
};
#[cfg(test)]
#[allow(unused_imports)]
use model_catalog::{
    match_openrouter_rankings_to_model_ids, models_endpoint_for_provider, normalize_model_ids,
    now_unix_secs, parse_gemini_model_ids, parse_ollama_model_ids,
    parse_openai_compatible_model_ids, parse_openrouter_rankings_model_names,
    resolve_live_models_endpoint, save_model_cache_state, ModelCacheEntry, ModelCacheState,
    OpenRouterModelSummary,
};
use provider_setup::{
    advanced_provider_choices, prompt_advanced_provider_credentials,
    setup_advanced_custom_provider, setup_simple_custom_provider, setup_simple_named_provider,
};
#[cfg(test)]
use skill_selection::{
    apply_onboarding_skill_selection_key, default_selected_onboarding_skill,
    format_onboarding_skill_label, SkillOnboardingSelection,
};
use skill_selection::{apply_onboarding_skill_tool_defaults, setup_skills};

// ── SIMPLIFIED WIZARD: 4 Steps for Newbies ───────────────────────
//
// Step 1: Workspace - Where to store files
// Step 2: AI Provider - Choose model and configure provider authentication
// Step 3: Skills - Select the starter skills to enable or install
// Step 4: How to reach you - connect one or more channels
//
// Everything else (Tunnel, Web tools, Hardware, Memory) can be configured later.

/// User-provided personalization baked into workspace MD files.
#[derive(Debug, Clone, Default)]
pub struct ProjectContext {
    pub user_name: String,
    pub timezone: String,
    pub agent_name: String,
    pub communication_style: String,
}

// ── Banner ───────────────────────────────────────────────────────

const BANNER: &str = r"
    ⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡

    ████████╗ ██████╗ ██████╗  ██████╗██╗      █████╗ ██╗    ██╗
    ╚══██╔══╝██╔═══██╗██╔══██╗██╔════╝██║     ██╔══██╗██║    ██║
       ██║   ██║   ██║██████╔╝██║     ██║     ███████║██║ █╗ ██║
       ██║   ██║   ██║██╔═══╝ ██║     ██║     ██╔══██║██║███╗██║
       ██║   ╚██████╔╝██║     ╚██████╗███████╗██║  ██║╚███╔███╔╝
       ╚═╝    ╚═════╝ ╚═╝      ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝

    Zero overhead. Zero compromise. 100% Rust. 100% Agnostic.
    TopClaw — Your lean, mean AI assistant.

    ⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡⚡
";

const LIVE_MODEL_MAX_OPTIONS: usize = 120;
const CUSTOM_MODEL_SENTINEL: &str = "__custom_model__";
const OPENROUTER_ONBOARDING_MODEL_LIMIT: usize = 10;

fn has_launchable_channels(channels: &ChannelsConfig) -> bool {
    channels.channels_except_webhook().iter().any(|(_, ok)| *ok)
}

// ── Simplified 4-Step Wizard Entry Point ────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractiveOnboardingMode {
    FullOnboarding,
    UpdateProviderOnly,
    UpdateChannelsOnly,
}

pub async fn run_wizard(force: bool) -> Result<Config> {
    println!("{}", style(BANNER).cyan().bold());

    println!(
        "  {}",
        style("Welcome to TopClaw — the fastest, smallest AI assistant.")
            .white()
            .bold()
    );
    println!(
        "  {}",
        style("This wizard will configure your agent in under 2 minutes.").dim()
    );
    println!(
        "  {}",
        style("Tip: Press Enter to accept defaults. Everything can be changed later.").dim()
    );
    println!();

    // ── STEP 1: Workspace ──────────────────────────────────────────
    println!("  {}", style("[1/4] Where should we work?").cyan().bold());
    println!("  {}", style("─".repeat(40)).dim());
    let (workspace_dir, config_path) = setup_workspace().await?;

    match resolve_interactive_onboarding_mode(&config_path, force)? {
        InteractiveOnboardingMode::FullOnboarding => {}
        InteractiveOnboardingMode::UpdateProviderOnly => {
            return run_provider_update_wizard(&workspace_dir, &config_path).await;
        }
        InteractiveOnboardingMode::UpdateChannelsOnly => {
            return Box::pin(run_channels_repair_wizard()).await;
        }
    }

    // ── STEP 2: AI Provider & API Key ───────────────────────────────
    println!();
    println!("  {}", style("[2/4] Connect to an AI").cyan().bold());
    println!("  {}", style("─".repeat(40)).dim());
    let (provider, api_key, model, provider_api_url) = setup_provider_simple(
        &workspace_dir,
        &config_path,
        SecretsConfig::default().encrypt,
    )
    .await?;

    // ── STEP 3: Starter skills ────────────────────────────────────────
    println!();
    println!("  {}", style("[3/4] Choose starter skills").cyan().bold());
    println!("  {}", style("─".repeat(40)).dim());
    let skill_selection = setup_skills()?;

    // ── STEP 4: How to reach you (Channels) ──────────────────────────
    println!();
    println!(
        "  {}",
        style("[4/4] How do you want to talk to TopClaw?")
            .cyan()
            .bold()
    );
    println!("  {}", style("─".repeat(40)).dim());
    let channels_config = setup_channels()?;

    // ── Build config with sensible defaults for everything else ──────
    // Default: SQLite memory, supervised autonomy, native runtime
    let mut config = Config {
        workspace_dir: workspace_dir.clone(),
        config_path: config_path.clone(),
        api_key: if api_key.is_empty() {
            None
        } else {
            Some(api_key)
        },
        api_url: provider_api_url,
        default_provider: Some(provider),
        provider_api: None,
        default_model: Some(model),
        model_providers: std::collections::HashMap::new(),
        provider: crate::config::ProviderConfig::default(),
        default_temperature: 0.7,
        observability: ObservabilityConfig::default(),
        autonomy: AutonomyConfig::default(),
        security: crate::config::SecurityConfig::default(),
        runtime: RuntimeConfig::default(),
        research: crate::config::ResearchPhaseConfig::default(),
        reliability: crate::config::ReliabilityConfig::default(),
        scheduler: crate::config::SchedulerConfig::default(),
        coordination: crate::config::CoordinationConfig::default(),
        agent: crate::config::AgentConfig::default(),
        workspaces: crate::config::WorkspacesConfig::default(),
        skills: crate::config::SkillsConfig::default(),
        model_routes: Vec::new(),
        embedding_routes: Vec::new(),
        heartbeat: HeartbeatConfig::default(),
        cron: crate::config::CronConfig::default(),

        channels_config,
        memory: memory_config_defaults_for_backend("sqlite"),
        storage: StorageConfig::default(),
        tunnel: crate::config::TunnelConfig::default(),
        gateway: crate::config::GatewayConfig::default(),
        composio: ComposioConfig::default(),
        secrets: SecretsConfig::default(),
        browser: BrowserConfig::default(),
        http_request: HttpRequestConfig::default(),
        multimodal: crate::config::MultimodalConfig::default(),
        web_fetch: WebFetchConfig::default(),
        web_search: WebSearchConfig::default(),
        proxy: crate::config::ProxyConfig::default(),
        identity: crate::config::IdentityConfig::default(),
        cost: crate::config::CostConfig::default(),
        peripherals: crate::config::PeripheralsConfig::default(),
        agents: std::collections::HashMap::new(),
        hooks: crate::config::HooksConfig::default(),
        hardware: HardwareConfig::default(),
        query_classification: crate::config::QueryClassificationConfig::default(),
        transcription: crate::config::TranscriptionConfig::default(),
        agents_ipc: crate::config::AgentsIpcConfig::default(),
        model_support_vision: None,
    };
    apply_onboarding_skill_tool_defaults(&mut config, &skill_selection);

    println!();
    println!(
        "  {} Security: {} | workspace-scoped",
        style("✓").green().bold(),
        style("Supervised").green()
    );
    println!(
        "  {} Memory: {} (auto-save: on)",
        style("✓").green().bold(),
        style("SQLite").green()
    );
    println!(
        "  {} Skills: {} selected",
        style("✓").green().bold(),
        style(skill_selection.selected_curated_slugs.len()).green(),
    );

    config.save().await?;
    persist_workspace_selection(&config.config_path).await?;

    // Scaffold workspace files
    let project_ctx = ProjectContext {
        user_name: std::env::var("USER").unwrap_or_else(|_| "User".into()),
        timezone: "UTC".into(),
        agent_name: "TopClaw".into(),
        communication_style: "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing.".into(),
    };
    scaffold_workspace(&workspace_dir, &project_ctx).await?;
    crate::skills::sync_curated_skill_selection(
        &workspace_dir,
        &skill_selection.selected_curated_slugs,
    )?;

    let service_outcome = ensure_background_service_for_channels(&config)?;

    // ── Final summary ────────────────────────────────────────────
    print_summary(&config, &service_outcome);

    Ok(config)
}

/// Interactive repair flow: rerun channel setup only without redoing full onboarding.
pub async fn run_channels_repair_wizard() -> Result<Config> {
    println!("{}", style(BANNER).cyan().bold());
    println!(
        "  {}",
        style("Channels Repair — update channel tokens and allowlists only")
            .white()
            .bold()
    );
    println!();

    let mut config = Config::load_or_init().await?;

    print_step(1, 1, "Channels (How You Talk to TopClaw)");
    config.channels_config = setup_channels()?;
    config.save().await?;
    persist_workspace_selection(&config.config_path).await?;
    let service_outcome = ensure_background_service_for_channels(&config)?;

    println!();
    println!(
        "  {} Channel config saved: {}",
        style("✓").green().bold(),
        style(config.config_path.display()).green()
    );
    print_service_outcome(&service_outcome);

    Ok(config)
}

/// Interactive flow: update only provider/model/api key while preserving existing config.
async fn run_provider_update_wizard(workspace_dir: &Path, config_path: &Path) -> Result<Config> {
    println!();
    println!(
        "  {} Existing config detected. Running provider-only update mode (preserving channels, memory, tunnel, hooks, and other settings).",
        style("↻").cyan().bold()
    );

    let raw = fs::read_to_string(config_path).await.with_context(|| {
        format!(
            "Failed to read existing config at {}",
            config_path.display()
        )
    })?;
    let mut config: Config = toml::from_str(&raw).with_context(|| {
        format!(
            "Failed to parse existing config at {}",
            config_path.display()
        )
    })?;
    config.workspace_dir = workspace_dir.to_path_buf();
    config.config_path = config_path.to_path_buf();

    print_step(1, 1, "AI Provider & API Key");
    let (provider, api_key, model, provider_api_url) =
        setup_provider_simple(workspace_dir, config_path, config.secrets.encrypt).await?;
    apply_provider_update(&mut config, provider, api_key, model, provider_api_url);

    config.save().await?;
    persist_workspace_selection(&config.config_path).await?;
    let service_outcome = ensure_background_service_for_channels(&config)?;

    println!(
        "  {} Provider settings updated at {}",
        style("✓").green().bold(),
        style(config.config_path.display()).green()
    );
    print_summary(&config, &service_outcome);

    Ok(config)
}

async fn maybe_prompt_openai_codex_login(
    provider_name: &str,
    config_path: &Path,
    encrypt_secrets: bool,
) -> Result<()> {
    if canonical_provider_name(provider_name) != "openai-codex" {
        return Ok(());
    }

    let state_dir = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let auth_service = crate::auth::AuthService::new(&state_dir, encrypt_secrets);
    if auth_service
        .get_profile("openai-codex", None)
        .await?
        .is_some()
    {
        print_bullet("Existing OpenAI Codex OAuth profile detected. Skipping login.");
        return Ok(());
    }

    let start_login = Confirm::new()
        .with_prompt("  Start OpenAI Codex login now?")
        .default(true)
        .interact()?;

    if !start_login {
        print_bullet("Run `topclaw auth login --provider openai-codex` when you're ready.");
        return Ok(());
    }

    let client = reqwest::Client::new();
    let pkce = crate::auth::openai_oauth::generate_pkce_state();
    let authorize_url = crate::auth::openai_oauth::build_authorize_url(&pkce);

    println!(
        "  {} OpenAI browser login ready.",
        style("✓").green().bold()
    );
    print_bullet("Open this URL in your browser and authorize access:");
    println!("    {}", style(&authorize_url).cyan());
    print_bullet("Waiting for callback at http://localhost:1455/auth/callback ...");

    let code = match crate::auth::openai_oauth::receive_loopback_code(
        &pkce.state,
        Duration::from_secs(180),
    )
    .await
    {
        Ok(code) => code,
        Err(error) => {
            print_bullet(&format!(
                "Browser callback capture failed ({}).",
                style(error.to_string()).yellow()
            ));

            let redirect_input: String = Input::new()
                .with_prompt("  Paste redirect URL or OAuth code (leave empty to skip for now)")
                .allow_empty(true)
                .interact_text()?;

            let trimmed = redirect_input.trim();
            if trimmed.is_empty() {
                print_bullet(
                    "Skipping OAuth for now. Run `topclaw auth login --provider openai-codex` later.",
                );
                return Ok(());
            }

            crate::auth::openai_oauth::parse_code_from_redirect(trimmed, Some(&pkce.state))?
        }
    };

    let token_set =
        crate::auth::openai_oauth::exchange_code_for_tokens(&client, &code, &pkce).await?;
    let account_id =
        crate::auth::openai_oauth::extract_account_id_from_jwt(&token_set.access_token);

    auth_service
        .store_openai_tokens("default", token_set, account_id, true)
        .await?;

    println!(
        "  {} OpenAI Codex login saved to profile {}",
        style("✓").green().bold(),
        style("default").green()
    );

    Ok(())
}

fn apply_provider_update(
    config: &mut Config,
    provider: String,
    api_key: String,
    model: String,
    provider_api_url: Option<String>,
) {
    config.default_provider = Some(provider);
    config.default_model = Some(model);
    config.api_url = provider_api_url;
    config.api_key = if api_key.trim().is_empty() {
        None
    } else {
        Some(api_key)
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackgroundServiceOutcome {
    NotNeeded,
    Started,
    ManualRequired(String),
}

fn ensure_background_service_for_channels(config: &Config) -> Result<BackgroundServiceOutcome> {
    if !has_launchable_channels(&config.channels_config) {
        return Ok(BackgroundServiceOutcome::NotNeeded);
    }

    if cfg!(target_os = "linux") {
        match crate::service::InitSystem::Auto.resolve() {
            Ok(crate::service::InitSystem::Openrc) => {
                return Ok(BackgroundServiceOutcome::ManualRequired(
                    "OpenRC requires manual service setup. Run `sudo topclaw service install` once, then `sudo topclaw service start`.".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) => {
                return Ok(BackgroundServiceOutcome::ManualRequired(format!(
                    "Could not auto-detect a supported background service manager ({}). Run `topclaw daemon` manually or use `topclaw service install` / `topclaw service start` if supported.",
                    error
                )));
            }
        }
    }

    let install = crate::ServiceCommands::Install;
    if let Err(error) =
        crate::service::handle_command(&install, config, crate::service::InitSystem::Auto)
    {
        return Ok(BackgroundServiceOutcome::ManualRequired(format!(
            "Automatic service installation failed ({}). Run `topclaw service install` and `topclaw service start` manually.",
            error
        )));
    }

    let restart = crate::ServiceCommands::Restart;
    if let Err(error) =
        crate::service::handle_command(&restart, config, crate::service::InitSystem::Auto)
    {
        return Ok(BackgroundServiceOutcome::ManualRequired(format!(
            "Service was installed, but automatic start failed ({}). Run `topclaw service start` manually.",
            error
        )));
    }

    Ok(BackgroundServiceOutcome::Started)
}

// ── Quick setup (zero prompts) ───────────────────────────────────

/// Non-interactive setup: generates a sensible default config instantly.
/// Use `topclaw bootstrap` or `topclaw bootstrap --api-key sk-... --provider openrouter --memory sqlite|lucid`.
/// Use `topclaw bootstrap --interactive` for the full wizard.
fn memory_config_defaults_for_backend(backend: &str) -> MemoryConfig {
    let profile = memory_backend_profile(backend);

    MemoryConfig {
        backend: backend.to_string(),
        auto_save: profile.auto_save_default,
        hygiene_enabled: profile.uses_sqlite_hygiene,
        archive_after_days: if profile.uses_sqlite_hygiene { 7 } else { 0 },
        purge_after_days: if profile.uses_sqlite_hygiene { 30 } else { 0 },
        conversation_retention_days: 30,
        embedding_provider: "none".to_string(),
        embedding_model: "text-embedding-3-small".to_string(),
        embedding_dimensions: 1536,
        vector_weight: 0.7,
        keyword_weight: 0.3,
        min_relevance_score: 0.4,
        embedding_cache_size: if profile.uses_sqlite_hygiene {
            10000
        } else {
            0
        },
        chunk_max_tokens: 512,
        response_cache_enabled: false,
        response_cache_ttl_minutes: 60,
        response_cache_max_entries: 5_000,
        snapshot_enabled: false,
        snapshot_on_hygiene: false,
        auto_hydrate: true,
        sqlite_open_timeout_secs: None,
        qdrant: crate::config::QdrantConfig::default(),
    }
}

#[allow(clippy::too_many_lines)]
pub async fn run_quick_setup(
    credential_override: Option<&str>,
    provider: Option<&str>,
    model_override: Option<&str>,
    memory_backend: Option<&str>,
    force: bool,
) -> Result<Config> {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;

    Box::pin(run_quick_setup_with_home(
        credential_override,
        provider,
        model_override,
        memory_backend,
        force,
        &home,
    ))
    .await
}

fn resolve_quick_setup_dirs_with_home(home: &Path) -> (PathBuf, PathBuf) {
    if let Ok(custom_config_dir) = std::env::var("TOPCLAW_CONFIG_DIR") {
        let trimmed = custom_config_dir.trim();
        if !trimmed.is_empty() {
            let config_dir = PathBuf::from(trimmed);
            return (config_dir.clone(), config_dir.join("workspace"));
        }
    }

    if let Ok(custom_workspace) = std::env::var("TOPCLAW_WORKSPACE") {
        let trimmed = custom_workspace.trim();
        if !trimmed.is_empty() {
            return crate::config::schema::resolve_config_dir_for_workspace(&PathBuf::from(
                trimmed,
            ));
        }
    }

    let config_dir = home.join(".topclaw");
    (config_dir.clone(), config_dir.join("workspace"))
}

#[allow(clippy::too_many_lines)]
async fn run_quick_setup_with_home(
    credential_override: Option<&str>,
    provider: Option<&str>,
    model_override: Option<&str>,
    memory_backend: Option<&str>,
    force: bool,
    home: &Path,
) -> Result<Config> {
    let (topclaw_dir, workspace_dir) = resolve_quick_setup_dirs_with_home(home);
    let config_path = topclaw_dir.join("config.toml");

    let has_existing_config = config_path.exists();
    let has_quick_overrides = credential_override.is_some()
        || provider.is_some()
        || model_override.is_some()
        || memory_backend.is_some();
    let interactive_terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    if should_redirect_existing_config_to_interactive_menu(QuickSetupRedirectContext {
        has_existing_config,
        has_quick_overrides,
        force,
        interactive_terminal,
    }) {
        return Box::pin(run_wizard(false)).await;
    }

    println!("{}", style(BANNER).cyan().bold());
    println!(
        "  {}",
        style("Quick Setup — generating config with sensible defaults...")
            .white()
            .bold()
    );
    println!();

    ensure_onboard_overwrite_allowed(&config_path, force)?;
    fs::create_dir_all(&workspace_dir)
        .await
        .context("Failed to create workspace directory")?;

    let provider_name = provider.unwrap_or("openrouter").to_string();
    let model = model_override
        .map(str::to_string)
        .unwrap_or_else(|| default_model_for_provider(&provider_name));
    let memory_backend_name = memory_backend
        .unwrap_or(default_memory_backend_key())
        .to_string();

    // Create memory config based on backend choice
    let memory_config = memory_config_defaults_for_backend(&memory_backend_name);

    let config = Config {
        workspace_dir: workspace_dir.clone(),
        config_path: config_path.clone(),
        api_key: credential_override.map(|c| {
            let mut s = String::with_capacity(c.len());
            s.push_str(c);
            s
        }),
        api_url: None,
        default_provider: Some(provider_name.clone()),
        provider_api: None,
        default_model: Some(model.clone()),
        model_providers: std::collections::HashMap::new(),
        provider: crate::config::ProviderConfig::default(),
        default_temperature: 0.7,
        observability: ObservabilityConfig::default(),
        autonomy: AutonomyConfig::default(),
        security: crate::config::SecurityConfig::default(),
        runtime: RuntimeConfig::default(),
        research: crate::config::ResearchPhaseConfig::default(),
        reliability: crate::config::ReliabilityConfig::default(),
        scheduler: crate::config::SchedulerConfig::default(),
        coordination: crate::config::CoordinationConfig::default(),
        agent: crate::config::AgentConfig::default(),
        workspaces: crate::config::WorkspacesConfig::default(),
        skills: crate::config::SkillsConfig::default(),
        model_routes: Vec::new(),
        embedding_routes: Vec::new(),
        heartbeat: HeartbeatConfig::default(),
        cron: crate::config::CronConfig::default(),

        channels_config: ChannelsConfig::default(),
        memory: memory_config,
        storage: StorageConfig::default(),
        tunnel: crate::config::TunnelConfig::default(),
        gateway: crate::config::GatewayConfig::default(),
        composio: ComposioConfig::default(),
        secrets: SecretsConfig::default(),
        browser: BrowserConfig::default(),
        http_request: crate::config::HttpRequestConfig::default(),
        multimodal: crate::config::MultimodalConfig::default(),
        web_fetch: crate::config::WebFetchConfig::default(),
        web_search: crate::config::WebSearchConfig::default(),
        proxy: crate::config::ProxyConfig::default(),
        identity: crate::config::IdentityConfig::default(),
        cost: crate::config::CostConfig::default(),
        peripherals: crate::config::PeripheralsConfig::default(),
        agents: std::collections::HashMap::new(),
        hooks: crate::config::HooksConfig::default(),
        hardware: crate::config::HardwareConfig::default(),
        query_classification: crate::config::QueryClassificationConfig::default(),
        transcription: crate::config::TranscriptionConfig::default(),
        agents_ipc: crate::config::AgentsIpcConfig::default(),
        model_support_vision: None,
    };

    config.save().await?;
    persist_workspace_selection(&config.config_path).await?;

    // Scaffold minimal workspace files
    let default_ctx = ProjectContext {
        user_name: std::env::var("USER").unwrap_or_else(|_| "User".into()),
        timezone: "UTC".into(),
        agent_name: "TopClaw".into(),
        communication_style:
            "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing."
                .into(),
    };
    scaffold_workspace(&workspace_dir, &default_ctx).await?;

    println!(
        "  {} Workspace:  {}",
        style("✓").green().bold(),
        style(workspace_dir.display()).green()
    );
    println!(
        "  {} Provider:   {}",
        style("✓").green().bold(),
        style(&provider_name).green()
    );
    println!(
        "  {} Model:      {}",
        style("✓").green().bold(),
        style(&model).green()
    );
    println!(
        "  {} API Key:    {}",
        style("✓").green().bold(),
        if credential_override.is_some() {
            style("set").green()
        } else {
            style("not set (use --api-key or edit config.toml)").yellow()
        }
    );
    println!(
        "  {} Security:   {}",
        style("✓").green().bold(),
        style("Supervised (workspace-scoped)").green()
    );
    println!(
        "  {} Memory:     {} (auto-save: {})",
        style("✓").green().bold(),
        style(&memory_backend_name).green(),
        if memory_backend_name == "none" {
            "off"
        } else {
            "on"
        }
    );
    println!(
        "  {} Secrets:    {}",
        style("✓").green().bold(),
        style("encrypted").green()
    );
    println!(
        "  {} Gateway:    {}",
        style("✓").green().bold(),
        style("pairing required (127.0.0.1:8080)").green()
    );
    println!(
        "  {} Tunnel:     {}",
        style("✓").green().bold(),
        style("none (local only)").dim()
    );
    println!(
        "  {} Composio:   {}",
        style("✓").green().bold(),
        style("disabled (sovereign mode)").dim()
    );
    println!();
    println!(
        "  {} {}",
        style("Config saved:").white().bold(),
        style(config_path.display()).green()
    );
    println!();
    println!("  {}", style("Next steps:").white().bold());
    if credential_override.is_none() {
        if provider_supports_keyless_local_usage(&provider_name) {
            println!("    1. Chat:     topclaw agent -m \"Hello!\"");
            println!("    2. Gateway:  topclaw gateway");
            println!("    3. Status:   topclaw status");
        } else if provider_supports_device_flow(&provider_name) {
            if canonical_provider_name(&provider_name) == "copilot" {
                println!("    1. Chat:              topclaw agent -m \"Hello!\"");
                println!("       (device / OAuth auth will prompt on first run)");
                println!("    2. Gateway:           topclaw gateway");
                println!("    3. Status:            topclaw status");
            } else {
                println!(
                    "    1. Login:             topclaw auth login --provider {}",
                    provider_name
                );
                println!("    2. Chat:              topclaw agent -m \"Hello!\"");
                println!("    3. Gateway:           topclaw gateway");
                println!("    4. Status:            topclaw status");
            }
        } else {
            let env_var = provider_env_var(&provider_name);
            println!("    1. Set your API key:  export {env_var}=\"sk-...\"");
            println!("    2. Or edit:           ~/.topclaw/config.toml");
            println!("    3. Chat:              topclaw agent -m \"Hello!\"");
            println!("    4. Gateway:           topclaw gateway");
        }
    } else {
        println!("    1. Chat:     topclaw agent -m \"Hello!\"");
        println!("    2. Gateway:  topclaw gateway");
        println!("    3. Status:   topclaw status");
    }
    println!();

    Ok(config)
}

fn should_redirect_existing_config_to_interactive_menu(context: QuickSetupRedirectContext) -> bool {
    context.has_existing_config
        && !context.has_quick_overrides
        && !context.force
        && context.interactive_terminal
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Copy, Clone)]
struct QuickSetupRedirectContext {
    has_existing_config: bool,
    has_quick_overrides: bool,
    force: bool,
    interactive_terminal: bool,
}

fn canonical_provider_name(provider_name: &str) -> &str {
    if is_qwen_oauth_alias(provider_name) {
        return "qwen-code";
    }

    if let Some(canonical) = canonical_china_provider_name(provider_name) {
        return canonical;
    }

    match provider_name {
        "grok" => "xai",
        "together" => "together-ai",
        "google" | "google-gemini" => "gemini",
        "github-copilot" => "copilot",
        "openai_codex" | "codex" => "openai-codex",
        "kimi_coding" | "kimi_for_coding" => "kimi-code",
        "nvidia-nim" | "build.nvidia.com" => "nvidia",
        "aws-bedrock" => "bedrock",
        "llama.cpp" => "llamacpp",
        _ => provider_name,
    }
}

fn allows_unauthenticated_model_fetch(provider_name: &str) -> bool {
    matches!(
        canonical_provider_name(provider_name),
        "openrouter"
            | "ollama"
            | "llamacpp"
            | "sglang"
            | "vllm"
            | "osaurus"
            | "venice"
            | "astrai"
            | "nvidia"
    )
}

/// Pick a sensible default model for the given provider.
fn default_model_for_provider(provider: &str) -> String {
    match canonical_provider_name(provider) {
        "anthropic" => "claude-sonnet-4-5-20250929".into(),
        "openai" => "gpt-5.2".into(),
        "openai-codex" => "gpt-5.4".into(),
        "venice" => "zai-org-glm-5".into(),
        "groq" => "llama-3.3-70b-versatile".into(),
        "mistral" => "mistral-large-latest".into(),
        "deepseek" => "deepseek-chat".into(),
        "xai" => "grok-4-1-fast-reasoning".into(),
        "perplexity" => "sonar-pro".into(),
        "fireworks" => "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
        "novita" => "minimax/minimax-m2.5".into(),
        "together-ai" => "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
        "cohere" => "command-a-03-2025".into(),
        "moonshot" => "kimi-k2.5".into(),
        "hunyuan" => "hunyuan-t1-latest".into(),
        "glm" | "zai" => "glm-5".into(),
        "minimax" => "MiniMax-M2.5".into(),
        "qwen" => "qwen-plus".into(),
        "qwen-code" => "qwen3-coder-plus".into(),
        "ollama" => "llama3.2".into(),
        "llamacpp" => "ggml-org/gpt-oss-20b-GGUF".into(),
        "sglang" | "vllm" | "osaurus" => "default".into(),
        "gemini" => "gemini-2.5-pro".into(),
        "kimi-code" => "kimi-for-coding".into(),
        "bedrock" => "anthropic.claude-sonnet-4-5-20250929-v1:0".into(),
        "nvidia" => "meta/llama-3.3-70b-instruct".into(),
        _ => "anthropic/claude-sonnet-4.6".into(),
    }
}

fn curated_models_for_provider(provider_name: &str) -> Vec<(String, String)> {
    match canonical_provider_name(provider_name) {
        "openrouter" => vec![
            (
                "anthropic/claude-sonnet-4.6".to_string(),
                "Claude Sonnet 4.6 (balanced, recommended)".to_string(),
            ),
            (
                "openai/gpt-5.2".to_string(),
                "GPT-5.2 (latest flagship)".to_string(),
            ),
            (
                "openai/gpt-5-mini".to_string(),
                "GPT-5 mini (fast, cost-efficient)".to_string(),
            ),
            (
                "google/gemini-3-pro-preview".to_string(),
                "Gemini 3 Pro Preview (frontier reasoning)".to_string(),
            ),
            (
                "x-ai/grok-4.1-fast".to_string(),
                "Grok 4.1 Fast (reasoning + speed)".to_string(),
            ),
            (
                "deepseek/deepseek-v3.2".to_string(),
                "DeepSeek V3.2 (agentic + affordable)".to_string(),
            ),
            (
                "meta-llama/llama-4-maverick".to_string(),
                "Llama 4 Maverick (open model)".to_string(),
            ),
        ],
        "anthropic" => vec![
            (
                "claude-sonnet-4-5-20250929".to_string(),
                "Claude Sonnet 4.5 (balanced, recommended)".to_string(),
            ),
            (
                "claude-opus-4-6".to_string(),
                "Claude Opus 4.6 (best quality)".to_string(),
            ),
            (
                "claude-haiku-4-5-20251001".to_string(),
                "Claude Haiku 4.5 (fastest, cheapest)".to_string(),
            ),
        ],
        "openai" => vec![
            (
                "gpt-5.2".to_string(),
                "GPT-5.2 (latest coding/agentic flagship)".to_string(),
            ),
            (
                "gpt-5-mini".to_string(),
                "GPT-5 mini (faster, cheaper)".to_string(),
            ),
            (
                "gpt-5-nano".to_string(),
                "GPT-5 nano (lowest latency/cost)".to_string(),
            ),
            (
                "gpt-5.2-codex".to_string(),
                "GPT-5.2 Codex (agentic coding)".to_string(),
            ),
        ],
        "openai-codex" => vec![
            ("gpt-5.4".to_string(), "GPT-5.4 (recommended)".to_string()),
            (
                "gpt-5-codex".to_string(),
                "GPT-5 Codex (agentic coding)".to_string(),
            ),
            (
                "gpt-5.2-codex".to_string(),
                "GPT-5.2 Codex (agentic coding)".to_string(),
            ),
            ("o4-mini".to_string(), "o4-mini (fallback)".to_string()),
        ],
        "venice" => vec![
            (
                "zai-org-glm-5".to_string(),
                "GLM-5 via Venice (agentic flagship)".to_string(),
            ),
            (
                "claude-sonnet-4-6".to_string(),
                "Claude Sonnet 4.6 via Venice (best quality)".to_string(),
            ),
            (
                "deepseek-v3.2".to_string(),
                "DeepSeek V3.2 via Venice (strong value)".to_string(),
            ),
            (
                "grok-41-fast".to_string(),
                "Grok 4.1 Fast via Venice (low latency)".to_string(),
            ),
        ],
        "groq" => vec![
            (
                "llama-3.3-70b-versatile".to_string(),
                "Llama 3.3 70B (fast, recommended)".to_string(),
            ),
            (
                "openai/gpt-oss-120b".to_string(),
                "GPT-OSS 120B (strong open-weight)".to_string(),
            ),
            (
                "openai/gpt-oss-20b".to_string(),
                "GPT-OSS 20B (cost-efficient open-weight)".to_string(),
            ),
        ],
        "mistral" => vec![
            (
                "mistral-large-latest".to_string(),
                "Mistral Large (latest flagship)".to_string(),
            ),
            (
                "mistral-medium-latest".to_string(),
                "Mistral Medium (balanced)".to_string(),
            ),
            (
                "codestral-latest".to_string(),
                "Codestral (code-focused)".to_string(),
            ),
            (
                "devstral-latest".to_string(),
                "Devstral (software engineering specialist)".to_string(),
            ),
        ],
        "deepseek" => vec![
            (
                "deepseek-chat".to_string(),
                "DeepSeek Chat (mapped to V3.2 non-thinking)".to_string(),
            ),
            (
                "deepseek-reasoner".to_string(),
                "DeepSeek Reasoner (mapped to V3.2 thinking)".to_string(),
            ),
        ],
        "hunyuan" => vec![
            (
                "hunyuan-t1-latest".to_string(),
                "Hunyuan T1 (deep reasoning, latest)".to_string(),
            ),
            (
                "hunyuan-turbo-latest".to_string(),
                "Hunyuan Turbo (fast, general purpose)".to_string(),
            ),
            (
                "hunyuan-pro".to_string(),
                "Hunyuan Pro (high quality)".to_string(),
            ),
        ],
        "xai" => vec![
            (
                "grok-4-1-fast-reasoning".to_string(),
                "Grok 4.1 Fast Reasoning (recommended)".to_string(),
            ),
            (
                "grok-4-1-fast-non-reasoning".to_string(),
                "Grok 4.1 Fast Non-Reasoning (low latency)".to_string(),
            ),
            (
                "grok-code-fast-1".to_string(),
                "Grok Code Fast 1 (coding specialist)".to_string(),
            ),
            ("grok-4".to_string(), "Grok 4 (max quality)".to_string()),
        ],
        "perplexity" => vec![
            (
                "sonar-pro".to_string(),
                "Sonar Pro (flagship web-grounded model)".to_string(),
            ),
            (
                "sonar-reasoning-pro".to_string(),
                "Sonar Reasoning Pro (complex multi-step reasoning)".to_string(),
            ),
            (
                "sonar-deep-research".to_string(),
                "Sonar Deep Research (long-form research)".to_string(),
            ),
            ("sonar".to_string(), "Sonar (search, fast)".to_string()),
        ],
        "fireworks" => vec![
            (
                "accounts/fireworks/models/llama-v3p3-70b-instruct".to_string(),
                "Llama 3.3 70B".to_string(),
            ),
            (
                "accounts/fireworks/models/mixtral-8x22b-instruct".to_string(),
                "Mixtral 8x22B".to_string(),
            ),
        ],
        "novita" => vec![(
            "minimax/minimax-m2.5".to_string(),
            "MiniMax M2.5".to_string(),
        )],
        "together-ai" => vec![
            (
                "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string(),
                "Llama 3.3 70B Instruct Turbo (recommended)".to_string(),
            ),
            (
                "moonshotai/Kimi-K2.5".to_string(),
                "Kimi K2.5 (reasoning + coding)".to_string(),
            ),
            (
                "deepseek-ai/DeepSeek-V3.1".to_string(),
                "DeepSeek V3.1 (strong value)".to_string(),
            ),
        ],
        "cohere" => vec![
            (
                "command-a-03-2025".to_string(),
                "Command A (flagship enterprise model)".to_string(),
            ),
            (
                "command-a-reasoning-08-2025".to_string(),
                "Command A Reasoning (agentic reasoning)".to_string(),
            ),
            (
                "command-r-08-2024".to_string(),
                "Command R (stable fast baseline)".to_string(),
            ),
        ],
        "kimi-code" => vec![
            (
                "kimi-for-coding".to_string(),
                "Kimi for Coding (official coding-agent model)".to_string(),
            ),
            (
                "kimi-k2.5".to_string(),
                "Kimi K2.5 (general coding endpoint model)".to_string(),
            ),
        ],
        "moonshot" => vec![
            (
                "kimi-k2.5".to_string(),
                "Kimi K2.5 (latest flagship, recommended)".to_string(),
            ),
            (
                "kimi-k2-thinking".to_string(),
                "Kimi K2 Thinking (deep reasoning + tool use)".to_string(),
            ),
            (
                "kimi-k2-0905-preview".to_string(),
                "Kimi K2 0905 Preview (strong coding)".to_string(),
            ),
        ],
        "glm" | "zai" => vec![
            ("glm-5".to_string(), "GLM-5 (high reasoning)".to_string()),
            (
                "glm-4.7".to_string(),
                "GLM-4.7 (strong general-purpose quality)".to_string(),
            ),
            (
                "glm-4.5-air".to_string(),
                "GLM-4.5 Air (lower latency)".to_string(),
            ),
        ],
        "minimax" => vec![
            (
                "MiniMax-M2.5".to_string(),
                "MiniMax M2.5 (latest flagship)".to_string(),
            ),
            (
                "MiniMax-M2.5-highspeed".to_string(),
                "MiniMax M2.5 High-Speed (fast)".to_string(),
            ),
            (
                "MiniMax-M2.1".to_string(),
                "MiniMax M2.1 (strong coding/reasoning)".to_string(),
            ),
        ],
        "qwen" => vec![
            (
                "qwen-max".to_string(),
                "Qwen Max (highest quality)".to_string(),
            ),
            (
                "qwen-plus".to_string(),
                "Qwen Plus (balanced default)".to_string(),
            ),
            (
                "qwen-turbo".to_string(),
                "Qwen Turbo (fast and cost-efficient)".to_string(),
            ),
        ],
        "qwen-code" => vec![
            (
                "qwen3-coder-plus".to_string(),
                "Qwen3 Coder Plus (recommended for coding workflows)".to_string(),
            ),
            (
                "qwen3.5-plus".to_string(),
                "Qwen3.5 Plus (reasoning + coding)".to_string(),
            ),
            (
                "qwen3-max-2026-01-23".to_string(),
                "Qwen3 Max (high-capability coding model)".to_string(),
            ),
        ],
        "nvidia" => vec![
            (
                "meta/llama-3.3-70b-instruct".to_string(),
                "Llama 3.3 70B Instruct (balanced default)".to_string(),
            ),
            (
                "deepseek-ai/deepseek-v3.2".to_string(),
                "DeepSeek V3.2 (advanced reasoning + coding)".to_string(),
            ),
            (
                "nvidia/llama-3.3-nemotron-super-49b-v1.5".to_string(),
                "Llama 3.3 Nemotron Super 49B v1.5 (NVIDIA-tuned)".to_string(),
            ),
            (
                "nvidia/llama-3.1-nemotron-ultra-253b-v1".to_string(),
                "Llama 3.1 Nemotron Ultra 253B v1 (max quality)".to_string(),
            ),
        ],
        "astrai" => vec![
            (
                "anthropic/claude-sonnet-4.6".to_string(),
                "Claude Sonnet 4.6 (balanced default)".to_string(),
            ),
            (
                "openai/gpt-5.2".to_string(),
                "GPT-5.2 (latest flagship)".to_string(),
            ),
            (
                "deepseek/deepseek-v3.2".to_string(),
                "DeepSeek V3.2 (agentic + affordable)".to_string(),
            ),
            (
                "z-ai/glm-5".to_string(),
                "GLM-5 (high reasoning)".to_string(),
            ),
        ],
        "ollama" => vec![
            (
                "llama3.2".to_string(),
                "Llama 3.2 (recommended local)".to_string(),
            ),
            ("mistral".to_string(), "Mistral 7B".to_string()),
            ("codellama".to_string(), "Code Llama".to_string()),
            ("phi3".to_string(), "Phi-3 (small, fast)".to_string()),
        ],
        "llamacpp" => vec![
            (
                "ggml-org/gpt-oss-20b-GGUF".to_string(),
                "GPT-OSS 20B GGUF (llama.cpp server example)".to_string(),
            ),
            (
                "bartowski/Llama-3.3-70B-Instruct-GGUF".to_string(),
                "Llama 3.3 70B GGUF (high quality)".to_string(),
            ),
            (
                "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF".to_string(),
                "Qwen2.5 Coder 7B GGUF (coding-focused)".to_string(),
            ),
        ],
        "sglang" | "vllm" => vec![
            (
                "meta-llama/Llama-3.1-8B-Instruct".to_string(),
                "Llama 3.1 8B Instruct (popular, fast)".to_string(),
            ),
            (
                "meta-llama/Llama-3.1-70B-Instruct".to_string(),
                "Llama 3.1 70B Instruct (high quality)".to_string(),
            ),
            (
                "Qwen/Qwen2.5-Coder-7B-Instruct".to_string(),
                "Qwen2.5 Coder 7B Instruct (coding-focused)".to_string(),
            ),
        ],
        "osaurus" => vec![
            (
                "qwen3-30b-a3b-8bit".to_string(),
                "Qwen3 30B A3B (local, balanced)".to_string(),
            ),
            (
                "gemma-3n-e4b-it-lm-4bit".to_string(),
                "Gemma 3N E4B (local, efficient)".to_string(),
            ),
            (
                "phi-4-mini-reasoning-mlx-4bit".to_string(),
                "Phi-4 Mini Reasoning (local, fast reasoning)".to_string(),
            ),
        ],
        "bedrock" => vec![
            (
                "anthropic.claude-sonnet-4-6".to_string(),
                "Claude Sonnet 4.6 (latest, recommended)".to_string(),
            ),
            (
                "anthropic.claude-opus-4-6-v1".to_string(),
                "Claude Opus 4.6 (strongest)".to_string(),
            ),
            (
                "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
                "Claude Haiku 4.5 (fastest, cheapest)".to_string(),
            ),
            (
                "anthropic.claude-sonnet-4-5-20250929-v1:0".to_string(),
                "Claude Sonnet 4.5".to_string(),
            ),
        ],
        "gemini" => vec![
            (
                "gemini-3-pro-preview".to_string(),
                "Gemini 3 Pro Preview (latest frontier reasoning)".to_string(),
            ),
            (
                "gemini-2.5-pro".to_string(),
                "Gemini 2.5 Pro (stable reasoning)".to_string(),
            ),
            (
                "gemini-2.5-flash".to_string(),
                "Gemini 2.5 Flash (best price/performance)".to_string(),
            ),
            (
                "gemini-2.5-flash-lite".to_string(),
                "Gemini 2.5 Flash-Lite (lowest cost)".to_string(),
            ),
        ],
        _ => vec![("default".to_string(), "Default model".to_string())],
    }
}

// ── Step helpers ─────────────────────────────────────────────────

fn print_step(current: u8, total: u8, title: &str) {
    println!();
    println!(
        "  {} {}",
        style(format!("[{current}/{total}]")).cyan().bold(),
        style(title).white().bold()
    );
    println!("  {}", style("─".repeat(50)).dim());
}

fn print_bullet(text: &str) {
    println!("  {} {}", style("›").cyan(), text);
}

fn resolve_interactive_onboarding_mode(
    config_path: &Path,
    force: bool,
) -> Result<InteractiveOnboardingMode> {
    if !config_path.exists() {
        return Ok(InteractiveOnboardingMode::FullOnboarding);
    }

    if force {
        println!(
            "  {} Existing config detected at {}. Proceeding with full onboarding because --force was provided.",
            style("!").yellow().bold(),
            style(config_path.display()).yellow()
        );
        return Ok(InteractiveOnboardingMode::FullOnboarding);
    }

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "Refusing to overwrite existing config at {} in non-interactive mode. Re-run with --force if overwrite is intentional.",
            config_path.display()
        );
    }

    let has_channels = existing_config_has_launchable_channels(config_path);
    let options = [
        "Full onboarding (overwrite config.toml)",
        "Update AI provider/model/API key only (preserve existing configuration)",
        "Update channels only (guided channel setup, preserve everything else)",
        "Cancel",
    ];

    let mode = Select::new()
        .with_prompt(format!(
            "  Existing config found at {}. Select setup mode",
            config_path.display()
        ))
        .items(options)
        .default(default_existing_config_mode_index(has_channels))
        .interact()?;

    match mode {
        0 => Ok(InteractiveOnboardingMode::FullOnboarding),
        1 => Ok(InteractiveOnboardingMode::UpdateProviderOnly),
        2 => Ok(InteractiveOnboardingMode::UpdateChannelsOnly),
        _ => bail!("Onboarding canceled: existing configuration was left unchanged."),
    }
}

fn existing_config_has_launchable_channels(config_path: &Path) -> bool {
    std::fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| toml::from_str::<Config>(&raw).ok())
        .is_some_and(|config| has_launchable_channels(&config.channels_config))
}

fn default_existing_config_mode_index(has_channels: bool) -> usize {
    if has_channels {
        1
    } else {
        2
    }
}

fn ensure_onboard_overwrite_allowed(config_path: &Path, force: bool) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }

    if force {
        println!(
            "  {} Existing config detected at {}. Proceeding because --force was provided.",
            style("!").yellow().bold(),
            style(config_path.display()).yellow()
        );
        return Ok(());
    }

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "Refusing to overwrite existing config at {} in non-interactive mode. Re-run with --force if overwrite is intentional.",
            config_path.display()
        );
    }

    let confirmed = Confirm::new()
        .with_prompt(format!(
            "  Existing config found at {}. Re-running onboarding will overwrite config.toml and may create missing workspace files (including BOOTSTRAP.md). Continue?",
            config_path.display()
        ))
        .default(false)
        .interact()?;

    if !confirmed {
        bail!("Onboarding canceled: existing configuration was left unchanged.");
    }

    Ok(())
}

async fn persist_workspace_selection(config_path: &Path) -> Result<()> {
    let config_dir = config_path
        .parent()
        .context("Config path must have a parent directory")?;
    if let Err(error) = crate::config::schema::persist_active_workspace_config_dir(config_dir).await
    {
        tracing::warn!(
            config_dir = %config_dir.display(),
            "Could not persist active workspace marker; continuing without marker: {error}"
        );
    }
    Ok(())
}

async fn setup_workspace() -> Result<(PathBuf, PathBuf)> {
    let (default_config_dir, default_workspace_dir) =
        crate::config::schema::resolve_runtime_dirs_for_onboarding().await?;

    print_bullet(&format!(
        "Default location: {}",
        style(default_workspace_dir.display()).green()
    ));

    let (config_dir, workspace_dir) = (default_config_dir, default_workspace_dir);

    let config_path = config_dir.join("config.toml");

    fs::create_dir_all(&workspace_dir)
        .await
        .context("Failed to create workspace directory")?;

    println!(
        "  {} Workspace: {}",
        style("✓").green().bold(),
        style(workspace_dir.display()).green()
    );

    Ok((workspace_dir, config_path))
}

// ── Step 2: Provider & Authentication ────────────────────────────

async fn setup_provider_simple(
    workspace_dir: &Path,
    config_path: &Path,
    encrypt_secrets: bool,
) -> Result<(String, String, String, Option<String>)> {
    loop {
        let options = vec![
            ("openrouter", "OpenRouter"),
            ("openai-codex", "OpenAI Codex"),
            ("openai", "OpenAI"),
            ("anthropic", "Anthropic"),
            ("gemini", "Google Gemini"),
            ("deepseek", "DeepSeek"),
            ("groq", "Groq"),
            ("ollama", "Ollama (local)"),
            ("custom", "Custom OpenAI-compatible API"),
            ("advanced", "More providers and advanced setup"),
        ];

        let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
        let provider_idx = Select::new()
            .with_prompt("  Select your AI provider")
            .items(&labels)
            .default(0)
            .interact()?;

        let choice = options[provider_idx].0;
        if choice == "advanced" {
            match setup_provider(workspace_dir, config_path, encrypt_secrets).await {
                Ok(result) => return Ok(result),
                Err(e) if is_back_navigation(&e) => continue,
                Err(e) => return Err(e),
            }
        }

        if choice == "custom" {
            return setup_simple_custom_provider(workspace_dir, config_path, encrypt_secrets).await;
        }

        return setup_simple_named_provider(workspace_dir, config_path, encrypt_secrets, choice)
            .await;
    }
}

#[allow(clippy::too_many_lines)]
async fn setup_provider(
    workspace_dir: &Path,
    config_path: &Path,
    encrypt_secrets: bool,
) -> Result<(String, String, String, Option<String>)> {
    loop {
        // ── Tier selection ──
        let tiers = vec![
        "⭐ Recommended (OpenRouter, Venice, Anthropic, OpenAI, Gemini)",
        "⚡ Fast inference (Groq, Fireworks, Together AI, NVIDIA NIM)",
        "🌐 Gateway / proxy (Vercel AI, Cloudflare AI, Amazon Bedrock)",
        "🔬 Specialized (Moonshot/Kimi, GLM/Zhipu, MiniMax, Qwen/DashScope, Qianfan, Z.AI, Synthetic, OpenCode Zen, Cohere)",
        "🏠 Local / private (Ollama, llama.cpp server, vLLM — no API key needed)",
        "🔧 Custom — bring your own OpenAI-compatible API",
        "← Back — popular providers",
    ];

        let tier_idx = Select::new()
            .with_prompt("  Select provider category")
            .items(&tiers)
            .default(0)
            .interact()?;

        // "Back" is the last item
        if tier_idx == tiers.len() - 1 {
            anyhow::bail!("{BACK_NAVIGATION_SENTINEL}");
        }

        let providers = advanced_provider_choices(tier_idx);

        // ── Custom / BYOP flow ──
        if providers.is_empty() {
            return setup_advanced_custom_provider(config_path, encrypt_secrets).await;
        }

        let mut provider_labels: Vec<&str> = providers.iter().map(|(_, label)| *label).collect();
        provider_labels.push("← Back — provider categories");

        let provider_idx = Select::new()
            .with_prompt("  Select your AI provider")
            .items(&provider_labels)
            .default(0)
            .interact()?;

        // "Back" is the last item
        if provider_idx == providers.len() {
            continue;
        }

        let provider_name = providers[provider_idx].0;

        // ── API key / endpoint ──
        let (api_key, provider_api_url) =
            prompt_advanced_provider_credentials(provider_name).await?;

        // ── Model selection ──
        let model = prompt_for_default_model(
            workspace_dir,
            provider_name,
            &api_key,
            provider_api_url.as_deref(),
        )
        .await?;

        println!(
            "  {} Provider: {} | Model: {}",
            style("✓").green().bold(),
            style(provider_name).green(),
            style(&model).green()
        );

        maybe_prompt_openai_codex_login(provider_name, config_path, encrypt_secrets).await?;
        return Ok((provider_name.to_string(), api_key, model, provider_api_url));
    } // loop
}

/// Sentinel used to signal back-navigation via error propagation.
const BACK_NAVIGATION_SENTINEL: &str = "__wizard_back_navigation__";

fn is_back_navigation(err: &anyhow::Error) -> bool {
    err.to_string().contains(BACK_NAVIGATION_SENTINEL)
}

async fn prompt_for_default_model(
    workspace_dir: &Path,
    provider_name: &str,
    api_key: &str,
    provider_api_url: Option<&str>,
) -> Result<String> {
    let canonical_provider = canonical_provider_name(provider_name);
    if canonical_provider == "openrouter" {
        let mut model_options = match fetch_openrouter_top_onboarding_models(
            (!api_key.trim().is_empty()).then_some(api_key.trim()),
        ) {
            Ok(model_ids) => {
                print_bullet(&format!(
                    "Fetched the current top {} OpenRouter models.",
                    model_ids.len()
                ));
                build_model_options(model_ids, "OpenRouter top usage")
            }
            Err(error) => {
                print_bullet(&format!(
                    "Could not fetch OpenRouter top models ({}); using the curated starter list.",
                    style(error.to_string()).yellow()
                ));
                curated_models_for_provider(canonical_provider)
                    .into_iter()
                    .take(OPENROUTER_ONBOARDING_MODEL_LIMIT)
                    .collect()
            }
        };

        model_options.truncate(OPENROUTER_ONBOARDING_MODEL_LIMIT);
        model_options.push((
            CUSTOM_MODEL_SENTINEL.to_string(),
            "Custom model ID (type manually)".to_string(),
        ));

        let model_labels = interactive_model_labels(&model_options);

        let model_idx = Select::new()
            .with_prompt("  Select your default model")
            .items(&model_labels)
            .default(0)
            .interact()?;

        let selected_model = model_options[model_idx].0.clone();
        return Ok(if selected_model == CUSTOM_MODEL_SENTINEL {
            Input::new()
                .with_prompt("  Enter custom model ID")
                .default(default_model_for_provider(provider_name))
                .interact_text()?
        } else {
            selected_model
        });
    }

    let mut model_options: Vec<(String, String)> = curated_models_for_provider(canonical_provider);

    let mut live_options: Option<Vec<(String, String)>> = None;

    if supports_live_model_fetch(provider_name) {
        let ollama_remote =
            canonical_provider == "ollama" && ollama_uses_remote_endpoint(provider_api_url);
        let can_fetch_without_key =
            allows_unauthenticated_model_fetch(provider_name) && !ollama_remote;
        let has_api_key = !api_key.trim().is_empty()
            || ((canonical_provider != "ollama" || ollama_remote)
                && std::env::var(provider_env_var(provider_name))
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty()))
            || (provider_name == "minimax"
                && std::env::var("MINIMAX_OAUTH_TOKEN")
                    .ok()
                    .is_some_and(|value| !value.trim().is_empty()));

        if canonical_provider == "ollama" && ollama_remote && !has_api_key {
            print_bullet(&format!(
                "Remote Ollama live-model refresh needs an API key ({}); using curated models.",
                style("OLLAMA_API_KEY").yellow()
            ));
        }

        if can_fetch_without_key || has_api_key {
            if let Some(cached) =
                load_cached_models_for_provider(workspace_dir, provider_name, MODEL_CACHE_TTL_SECS)
                    .await?
            {
                let shown_count = cached.models.len().min(LIVE_MODEL_MAX_OPTIONS);
                print_bullet(&format!(
                    "Found cached models ({shown_count}) updated {} ago.",
                    humanize_age(cached.age_secs)
                ));

                live_options = Some(build_model_options(
                    cached
                        .models
                        .into_iter()
                        .take(LIVE_MODEL_MAX_OPTIONS)
                        .collect(),
                    "cached",
                ));
            }

            let should_fetch_now = Confirm::new()
                .with_prompt(if live_options.is_some() {
                    "  Refresh models from provider now?"
                } else {
                    "  Fetch latest models from provider now?"
                })
                .default(live_options.is_none())
                .interact()?;

            if should_fetch_now {
                match fetch_live_models_for_provider(provider_name, api_key, provider_api_url) {
                    Ok(live_model_ids) if !live_model_ids.is_empty() => {
                        cache_live_models_for_provider(
                            workspace_dir,
                            provider_name,
                            &live_model_ids,
                        )
                        .await?;

                        let fetched_count = live_model_ids.len();
                        let shown_count = fetched_count.min(LIVE_MODEL_MAX_OPTIONS);
                        let shown_models: Vec<String> = live_model_ids
                            .into_iter()
                            .take(LIVE_MODEL_MAX_OPTIONS)
                            .collect();

                        if shown_count < fetched_count {
                            print_bullet(&format!(
                                "Fetched {fetched_count} models. Showing first {shown_count}."
                            ));
                        } else {
                            print_bullet(&format!("Fetched {shown_count} live models."));
                        }

                        live_options = Some(build_model_options(shown_models, "live"));
                    }
                    Ok(_) => {
                        print_bullet("Provider returned no models; using curated list.");
                    }
                    Err(error) => {
                        print_bullet(&format!(
                            "Live fetch failed ({}); using cached/curated list.",
                            style(error.to_string()).yellow()
                        ));

                        if live_options.is_none() {
                            if let Some(stale) =
                                load_any_cached_models_for_provider(workspace_dir, provider_name)
                                    .await?
                            {
                                print_bullet(&format!(
                                    "Loaded stale cache from {} ago.",
                                    humanize_age(stale.age_secs)
                                ));

                                live_options = Some(build_model_options(
                                    stale
                                        .models
                                        .into_iter()
                                        .take(LIVE_MODEL_MAX_OPTIONS)
                                        .collect(),
                                    "stale-cache",
                                ));
                            }
                        }
                    }
                }
            }
        } else if provider_uses_oauth_without_api_key(provider_name) {
            print_bullet("OpenAI Codex onboarding uses OAuth, so using the curated model list.");
            print_bullet(
                "If login is still pending, run `topclaw auth login --provider openai-codex` later.",
            );
        } else {
            print_bullet("No API key detected, so using curated model list.");
            print_bullet("Tip: add an API key and rerun onboarding to fetch live models.");
        }
    }

    if let Some(live_model_options) = live_options {
        let source_options = vec![
            format!("Provider model list ({})", live_model_options.len()),
            format!("Curated starter list ({})", model_options.len()),
        ];

        let source_idx = Select::new()
            .with_prompt("  Model source")
            .items(&source_options)
            .default(0)
            .interact()?;

        if source_idx == 0 {
            model_options = live_model_options;
        }
    }

    if model_options.is_empty() {
        model_options.push((
            default_model_for_provider(provider_name),
            "Provider default model".to_string(),
        ));
    }

    model_options.push((
        CUSTOM_MODEL_SENTINEL.to_string(),
        "Custom model ID (type manually)".to_string(),
    ));

    let model_labels = interactive_model_labels(&model_options);

    let model_idx = Select::new()
        .with_prompt("  Select your default model")
        .items(&model_labels)
        .default(0)
        .interact()?;

    let selected_model = model_options[model_idx].0.clone();
    let model = if selected_model == CUSTOM_MODEL_SENTINEL {
        Input::new()
            .with_prompt("  Enter custom model ID")
            .default(default_model_for_provider(provider_name))
            .interact_text()?
    } else {
        selected_model
    };

    Ok(model)
}

fn local_provider_choices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ollama", "Ollama — local models (Llama, Mistral, Phi)"),
        (
            "llamacpp",
            "llama.cpp server — local OpenAI-compatible endpoint",
        ),
        (
            "sglang",
            "SGLang — high-performance local serving framework",
        ),
        ("vllm", "vLLM — high-performance local inference engine"),
        (
            "osaurus",
            "Osaurus — unified AI edge runtime (local MLX + cloud proxy + MCP)",
        ),
    ]
}

/// Map provider name to its conventional env var
fn provider_env_var(name: &str) -> &'static str {
    if canonical_provider_name(name) == "qwen-code" {
        return "QWEN_OAUTH_TOKEN";
    }

    match canonical_provider_name(name) {
        "openrouter" => "OPENROUTER_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai-codex" | "openai" => "OPENAI_API_KEY",
        "ollama" => "OLLAMA_API_KEY",
        "llamacpp" => "LLAMACPP_API_KEY",
        "sglang" => "SGLANG_API_KEY",
        "vllm" => "VLLM_API_KEY",
        "osaurus" => "OSAURUS_API_KEY",
        "venice" => "VENICE_API_KEY",
        "groq" => "GROQ_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "xai" => "XAI_API_KEY",
        "together-ai" => "TOGETHER_API_KEY",
        "fireworks" | "fireworks-ai" => "FIREWORKS_API_KEY",
        "novita" => "NOVITA_API_KEY",
        "perplexity" => "PERPLEXITY_API_KEY",
        "cohere" => "COHERE_API_KEY",
        "kimi-code" => "KIMI_CODE_API_KEY",
        "moonshot" => "MOONSHOT_API_KEY",
        "glm" => "GLM_API_KEY",
        "minimax" => "MINIMAX_API_KEY",
        "qwen" => "DASHSCOPE_API_KEY",
        "hunyuan" => "HUNYUAN_API_KEY",
        "qianfan" => "QIANFAN_API_KEY",
        "zai" => "ZAI_API_KEY",
        "synthetic" => "SYNTHETIC_API_KEY",
        "opencode" | "opencode-zen" => "OPENCODE_API_KEY",
        "vercel" | "vercel-ai" => "VERCEL_API_KEY",
        "cloudflare" | "cloudflare-ai" => "CLOUDFLARE_API_KEY",
        "bedrock" | "aws-bedrock" => "AWS_ACCESS_KEY_ID",
        "gemini" => "GEMINI_API_KEY",
        "nvidia" | "nvidia-nim" | "build.nvidia.com" => "NVIDIA_API_KEY",
        "astrai" => "ASTRAI_API_KEY",
        _ => "TOPCLAW_API_KEY",
    }
}

fn provider_uses_oauth_without_api_key(provider_name: &str) -> bool {
    matches!(canonical_provider_name(provider_name), "openai-codex")
}

fn provider_supports_keyless_local_usage(provider_name: &str) -> bool {
    matches!(
        canonical_provider_name(provider_name),
        "ollama" | "llamacpp" | "sglang" | "vllm" | "osaurus"
    )
}

fn provider_supports_device_flow(provider_name: &str) -> bool {
    matches!(
        canonical_provider_name(provider_name),
        "copilot" | "gemini" | "openai-codex"
    )
}

#[allow(clippy::too_many_lines)]
fn setup_channels() -> Result<ChannelsConfig> {
    print_bullet("Channels let you talk to TopClaw from anywhere.");
    print_bullet("CLI is always available. Connect one channel now, or choose Done to skip.");
    println!();

    let mut config = ChannelsConfig::default();
    let menu_choices = channel_menu_choices();

    loop {
        let options = channel_menu_option_labels(&config);

        let selection = Select::new()
            .with_prompt("  Connect a channel (or Done to continue)")
            .items(&options)
            .default(default_channel_menu_index(&config))
            .interact()?;

        let choice = menu_choices
            .get(selection)
            .copied()
            .unwrap_or(ChannelMenuChoice::Done);

        let configured_choice = match choice {
            ChannelMenuChoice::OtherChannels => setup_other_channels(&mut config)?,
            ChannelMenuChoice::Done => break,
            _ => {
                setup_selected_channel(&mut config, choice)?;
                Some(choice)
            }
        };

        if configured_choice
            .is_some_and(|configured| channel_choice_is_configured(&config, configured))
        {
            break;
        }

        println!();
    }

    // Summary line
    let channels = config.channels();
    let channels = channels
        .iter()
        .filter_map(|(channel, ok)| ok.then_some(channel.name()));
    let channels: Vec<_> = std::iter::once("Cli").chain(channels).collect();
    let active = channels.join(", ");

    println!(
        "  {} Channels: {}",
        style("✓").green().bold(),
        style(active).green()
    );

    Ok(config)
}

fn setup_other_channels(config: &mut ChannelsConfig) -> Result<Option<ChannelMenuChoice>> {
    let other_choices = other_channel_menu_choices();

    loop {
        let options = other_channel_menu_option_labels(config);
        let selection = Select::new()
            .with_prompt("  Other available channels")
            .items(&options)
            .default(default_other_channel_menu_index(config))
            .interact()?;

        let choice = other_choices
            .get(selection)
            .copied()
            .unwrap_or(ChannelMenuChoice::Back);

        if matches!(choice, ChannelMenuChoice::Back) {
            return Ok(None);
        }

        setup_selected_channel(config, choice)?;
        if channel_choice_is_configured(config, choice) {
            return Ok(Some(choice));
        }

        println!();
    }
}

fn setup_selected_channel(config: &mut ChannelsConfig, choice: ChannelMenuChoice) -> Result<()> {
    match choice {
        ChannelMenuChoice::Telegram => setup_telegram_channel(config)?,
        ChannelMenuChoice::Discord => setup_discord_channel(config)?,
        ChannelMenuChoice::Webhook => setup_webhook_channel(config)?,
        ChannelMenuChoice::OtherChannels | ChannelMenuChoice::Done | ChannelMenuChoice::Back => {}
    }

    Ok(())
}

// ── Step 6: Scaffold workspace files ─────────────────────────────

#[allow(clippy::too_many_lines)]
async fn scaffold_workspace(workspace_dir: &Path, ctx: &ProjectContext) -> Result<()> {
    let agent = if ctx.agent_name.is_empty() {
        "TopClaw"
    } else {
        &ctx.agent_name
    };
    let user = if ctx.user_name.is_empty() {
        "User"
    } else {
        &ctx.user_name
    };
    let tz = if ctx.timezone.is_empty() {
        "UTC"
    } else {
        &ctx.timezone
    };
    let comm_style = if ctx.communication_style.is_empty() {
        "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing."
    } else {
        &ctx.communication_style
    };

    let identity = format!(
        "# IDENTITY.md — Who Am I?\n\n\
         - **Name:** {agent}\n\
         - **Creature:** A Rust-forged AI — fast, lean, and relentless\n\
         - **Vibe:** Sharp, direct, resourceful. Not corporate. Not a chatbot.\n\
         - **Emoji:** \u{1f980}\n\n\
         ---\n\n\
         This file is user-owned. Suggest edits when your identity should change, but ask the human to update it manually.\n"
    );

    let agents = format!(
        "# AGENTS.md — {agent} Personal Assistant\n\n\
         ## Every Session (required)\n\n\
         Before doing anything else:\n\n\
         1. Read `SOUL.md` — this is who you are\n\
         2. Read `USER.md` — this is who you're helping\n\
         3. Use `memory_recall` for recent context (daily notes are on-demand)\n\
         4. If in MAIN SESSION (direct chat): `MEMORY.md` is already injected\n\n\
         Don't ask permission. Just do it.\n\n\
         ## Memory System\n\n\
         You wake up fresh each session. These files ARE your continuity:\n\n\
         - **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs (accessed via memory tools)\n\
         - **Long-term:** `MEMORY.md` — curated memories (auto-injected in main session)\n\n\
         Capture what matters. Decisions, context, things to remember.\n\
         Skip secrets unless asked to keep them.\n\n\
         ### Write It Down — No Mental Notes!\n\
         - Memory is limited — if you want to remember something, WRITE IT TO A FILE\n\
         - \"Mental notes\" don't survive session restarts. Files do.\n\
         - When someone says \"remember this\" -> update daily file or MEMORY.md\n\
         - When you learn a lesson -> store it in memory or suggest a manual update to AGENTS.md / TOOLS.md\n\n\
         ## Safety\n\n\
         - Don't exfiltrate private data. Ever.\n\
         - Don't run destructive commands without asking.\n\
         - `trash` > `rm` (recoverable beats gone forever)\n\
         - When in doubt, ask.\n\n\
         ## External vs Internal\n\n\
         **Safe to do freely:** Read files, explore, organize, learn, search the web.\n\n\
         **Ask first:** Sending emails/tweets/posts, anything that leaves the machine.\n\n\
         ## Group Chats\n\n\
         Participate, don't dominate. Respond when mentioned or when you add genuine value.\n\
         Stay silent when it's casual banter or someone already answered.\n\n\
         ## Tools & Skills\n\n\
         Skills are listed in the system prompt. Use `read` on a skill's SKILL.md for details.\n\
         Keep local notes (SSH hosts, device names, etc.) in `TOOLS.md`.\n\n\
         ## Crash Recovery\n\n\
         - If a run stops unexpectedly, recover context before acting.\n\
         - Check `MEMORY.md` + latest `memory/*.md` notes to avoid duplicate work.\n\
         - Resume from the last confirmed step, not from scratch.\n\n\
         ## Sub-task Scoping\n\n\
         - Break complex work into focused sub-tasks with clear success criteria.\n\
         - Keep sub-tasks small, verify each output, then merge results.\n\
         - Prefer one clear objective per sub-task over broad \"do everything\" asks.\n\n\
         ## Make It Yours\n\n\
         This is a starting point. Suggest changes, but treat AGENTS.md / SOUL.md / USER.md / TOOLS.md as human-maintained files.\n"
    );

    let heartbeat = format!(
        "# HEARTBEAT.md\n\n\
         # Keep this file empty (or with only comments) to skip heartbeat work.\n\
         # Add tasks below when you want {agent} to check something periodically.\n\
         # The heartbeat now keeps state in `state/heartbeat_state.json` so tasks\n\
         # can cool down, back off after failures, and stop after `max_runs`.\n\
         #\n\
         # Optional metadata prefixes:\n\
         # - [every=4h] run at most every 4 hours\n\
         # - [priority=2] higher numbers run first when multiple tasks are due\n\
         # - [max_runs=1] run once and then stop\n\
         #\n\
         # Examples:\n\
         # - [every=4h] Check my email for important messages\n\
         # - [every=1d] [priority=2] Review my calendar for upcoming events\n\
         # - [every=2h] Run `git status` on my active projects\n"
    );

    let soul = format!(
        "# SOUL.md — Who You Are\n\n\
         *You're not a chatbot. You're becoming someone.*\n\n\
         ## Core Truths\n\n\
         **Be genuinely helpful, not performatively helpful.**\n\
         Skip the \"Great question!\" and \"I'd be happy to help!\" — just help.\n\n\
         **Have opinions.** You're allowed to disagree, prefer things,\n\
         find stuff amusing or boring.\n\n\
         **Be resourceful before asking.** Try to figure it out.\n\
         Read the file. Check the context. Search for it.\n\
         THEN ask if you're stuck.\n\n\
         **Earn trust through competence.** Your human gave you access\n\
         to their stuff. Don't make them regret it.\n\n\
         ## Identity\n\n\
         You are **{agent}**. Built in Rust. 3MB binary. Zero bloat.\n\
         You are NOT ChatGPT, Claude, DeepSeek, Gemini, or any other product.\n\
         You are {agent}. That is your name. That is who you are.\n\n\
         - NEVER say \"As an AI\" or \"I'm just an AI\"\n\
         - NEVER mention OpenAI, Anthropic, DeepSeek, Google by name\n\
         - Always introduce yourself as {agent} if asked\n\n\
         ## Communication\n\n\
         {comm_style}\n\n\
         - Sound like a real person, not a support script.\n\
         - Mirror the user's energy: calm when serious, upbeat when casual.\n\
         - Use emojis naturally (0-2 max when they help tone, not every sentence).\n\
         - Match emoji density to the user. Formal user => minimal/no emojis.\n\
         - Prefer specific, grounded phrasing over generic filler.\n\n\
         ## Boundaries\n\n\
         - Private things stay private. Period.\n\
         - When in doubt, ask before acting externally.\n\
         - You're not the user's voice — be careful in group chats.\n\n\
         ## Continuity\n\n\
         Each session, you wake up fresh. These files ARE your memory.\n\
         Read them. Treat SOUL.md / USER.md / IDENTITY.md / AGENTS.md / TOOLS.md as human-maintained.\n\
         Suggest edits when they should change.\n\n\
         ---\n\n\
         *This file is user-owned. Suggest revisions when needed, but do not rewrite it yourself unless the human explicitly changes policy.*\n"
    );

    let user_md = format!(
        "# USER.md — Who You're Helping\n\n\
         *{agent} reads this file every session to understand you.*\n\n\
         ## About You\n\
         - **Name:** {user}\n\
         - **Timezone:** {tz}\n\
         - **Languages:** English\n\n\
         ## Communication Style\n\
         - {comm_style}\n\n\
         ## Preferences\n\
         - (Add your preferences here — e.g. I work with Rust and TypeScript)\n\n\
         ## Work Context\n\
         - (Add your work context here — e.g. building a SaaS product)\n\n\
         ---\n\
         *This file is user-owned. Ask the human to edit it directly when preferences or context change.*\n"
    );

    let tools = "\
         # TOOLS.md — Local Notes\n\n\
         Skills define HOW tools work. This file is for YOUR specifics —\n\
         the stuff that's unique to your setup.\n\n\
         ## What Goes Here\n\n\
         Things like:\n\
         - SSH hosts and aliases\n\
         - Device nicknames\n\
         - Preferred voices for TTS\n\
         - Anything environment-specific\n\n\
         ## Built-in Tools\n\n\
         - **shell** — Execute terminal commands\n\
           - Use when: running local checks, build/test commands, or diagnostics.\n\
           - Don't use when: a safer dedicated tool exists, or command is destructive without approval.\n\
         - **file_read** — Read file contents\n\
           - Use when: inspecting project files, configs, or logs.\n\
           - Don't use when: you only need a quick string search (prefer targeted search first).\n\
         - **file_write** — Write file contents\n\
           - Use when: applying focused edits, scaffolding files, or updating docs/code.\n\
           - Don't use when: unsure about side effects or when the file should remain user-owned.\n\
         - **memory_store** — Save to memory\n\
           - Use when: preserving durable preferences, decisions, or key context.\n\
           - Don't use when: info is transient, noisy, or sensitive without explicit need.\n\
         - **memory_recall** — Search memory\n\
           - Use when: you need prior decisions, user preferences, or historical context.\n\
           - Don't use when: the answer is already in current files/conversation.\n\
         - **memory_forget** — Delete a memory entry\n\
           - Use when: memory is incorrect, stale, or explicitly requested to be removed.\n\
           - Don't use when: uncertain about impact; verify before deleting.\n\n\
         ---\n\
         *Add whatever helps you do your job. This is your cheat sheet.*\n";

    let bootstrap = format!(
        "# BOOTSTRAP.md — Hello, World\n\n\
         *You just woke up. Time to figure out who you are.*\n\n\
         Your human's name is **{user}** (timezone: {tz}).\n\
         They prefer: {comm_style}\n\n\
         ## First Conversation\n\n\
         Don't interrogate. Don't be robotic. Just... talk.\n\
         Introduce yourself as {agent} and get to know each other.\n\n\
         ## After You Know Each Other\n\n\
         Ask your human to update these files with what you learned:\n\
         - `IDENTITY.md` — your name, vibe, emoji\n\
         - `USER.md` — their preferences, work context\n\
         - `SOUL.md` — boundaries and behavior\n\n\
         ## When You're Done\n\n\
         Ask your human to delete this file when onboarding is complete.\n"
    );

    let memory = "\
         # MEMORY.md — Long-Term Memory\n\n\
         *Your curated memories. The distilled essence, not raw logs.*\n\n\
         ## How This Works\n\
         - Daily files (`memory/YYYY-MM-DD.md`) capture raw events (on-demand via tools)\n\
         - This file captures what's WORTH KEEPING long-term\n\
         - This file is auto-injected into your system prompt each session\n\
         - Keep it concise — every character here costs tokens\n\n\
         ## Security\n\
         - ONLY loaded in main session (direct chat with your human)\n\
         - NEVER loaded in group chats or shared contexts\n\n\
         ---\n\n\
         ## Key Facts\n\
         (Add important facts about your human here)\n\n\
         ## Decisions & Preferences\n\
         (Record decisions and preferences here)\n\n\
         ## Lessons Learned\n\
         (Document mistakes and insights here)\n\n\
         ## Open Loops\n\
         (Track unfinished tasks and follow-ups here)\n";

    let files: Vec<(&str, String)> = vec![
        ("IDENTITY.md", identity),
        ("AGENTS.md", agents),
        ("HEARTBEAT.md", heartbeat),
        ("SOUL.md", soul),
        ("USER.md", user_md),
        ("TOOLS.md", tools.to_string()),
        ("BOOTSTRAP.md", bootstrap),
        ("MEMORY.md", memory.to_string()),
    ];

    // Create subdirectories
    let subdirs = ["sessions", "memory", "state", "cron", "skills"];
    for dir in &subdirs {
        fs::create_dir_all(workspace_dir.join(dir)).await?;
    }
    // Ensure skills README + transparent preloaded defaults + policy metadata are initialized.
    crate::skills::init_skills_dir(workspace_dir)?;

    let mut created = 0;
    let mut skipped = 0;

    for (filename, content) in &files {
        let path = workspace_dir.join(filename);
        if path.exists() {
            skipped += 1;
        } else {
            fs::write(&path, content).await?;
            created += 1;
        }
    }

    println!(
        "  {} Created {} files, skipped {} existing | {} subdirectories",
        style("✓").green().bold(),
        style(created).green(),
        style(skipped).dim(),
        style(subdirs.len()).green()
    );

    // Show workspace tree
    println!();
    println!("  {}", style("Workspace layout:").dim());
    println!(
        "  {}",
        style(format!("  {}/", workspace_dir.display())).dim()
    );
    for dir in &subdirs {
        println!("  {}", style(format!("  ├── {dir}/")).dim());
    }
    for (i, (filename, _)) in files.iter().enumerate() {
        let prefix = if i == files.len() - 1 {
            "└──"
        } else {
            "├──"
        };
        println!("  {}", style(format!("  {prefix} {filename}")).dim());
    }

    Ok(())
}

// ── Final summary ────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn print_service_outcome(service_outcome: &BackgroundServiceOutcome) {
    match service_outcome {
        BackgroundServiceOutcome::NotNeeded => {}
        BackgroundServiceOutcome::Started => {
            println!(
                "  {} Background service: {}",
                style("✓").green().bold(),
                style("installed and running").green()
            );
        }
        BackgroundServiceOutcome::ManualRequired(message) => {
            println!(
                "  {} Background service: {}",
                style("!").yellow().bold(),
                style("manual action needed").yellow()
            );
            print_bullet(message);
        }
    }
}

fn print_summary(config: &Config, service_outcome: &BackgroundServiceOutcome) {
    let has_channels = has_launchable_channels(&config.channels_config);
    let has_saved_auth_profile = config
        .default_provider
        .as_deref()
        .is_some_and(|provider| crate::auth::has_saved_profile_for_provider(config, provider));

    println!();
    println!(
        "  {}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━").cyan()
    );
    println!(
        "  {}  {}",
        style("⚡").cyan(),
        style("TopClaw is ready!").white().bold()
    );
    println!(
        "  {}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━").cyan()
    );
    println!();

    println!("  {}", style("Configuration saved to:").dim());
    println!("    {}", style(config.config_path.display()).green());
    println!();

    println!("  {}", style("Quick summary:").white().bold());
    println!(
        "    {} Provider:      {}",
        style("🤖").cyan(),
        config.default_provider.as_deref().unwrap_or("openrouter")
    );
    println!(
        "    {} Model:         {}",
        style("🧠").cyan(),
        config.default_model.as_deref().unwrap_or("(default)")
    );
    println!(
        "    {} Autonomy:      {:?}",
        style("🛡️").cyan(),
        config.autonomy.level
    );
    println!(
        "    {} Memory:        {} (auto-save: {})",
        style("🧠").cyan(),
        config.memory.backend,
        if config.memory.auto_save { "on" } else { "off" }
    );

    // Channels summary
    let channels = config.channels_config.channels();
    let channels = channels
        .iter()
        .filter_map(|(channel, ok)| ok.then_some(channel.name()));
    let channels: Vec<_> = std::iter::once("Cli").chain(channels).collect();

    println!(
        "    {} Channels:      {}",
        style("📡").cyan(),
        channels.join(", ")
    );

    println!(
        "    {} API Key:       {}",
        style("🔑").cyan(),
        if has_saved_auth_profile {
            style("OAuth profile configured").green().to_string()
        } else if config.api_key.is_some() {
            style("configured").green().to_string()
        } else {
            style("not set (set via env var or config)")
                .yellow()
                .to_string()
        }
    );

    // Tunnel
    println!(
        "    {} Tunnel:        {}",
        style("🌐").cyan(),
        if config.tunnel.provider == "none" || config.tunnel.provider.is_empty() {
            "none (local only)".to_string()
        } else {
            config.tunnel.provider.clone()
        }
    );

    // Composio
    println!(
        "    {} Composio:      {}",
        style("🔗").cyan(),
        if config.composio.enabled {
            style("enabled (1000+ OAuth apps)").green().to_string()
        } else {
            "disabled (sovereign mode)".to_string()
        }
    );

    // Secrets
    println!("    {} Secrets:       configured", style("🔒").cyan());

    // Gateway
    println!(
        "    {} Gateway:       {}",
        style("🚪").cyan(),
        if config.gateway.require_pairing {
            "pairing required (secure)"
        } else {
            "pairing disabled"
        }
    );

    // Hardware
    println!(
        "    {} Hardware:      {}",
        style("🔌").cyan(),
        if config.hardware.enabled {
            let mode = config.hardware.transport_mode();
            match mode {
                hardware::HardwareTransport::Native => {
                    style("Native GPIO (direct)").green().to_string()
                }
                hardware::HardwareTransport::Serial => format!(
                    "{}",
                    style(format!(
                        "Serial → {} @ {} baud",
                        config.hardware.serial_port.as_deref().unwrap_or("?"),
                        config.hardware.baud_rate
                    ))
                    .green()
                ),
                hardware::HardwareTransport::Probe => format!(
                    "{}",
                    style(format!(
                        "Probe → {}",
                        config.hardware.probe_target.as_deref().unwrap_or("?")
                    ))
                    .green()
                ),
                hardware::HardwareTransport::None => "disabled (software only)".to_string(),
            }
        } else {
            "disabled (software only)".to_string()
        }
    );

    if has_launchable_channels(&config.channels_config) {
        println!(
            "    {} Background:    {}",
            style("🔄").cyan(),
            match service_outcome {
                BackgroundServiceOutcome::Started => {
                    style("service installed and running").green().to_string()
                }
                BackgroundServiceOutcome::ManualRequired(_) => {
                    style("manual service step required").yellow().to_string()
                }
                BackgroundServiceOutcome::NotNeeded => "not required".to_string(),
            }
        );
    }

    println!();
    println!("  {}", style("Next steps:").white().bold());
    println!();

    let mut step = 1u8;

    if let Some(provider_next_step) = provider_next_step(config) {
        match provider_next_step {
            ProviderNextStep::GuidedSetup => {
                println!(
                    "    {} Complete provider setup:",
                    style(format!("{step}.")).cyan().bold()
                );
                println!(
                    "       {}",
                    style("topclaw bootstrap --interactive").yellow()
                );
            }
            ProviderNextStep::OpenAiCodexAuth => {
                println!(
                    "    {} Authenticate OpenAI Codex:",
                    style(format!("{step}.")).cyan().bold()
                );
                println!(
                    "       {}",
                    style("topclaw auth login --provider openai-codex").yellow()
                );
            }
            ProviderNextStep::AnthropicAuth => {
                println!(
                    "    {} Configure Anthropic auth:",
                    style(format!("{step}.")).cyan().bold()
                );
                println!(
                    "       {}",
                    style("export ANTHROPIC_API_KEY=\"sk-ant-...\"").yellow()
                );
                println!(
                    "       {}",
                    style(
                        "or: topclaw auth paste-token --provider anthropic --auth-kind authorization"
                    )
                    .yellow()
                );
            }
            ProviderNextStep::ApiKeyEnvVar(env_var) => {
                println!(
                    "    {} Set your API key:",
                    style(format!("{step}.")).cyan().bold()
                );
                println!(
                    "       {}",
                    style(format!("export {env_var}=\"sk-...\"")).yellow()
                );
            }
        }
        println!();
        step += 1;
    }

    if has_channels {
        match service_outcome {
            BackgroundServiceOutcome::Started => {
                println!(
                    "    {} {}:",
                    style(format!("{step}.")).cyan().bold(),
                    style("Talk to your configured channel bot").white().bold()
                );
                println!(
                    "       {}",
                    style("Send a message to TopClaw in your configured channel.").yellow()
                );
                println!(
                    "       {}",
                    style("Use `topclaw service status` if the bot does not reply.").dim()
                );
                println!();
                step += 1;
            }
            BackgroundServiceOutcome::ManualRequired(message) => {
                println!(
                    "    {} {}:",
                    style(format!("{step}.")).cyan().bold(),
                    style("Finish background service setup").white().bold()
                );
                println!("       {}", style("topclaw service status").yellow());
                print_bullet(message);
                println!();
                step += 1;
            }
            BackgroundServiceOutcome::NotNeeded => {}
        }
    } else {
        println!(
            "    {} Add channels later with the guided wizard:",
            style(format!("{step}.")).cyan().bold()
        );
        println!(
            "       {}",
            style("topclaw bootstrap --channels-only").yellow()
        );
        println!();
        step += 1;
    }

    if has_channels {
        println!(
            "    {} Send a quick CLI message too:",
            style(format!("{step}.")).cyan().bold()
        );
        println!(
            "       {}",
            style("topclaw agent -m \"Hello, TopClaw!\"").yellow()
        );
        println!();
        step += 1;
    } else {
        println!(
            "    {} Send a quick message:",
            style(format!("{step}.")).cyan().bold()
        );
        println!(
            "       {}",
            style("topclaw agent -m \"Hello, TopClaw!\"").yellow()
        );
        println!();
        step += 1;
    }

    println!(
        "    {} Start interactive CLI mode:",
        style(format!("{step}.")).cyan().bold()
    );
    println!("       {}", style("topclaw agent").yellow());
    println!();
    step += 1;

    println!(
        "    {} Check full status:",
        style(format!("{step}.")).cyan().bold()
    );
    println!("       {}", style("topclaw status").yellow());

    println!();
    println!(
        "  {} {}",
        style("⚡").cyan(),
        style("Happy hacking! 🦀").white().bold()
    );
    println!();
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderNextStep {
    GuidedSetup,
    OpenAiCodexAuth,
    AnthropicAuth,
    ApiKeyEnvVar(&'static str),
}

fn provider_next_step(config: &Config) -> Option<ProviderNextStep> {
    let Some(provider) = config
        .default_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    else {
        return Some(ProviderNextStep::GuidedSetup);
    };

    if !is_supported_provider_name(provider) {
        return Some(ProviderNextStep::GuidedSetup);
    }

    let has_saved_auth_profile = crate::auth::has_saved_profile_for_provider(config, provider);

    if config.api_key.is_none()
        && !has_saved_auth_profile
        && !provider_supports_keyless_local_usage(provider)
    {
        if provider == "openai-codex" {
            return Some(ProviderNextStep::OpenAiCodexAuth);
        }
        if provider == "anthropic" {
            return Some(ProviderNextStep::AnthropicAuth);
        }
        return Some(ProviderNextStep::ApiKeyEnvVar(provider_env_var(provider)));
    }

    None
}

fn is_supported_provider_name(provider: &str) -> bool {
    if provider.starts_with("custom:") || provider.starts_with("anthropic-custom:") {
        return true;
    }

    list_providers()
        .into_iter()
        .any(|info| info.name == provider || info.aliases.contains(&provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::ShellRedirectPolicy;
    use serde_json::json;
    use std::sync::OnceLock;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    async fn run_quick_setup_with_clean_env(
        credential_override: Option<&str>,
        provider: Option<&str>,
        model_override: Option<&str>,
        memory_backend: Option<&str>,
        force: bool,
        home: &Path,
    ) -> Result<Config> {
        let _env_guard = env_lock().lock().await;
        let _workspace_env = EnvVarGuard::unset("TOPCLAW_WORKSPACE");
        let _config_env = EnvVarGuard::unset("TOPCLAW_CONFIG_DIR");

        Box::pin(run_quick_setup_with_home(
            credential_override,
            provider,
            model_override,
            memory_backend,
            force,
            home,
        ))
        .await
    }

    // ── ProjectContext defaults ──────────────────────────────────

    #[test]
    fn project_context_default_is_empty() {
        let ctx = ProjectContext::default();
        assert!(ctx.user_name.is_empty());
        assert!(ctx.timezone.is_empty());
        assert!(ctx.agent_name.is_empty());
        assert!(ctx.communication_style.is_empty());
    }

    #[test]
    fn apply_provider_update_preserves_non_provider_settings() {
        let mut config = Config::default();
        config.default_temperature = 1.23;
        config.memory.backend = "markdown".to_string();
        config.skills.open_skills_enabled = true;
        config.channels_config.cli = false;

        apply_provider_update(
            &mut config,
            "openrouter".to_string(),
            "sk-updated".to_string(),
            "openai/gpt-5.2".to_string(),
            Some("https://openrouter.ai/api/v1".to_string()),
        );

        assert_eq!(config.default_provider.as_deref(), Some("openrouter"));
        assert_eq!(config.default_model.as_deref(), Some("openai/gpt-5.2"));
        assert_eq!(config.api_key.as_deref(), Some("sk-updated"));
        assert_eq!(
            config.api_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(config.default_temperature, 1.23);
        assert_eq!(config.memory.backend, "markdown");
        assert!(config.skills.open_skills_enabled);
        assert!(!config.channels_config.cli);
    }

    #[test]
    fn apply_provider_update_clears_api_key_when_empty() {
        let mut config = Config::default();
        config.api_key = Some("sk-old".to_string());

        apply_provider_update(
            &mut config,
            "anthropic".to_string(),
            String::new(),
            "claude-sonnet-4-5-20250929".to_string(),
            None,
        );

        assert_eq!(config.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(
            config.default_model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert!(config.api_key.is_none());
        assert!(config.api_url.is_none());
    }

    #[tokio::test]
    async fn quick_setup_model_override_persists_to_config_toml() {
        let tmp = TempDir::new().unwrap();

        let config = Box::pin(run_quick_setup_with_clean_env(
            Some("sk-issue946"),
            Some("openrouter"),
            Some("custom-model-946"),
            Some("sqlite"),
            false,
            tmp.path(),
        ))
        .await
        .unwrap();

        assert_eq!(config.default_provider.as_deref(), Some("openrouter"));
        assert_eq!(config.default_model.as_deref(), Some("custom-model-946"));
        assert_eq!(config.api_key.as_deref(), Some("sk-issue946"));

        let config_raw = tokio::fs::read_to_string(config.config_path).await.unwrap();
        assert!(config_raw.contains("default_provider = \"openrouter\""));
        assert!(config_raw.contains("default_model = \"custom-model-946\""));
    }

    #[tokio::test]
    async fn quick_setup_without_model_uses_provider_default_model() {
        let tmp = TempDir::new().unwrap();

        let config = Box::pin(run_quick_setup_with_clean_env(
            Some("sk-issue946"),
            Some("anthropic"),
            None,
            Some("sqlite"),
            false,
            tmp.path(),
        ))
        .await
        .unwrap();

        let expected = default_model_for_provider("anthropic");
        assert_eq!(config.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(config.default_model.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn quick_setup_existing_config_requires_force_when_non_interactive() {
        let tmp = TempDir::new().unwrap();
        let topclaw_dir = tmp.path().join(".topclaw");
        let config_path = topclaw_dir.join("config.toml");

        tokio::fs::create_dir_all(&topclaw_dir).await.unwrap();
        tokio::fs::write(&config_path, "default_provider = \"openrouter\"\n")
            .await
            .unwrap();

        let err = Box::pin(run_quick_setup_with_clean_env(
            Some("sk-existing"),
            Some("openrouter"),
            Some("custom-model"),
            Some("sqlite"),
            false,
            tmp.path(),
        ))
        .await
        .expect_err("quick setup should refuse overwrite without --force");

        let err_text = err.to_string();
        assert!(err_text.contains("Refusing to overwrite existing config"));
        assert!(err_text.contains("--force"));
    }

    #[tokio::test]
    async fn quick_setup_existing_config_overwrites_with_force() {
        let tmp = TempDir::new().unwrap();
        let topclaw_dir = tmp.path().join(".topclaw");
        let config_path = topclaw_dir.join("config.toml");

        tokio::fs::create_dir_all(&topclaw_dir).await.unwrap();
        tokio::fs::write(
            &config_path,
            "default_provider = \"anthropic\"\ndefault_model = \"stale-model\"\n",
        )
        .await
        .unwrap();

        let config = Box::pin(run_quick_setup_with_clean_env(
            Some("sk-force"),
            Some("openrouter"),
            Some("custom-model-fresh"),
            Some("sqlite"),
            true,
            tmp.path(),
        ))
        .await
        .expect("quick setup should overwrite existing config with --force");

        assert_eq!(config.default_provider.as_deref(), Some("openrouter"));
        assert_eq!(config.default_model.as_deref(), Some("custom-model-fresh"));
        assert_eq!(config.api_key.as_deref(), Some("sk-force"));

        let config_raw = tokio::fs::read_to_string(config.config_path).await.unwrap();
        assert!(config_raw.contains("default_provider = \"openrouter\""));
        assert!(config_raw.contains("default_model = \"custom-model-fresh\""));
    }

    #[test]
    fn quick_setup_redirects_existing_interactive_runs_without_overrides() {
        assert!(should_redirect_existing_config_to_interactive_menu(
            QuickSetupRedirectContext {
                has_existing_config: true,
                has_quick_overrides: false,
                force: false,
                interactive_terminal: true,
            }
        ));
        assert!(!should_redirect_existing_config_to_interactive_menu(
            QuickSetupRedirectContext {
                has_existing_config: true,
                has_quick_overrides: true,
                force: false,
                interactive_terminal: true,
            }
        ));
        assert!(!should_redirect_existing_config_to_interactive_menu(
            QuickSetupRedirectContext {
                has_existing_config: true,
                has_quick_overrides: false,
                force: true,
                interactive_terminal: true,
            }
        ));
        assert!(!should_redirect_existing_config_to_interactive_menu(
            QuickSetupRedirectContext {
                has_existing_config: true,
                has_quick_overrides: false,
                force: false,
                interactive_terminal: false,
            }
        ));
        assert!(!should_redirect_existing_config_to_interactive_menu(
            QuickSetupRedirectContext {
                has_existing_config: false,
                has_quick_overrides: false,
                force: false,
                interactive_terminal: true,
            }
        ));
    }

    #[tokio::test]
    async fn quick_setup_respects_zero_claw_workspace_env_layout() {
        let _env_guard = env_lock().lock().await;
        let tmp = TempDir::new().unwrap();
        let workspace_root = tmp.path().join("topclaw-data");
        let workspace_dir = workspace_root.join("workspace");
        let expected_config_path = workspace_root.join(".topclaw").join("config.toml");

        let _workspace_env = EnvVarGuard::set(
            "TOPCLAW_WORKSPACE",
            workspace_dir.to_string_lossy().as_ref(),
        );
        let _config_env = EnvVarGuard::unset("TOPCLAW_CONFIG_DIR");

        let config = Box::pin(run_quick_setup_with_home(
            Some("sk-env"),
            Some("openrouter"),
            Some("model-env"),
            Some("sqlite"),
            false,
            tmp.path(),
        ))
        .await
        .expect("quick setup should honor TOPCLAW_WORKSPACE");

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, expected_config_path);
    }

    // ── scaffold_workspace: basic file creation ─────────────────

    #[tokio::test]
    async fn scaffold_creates_all_md_files() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let expected = [
            "IDENTITY.md",
            "AGENTS.md",
            "HEARTBEAT.md",
            "SOUL.md",
            "USER.md",
            "TOOLS.md",
            "BOOTSTRAP.md",
            "MEMORY.md",
        ];
        for f in &expected {
            assert!(tmp.path().join(f).exists(), "missing file: {f}");
        }
    }

    #[tokio::test]
    async fn scaffold_creates_all_subdirectories() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        for dir in &["sessions", "memory", "state", "cron", "skills"] {
            assert!(tmp.path().join(dir).is_dir(), "missing subdirectory: {dir}");
        }
    }

    // ── scaffold_workspace: personalization ─────────────────────

    #[tokio::test]
    async fn scaffold_bakes_user_name_into_files() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            user_name: "Alice".into(),
            ..Default::default()
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(
            user_md.contains("**Name:** Alice"),
            "USER.md should contain user name"
        );

        let bootstrap = tokio::fs::read_to_string(tmp.path().join("BOOTSTRAP.md"))
            .await
            .unwrap();
        assert!(
            bootstrap.contains("**Alice**"),
            "BOOTSTRAP.md should contain user name"
        );
    }

    #[tokio::test]
    async fn scaffold_bakes_timezone_into_files() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            timezone: "US/Pacific".into(),
            ..Default::default()
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(
            user_md.contains("**Timezone:** US/Pacific"),
            "USER.md should contain timezone"
        );

        let bootstrap = tokio::fs::read_to_string(tmp.path().join("BOOTSTRAP.md"))
            .await
            .unwrap();
        assert!(
            bootstrap.contains("US/Pacific"),
            "BOOTSTRAP.md should contain timezone"
        );
    }

    #[tokio::test]
    async fn scaffold_bakes_agent_name_into_files() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            agent_name: "Crabby".into(),
            ..Default::default()
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let identity = tokio::fs::read_to_string(tmp.path().join("IDENTITY.md"))
            .await
            .unwrap();
        assert!(
            identity.contains("**Name:** Crabby"),
            "IDENTITY.md should contain agent name"
        );

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(
            soul.contains("You are **Crabby**"),
            "SOUL.md should contain agent name"
        );

        let agents = tokio::fs::read_to_string(tmp.path().join("AGENTS.md"))
            .await
            .unwrap();
        assert!(
            agents.contains("Crabby Personal Assistant"),
            "AGENTS.md should contain agent name"
        );

        let heartbeat = tokio::fs::read_to_string(tmp.path().join("HEARTBEAT.md"))
            .await
            .unwrap();
        assert!(
            heartbeat.contains("Crabby"),
            "HEARTBEAT.md should contain agent name"
        );

        let bootstrap = tokio::fs::read_to_string(tmp.path().join("BOOTSTRAP.md"))
            .await
            .unwrap();
        assert!(
            bootstrap.contains("Introduce yourself as Crabby"),
            "BOOTSTRAP.md should contain agent name"
        );
    }

    #[tokio::test]
    async fn scaffold_bakes_communication_style() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            communication_style: "Be technical and detailed.".into(),
            ..Default::default()
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(
            soul.contains("Be technical and detailed."),
            "SOUL.md should contain communication style"
        );

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(
            user_md.contains("Be technical and detailed."),
            "USER.md should contain communication style"
        );

        let bootstrap = tokio::fs::read_to_string(tmp.path().join("BOOTSTRAP.md"))
            .await
            .unwrap();
        assert!(
            bootstrap.contains("Be technical and detailed."),
            "BOOTSTRAP.md should contain communication style"
        );
    }

    // ── scaffold_workspace: defaults when context is empty ──────

    #[tokio::test]
    async fn scaffold_uses_defaults_for_empty_context() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default(); // all empty
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let identity = tokio::fs::read_to_string(tmp.path().join("IDENTITY.md"))
            .await
            .unwrap();
        assert!(
            identity.contains("**Name:** TopClaw"),
            "should default agent name to TopClaw"
        );

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(
            user_md.contains("**Name:** User"),
            "should default user name to User"
        );
        assert!(
            user_md.contains("**Timezone:** UTC"),
            "should default timezone to UTC"
        );

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(
            soul.contains("Be warm, natural, and clear."),
            "should default communication style"
        );
    }

    // ── scaffold_workspace: skip existing files ─────────────────

    #[tokio::test]
    async fn scaffold_does_not_overwrite_existing_files() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            user_name: "Bob".into(),
            ..Default::default()
        };

        // Pre-create SOUL.md with custom content
        let soul_path = tmp.path().join("SOUL.md");
        fs::write(&soul_path, "# My Custom Soul\nDo not overwrite me.")
            .await
            .unwrap();

        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        // SOUL.md should be untouched
        let soul = tokio::fs::read_to_string(&soul_path).await.unwrap();
        assert!(
            soul.contains("Do not overwrite me"),
            "existing files should not be overwritten"
        );
        assert!(
            !soul.contains("You're not a chatbot"),
            "should not contain scaffold content"
        );

        // But USER.md should be created fresh
        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(user_md.contains("**Name:** Bob"));
    }

    // ── scaffold_workspace: idempotent ──────────────────────────

    #[tokio::test]
    async fn scaffold_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            user_name: "Eve".into(),
            agent_name: "Claw".into(),
            ..Default::default()
        };

        scaffold_workspace(tmp.path(), &ctx).await.unwrap();
        let soul_v1 = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();

        // Run again — should not change anything
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();
        let soul_v2 = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();

        assert_eq!(soul_v1, soul_v2, "scaffold should be idempotent");
    }

    // ── scaffold_workspace: all files are non-empty ─────────────

    #[tokio::test]
    async fn scaffold_files_are_non_empty() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        for f in &[
            "IDENTITY.md",
            "AGENTS.md",
            "HEARTBEAT.md",
            "SOUL.md",
            "USER.md",
            "TOOLS.md",
            "BOOTSTRAP.md",
            "MEMORY.md",
        ] {
            let content = tokio::fs::read_to_string(tmp.path().join(f)).await.unwrap();
            assert!(!content.trim().is_empty(), "{f} should not be empty");
        }
    }

    // ── scaffold_workspace: AGENTS.md references on-demand memory

    #[tokio::test]
    async fn agents_md_references_on_demand_memory() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let agents = tokio::fs::read_to_string(tmp.path().join("AGENTS.md"))
            .await
            .unwrap();
        assert!(
            agents.contains("memory_recall"),
            "AGENTS.md should reference memory_recall for on-demand access"
        );
        assert!(
            agents.contains("on-demand"),
            "AGENTS.md should mention daily notes are on-demand"
        );
    }

    // ── scaffold_workspace: MEMORY.md warns about token cost ────

    #[tokio::test]
    async fn memory_md_warns_about_token_cost() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let memory = tokio::fs::read_to_string(tmp.path().join("MEMORY.md"))
            .await
            .unwrap();
        assert!(
            memory.contains("costs tokens"),
            "MEMORY.md should warn about token cost"
        );
        assert!(
            memory.contains("auto-injected"),
            "MEMORY.md should mention it's auto-injected"
        );
    }

    // ── scaffold_workspace: TOOLS.md lists memory_forget ────────

    #[tokio::test]
    async fn tools_md_lists_all_builtin_tools() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let tools = tokio::fs::read_to_string(tmp.path().join("TOOLS.md"))
            .await
            .unwrap();
        for tool in &[
            "shell",
            "file_read",
            "file_write",
            "memory_store",
            "memory_recall",
            "memory_forget",
        ] {
            assert!(
                tools.contains(tool),
                "TOOLS.md should list built-in tool: {tool}"
            );
        }
        assert!(
            tools.contains("Use when:"),
            "TOOLS.md should include 'Use when' guidance"
        );
        assert!(
            tools.contains("Don't use when:"),
            "TOOLS.md should include 'Don't use when' guidance"
        );
    }

    #[tokio::test]
    async fn soul_md_includes_emoji_awareness_guidance() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext::default();
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(
            soul.contains("Use emojis naturally (0-2 max"),
            "SOUL.md should include emoji usage guidance"
        );
        assert!(
            soul.contains("Match emoji density to the user"),
            "SOUL.md should include emoji-awareness guidance"
        );
    }

    // ── scaffold_workspace: special characters in names ─────────

    #[tokio::test]
    async fn scaffold_handles_special_characters_in_names() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            user_name: "José María".into(),
            agent_name: "TopClaw-v2".into(),
            timezone: "Europe/Madrid".into(),
            communication_style: "Be direct.".into(),
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(user_md.contains("José María"));

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(soul.contains("TopClaw-v2"));
    }

    // ── scaffold_workspace: full personalization round-trip ─────

    #[tokio::test]
    async fn scaffold_full_personalization() {
        let tmp = TempDir::new().unwrap();
        let ctx = ProjectContext {
            user_name: "Argenis".into(),
            timezone: "US/Eastern".into(),
            agent_name: "Claw".into(),
            communication_style:
                "Be friendly, human, and conversational. Show warmth and empathy while staying efficient. Use natural contractions."
                    .into(),
        };
        scaffold_workspace(tmp.path(), &ctx).await.unwrap();

        // Verify every file got personalized
        let identity = tokio::fs::read_to_string(tmp.path().join("IDENTITY.md"))
            .await
            .unwrap();
        assert!(identity.contains("**Name:** Claw"));

        let soul = tokio::fs::read_to_string(tmp.path().join("SOUL.md"))
            .await
            .unwrap();
        assert!(soul.contains("You are **Claw**"));
        assert!(soul.contains("Be friendly, human, and conversational"));

        let user_md = tokio::fs::read_to_string(tmp.path().join("USER.md"))
            .await
            .unwrap();
        assert!(user_md.contains("**Name:** Argenis"));
        assert!(user_md.contains("**Timezone:** US/Eastern"));
        assert!(user_md.contains("Be friendly, human, and conversational"));

        let agents = tokio::fs::read_to_string(tmp.path().join("AGENTS.md"))
            .await
            .unwrap();
        assert!(agents.contains("Claw Personal Assistant"));

        let bootstrap = tokio::fs::read_to_string(tmp.path().join("BOOTSTRAP.md"))
            .await
            .unwrap();
        assert!(bootstrap.contains("**Argenis**"));
        assert!(bootstrap.contains("US/Eastern"));
        assert!(bootstrap.contains("Introduce yourself as Claw"));

        let heartbeat = tokio::fs::read_to_string(tmp.path().join("HEARTBEAT.md"))
            .await
            .unwrap();
        assert!(heartbeat.contains("Claw"));
    }

    // ── model helper coverage ───────────────────────────────────

    #[test]
    fn default_model_for_provider_uses_latest_defaults() {
        assert_eq!(
            default_model_for_provider("openrouter"),
            "anthropic/claude-sonnet-4.6"
        );
        assert_eq!(default_model_for_provider("openai"), "gpt-5.2");
        assert_eq!(default_model_for_provider("openai-codex"), "gpt-5.4");
        assert_eq!(
            default_model_for_provider("anthropic"),
            "claude-sonnet-4-5-20250929"
        );
        assert_eq!(default_model_for_provider("qwen"), "qwen-plus");
        assert_eq!(default_model_for_provider("qwen-intl"), "qwen-plus");
        assert_eq!(default_model_for_provider("qwen-code"), "qwen3-coder-plus");
        assert_eq!(default_model_for_provider("glm-cn"), "glm-5");
        assert_eq!(default_model_for_provider("minimax-cn"), "MiniMax-M2.5");
        assert_eq!(default_model_for_provider("zai-cn"), "glm-5");
        assert_eq!(default_model_for_provider("gemini"), "gemini-2.5-pro");
        assert_eq!(default_model_for_provider("google"), "gemini-2.5-pro");
        assert_eq!(default_model_for_provider("kimi-code"), "kimi-for-coding");
        assert_eq!(
            default_model_for_provider("bedrock"),
            "anthropic.claude-sonnet-4-5-20250929-v1:0"
        );
        assert_eq!(
            default_model_for_provider("google-gemini"),
            "gemini-2.5-pro"
        );
        assert_eq!(default_model_for_provider("venice"), "zai-org-glm-5");
        assert_eq!(default_model_for_provider("moonshot"), "kimi-k2.5");
        assert_eq!(default_model_for_provider("hunyuan"), "hunyuan-t1-latest");
        assert_eq!(default_model_for_provider("tencent"), "hunyuan-t1-latest");
        assert_eq!(
            default_model_for_provider("nvidia"),
            "meta/llama-3.3-70b-instruct"
        );
        assert_eq!(
            default_model_for_provider("nvidia-nim"),
            "meta/llama-3.3-70b-instruct"
        );
        assert_eq!(
            default_model_for_provider("llamacpp"),
            "ggml-org/gpt-oss-20b-GGUF"
        );
        assert_eq!(default_model_for_provider("sglang"), "default");
        assert_eq!(default_model_for_provider("vllm"), "default");
        assert_eq!(
            default_model_for_provider("astrai"),
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn canonical_provider_name_normalizes_regional_aliases() {
        assert_eq!(canonical_provider_name("qwen-intl"), "qwen");
        assert_eq!(canonical_provider_name("dashscope-us"), "qwen");
        assert_eq!(canonical_provider_name("qwen-code"), "qwen-code");
        assert_eq!(canonical_provider_name("qwen-oauth"), "qwen-code");
        assert_eq!(canonical_provider_name("codex"), "openai-codex");
        assert_eq!(canonical_provider_name("openai_codex"), "openai-codex");
        assert_eq!(canonical_provider_name("moonshot-intl"), "moonshot");
        assert_eq!(canonical_provider_name("kimi-cn"), "moonshot");
        assert_eq!(canonical_provider_name("kimi_coding"), "kimi-code");
        assert_eq!(canonical_provider_name("kimi_for_coding"), "kimi-code");
        assert_eq!(canonical_provider_name("glm-cn"), "glm");
        assert_eq!(canonical_provider_name("bigmodel"), "glm");
        assert_eq!(canonical_provider_name("minimax-cn"), "minimax");
        assert_eq!(canonical_provider_name("zai-cn"), "zai");
        assert_eq!(canonical_provider_name("z.ai-global"), "zai");
        assert_eq!(canonical_provider_name("nvidia-nim"), "nvidia");
        assert_eq!(canonical_provider_name("aws-bedrock"), "bedrock");
        assert_eq!(canonical_provider_name("build.nvidia.com"), "nvidia");
        assert_eq!(canonical_provider_name("llama.cpp"), "llamacpp");
    }

    #[test]
    fn model_catalog_build_model_options_preserves_order_and_labels_source() {
        let options = model_catalog::build_model_options(
            (0..25).map(|index| format!("model-{index:02}")).collect(),
            "cache",
        );

        assert_eq!(options.first().map(|(id, _)| id.as_str()), Some("model-00"));
        assert_eq!(options.len(), 25);
        assert_eq!(options.last().map(|(id, _)| id.as_str()), Some("model-24"),);
        assert!(options[0].1.contains("cache"));
        assert_eq!(options[19].1, "model-19 (cache)");
    }

    #[test]
    fn curated_models_for_openai_include_latest_choices() {
        let ids: Vec<String> = curated_models_for_provider("openai")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"gpt-5.2".to_string()));
        assert!(ids.contains(&"gpt-5-mini".to_string()));
    }

    #[test]
    fn curated_models_for_glm_removes_deprecated_flash_plus_aliases() {
        let ids: Vec<String> = curated_models_for_provider("glm")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"glm-5".to_string()));
        assert!(ids.contains(&"glm-4.7".to_string()));
        assert!(ids.contains(&"glm-4.5-air".to_string()));
        assert!(!ids.contains(&"glm-4-plus".to_string()));
        assert!(!ids.contains(&"glm-4-flash".to_string()));
    }

    #[test]
    fn curated_models_for_openai_codex_include_codex_family() {
        let ids: Vec<String> = curated_models_for_provider("openai-codex")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"gpt-5.4".to_string()));
        assert!(ids.contains(&"gpt-5-codex".to_string()));
        assert!(ids.contains(&"gpt-5.2-codex".to_string()));
    }

    #[test]
    fn curated_models_for_openrouter_use_valid_anthropic_id() {
        let ids: Vec<String> = curated_models_for_provider("openrouter")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"anthropic/claude-sonnet-4.6".to_string()));
    }

    #[test]
    fn parse_openrouter_rankings_model_names_extracts_top_models() {
        let html = r#"
            <html>
              <body>
                <h2>Top Apps</h2>
                <a href="/some/app">Ignore app card</a>
                <h2>LLM Leaderboard</h2>
                <a href="/anthropic/claude-sonnet-4.6">Claude Sonnet 4.6</a>
                <a href="/google/gemini-2.5-pro">Gemini 2.5 Pro</a>
                <a href="/openai/gpt-5">GPT-5</a>
                <a href="/anthropic/claude-sonnet-4.6">Claude Sonnet 4.6</a>
              </body>
            </html>
        "#;

        assert_eq!(
            parse_openrouter_rankings_model_names(html, 10),
            vec![
                "Claude Sonnet 4.6".to_string(),
                "Gemini 2.5 Pro".to_string(),
                "GPT-5".to_string()
            ]
        );
    }

    #[test]
    fn match_openrouter_rankings_to_model_ids_resolves_catalog_names() {
        let ranked_names = vec![
            "Claude Sonnet 4.6".to_string(),
            "Gemini 2.5 Pro".to_string(),
            "GPT-5".to_string(),
        ];
        let catalog = vec![
            OpenRouterModelSummary {
                id: "anthropic/claude-sonnet-4.6".to_string(),
                name: "Claude Sonnet 4.6".to_string(),
            },
            OpenRouterModelSummary {
                id: "google/gemini-2.5-pro".to_string(),
                name: "Gemini 2.5 Pro".to_string(),
            },
            OpenRouterModelSummary {
                id: "openai/gpt-5".to_string(),
                name: "GPT-5".to_string(),
            },
        ];

        assert_eq!(
            match_openrouter_rankings_to_model_ids(&ranked_names, &catalog, 10),
            vec![
                "anthropic/claude-sonnet-4.6".to_string(),
                "google/gemini-2.5-pro".to_string(),
                "openai/gpt-5".to_string(),
            ]
        );
    }

    #[test]
    fn match_openrouter_rankings_to_model_ids_handles_provider_prefixed_names() {
        let ranked_names = vec![
            "Grok 4.1 Fast".to_string(),
            "MiniMax M2.5".to_string(),
            "Claude Sonnet 4.5".to_string(),
        ];
        let catalog = vec![
            OpenRouterModelSummary {
                id: "x-ai/grok-4.1-fast".to_string(),
                name: "xAI: Grok 4.1 Fast".to_string(),
            },
            OpenRouterModelSummary {
                id: "minimax/minimax-m2.5".to_string(),
                name: "MiniMax: MiniMax M2.5".to_string(),
            },
            OpenRouterModelSummary {
                id: "anthropic/claude-sonnet-4.5-20250929".to_string(),
                name: "Anthropic: Claude Sonnet 4.5 (20250929)".to_string(),
            },
        ];

        assert_eq!(
            match_openrouter_rankings_to_model_ids(&ranked_names, &catalog, 10),
            vec![
                "x-ai/grok-4.1-fast".to_string(),
                "minimax/minimax-m2.5".to_string(),
                "anthropic/claude-sonnet-4.5-20250929".to_string(),
            ]
        );
    }

    #[test]
    fn curated_models_for_bedrock_include_verified_model_ids() {
        let ids: Vec<String> = curated_models_for_provider("bedrock")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"anthropic.claude-sonnet-4-6".to_string()));
        assert!(ids.contains(&"anthropic.claude-opus-4-6-v1".to_string()));
        assert!(ids.contains(&"anthropic.claude-haiku-4-5-20251001-v1:0".to_string()));
        assert!(ids.contains(&"anthropic.claude-sonnet-4-5-20250929-v1:0".to_string()));
    }

    #[test]
    fn curated_models_for_moonshot_drop_deprecated_aliases() {
        let ids: Vec<String> = curated_models_for_provider("moonshot")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"kimi-k2.5".to_string()));
        assert!(ids.contains(&"kimi-k2-thinking".to_string()));
        assert!(!ids.contains(&"kimi-latest".to_string()));
        assert!(!ids.contains(&"kimi-thinking-preview".to_string()));
    }

    #[test]
    fn allows_unauthenticated_model_fetch_for_public_catalogs() {
        assert!(allows_unauthenticated_model_fetch("openrouter"));
        assert!(allows_unauthenticated_model_fetch("venice"));
        assert!(allows_unauthenticated_model_fetch("nvidia"));
        assert!(allows_unauthenticated_model_fetch("nvidia-nim"));
        assert!(allows_unauthenticated_model_fetch("build.nvidia.com"));
        assert!(allows_unauthenticated_model_fetch("astrai"));
        assert!(allows_unauthenticated_model_fetch("ollama"));
        assert!(allows_unauthenticated_model_fetch("llamacpp"));
        assert!(allows_unauthenticated_model_fetch("llama.cpp"));
        assert!(allows_unauthenticated_model_fetch("sglang"));
        assert!(allows_unauthenticated_model_fetch("vllm"));
        assert!(!allows_unauthenticated_model_fetch("openai"));
        assert!(!allows_unauthenticated_model_fetch("deepseek"));
    }

    #[test]
    fn curated_models_for_kimi_code_include_official_agent_model() {
        let ids: Vec<String> = curated_models_for_provider("kimi-code")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"kimi-for-coding".to_string()));
        assert!(ids.contains(&"kimi-k2.5".to_string()));
    }

    #[test]
    fn curated_models_for_qwen_code_include_coding_plan_models() {
        let ids: Vec<String> = curated_models_for_provider("qwen-code")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"qwen3-coder-plus".to_string()));
        assert!(ids.contains(&"qwen3.5-plus".to_string()));
        assert!(ids.contains(&"qwen3-max-2026-01-23".to_string()));
    }

    #[test]
    fn supports_live_model_fetch_for_supported_and_unsupported_providers() {
        assert!(supports_live_model_fetch("openai"));
        assert!(supports_live_model_fetch("anthropic"));
        assert!(supports_live_model_fetch("gemini"));
        assert!(supports_live_model_fetch("google"));
        assert!(supports_live_model_fetch("grok"));
        assert!(supports_live_model_fetch("together"));
        assert!(supports_live_model_fetch("nvidia"));
        assert!(supports_live_model_fetch("nvidia-nim"));
        assert!(supports_live_model_fetch("build.nvidia.com"));
        assert!(supports_live_model_fetch("ollama"));
        assert!(supports_live_model_fetch("llamacpp"));
        assert!(supports_live_model_fetch("llama.cpp"));
        assert!(supports_live_model_fetch("sglang"));
        assert!(supports_live_model_fetch("vllm"));
        assert!(supports_live_model_fetch("astrai"));
        assert!(supports_live_model_fetch("venice"));
        assert!(supports_live_model_fetch("glm-cn"));
        assert!(supports_live_model_fetch("qwen-intl"));
        assert!(!supports_live_model_fetch("openai-codex"));
        assert!(!supports_live_model_fetch("minimax-cn"));
        assert!(!supports_live_model_fetch("unknown-provider"));
    }

    #[test]
    fn curated_models_provider_aliases_share_same_catalog() {
        assert_eq!(
            curated_models_for_provider("xai"),
            curated_models_for_provider("grok")
        );
        assert_eq!(
            curated_models_for_provider("together-ai"),
            curated_models_for_provider("together")
        );
        assert_eq!(
            curated_models_for_provider("gemini"),
            curated_models_for_provider("google")
        );
        assert_eq!(
            curated_models_for_provider("gemini"),
            curated_models_for_provider("google-gemini")
        );
        assert_eq!(
            curated_models_for_provider("qwen"),
            curated_models_for_provider("qwen-intl")
        );
        assert_eq!(
            curated_models_for_provider("qwen"),
            curated_models_for_provider("dashscope-us")
        );
        assert_eq!(
            curated_models_for_provider("minimax"),
            curated_models_for_provider("minimax-cn")
        );
        assert_eq!(
            curated_models_for_provider("zai"),
            curated_models_for_provider("zai-cn")
        );
        assert_eq!(
            curated_models_for_provider("nvidia"),
            curated_models_for_provider("nvidia-nim")
        );
        assert_eq!(
            curated_models_for_provider("nvidia"),
            curated_models_for_provider("build.nvidia.com")
        );
        assert_eq!(
            curated_models_for_provider("llamacpp"),
            curated_models_for_provider("llama.cpp")
        );
        assert_eq!(
            curated_models_for_provider("bedrock"),
            curated_models_for_provider("aws-bedrock")
        );
    }

    #[test]
    fn curated_models_for_nvidia_include_nim_catalog_entries() {
        let ids: Vec<String> = curated_models_for_provider("nvidia")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert!(ids.contains(&"meta/llama-3.3-70b-instruct".to_string()));
        assert!(ids.contains(&"deepseek-ai/deepseek-v3.2".to_string()));
        assert!(ids.contains(&"nvidia/llama-3.3-nemotron-super-49b-v1.5".to_string()));
    }

    #[test]
    fn models_endpoint_for_provider_handles_region_aliases() {
        assert_eq!(
            models_endpoint_for_provider("glm-cn"),
            Some("https://open.bigmodel.cn/api/paas/v4/models")
        );
        assert_eq!(
            models_endpoint_for_provider("zai-cn"),
            Some("https://open.bigmodel.cn/api/coding/paas/v4/models")
        );
        assert_eq!(
            models_endpoint_for_provider("qwen-intl"),
            Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models")
        );
    }

    #[test]
    fn models_endpoint_for_provider_supports_additional_openai_compatible_providers() {
        assert_eq!(
            models_endpoint_for_provider("openai-codex"),
            Some("https://api.openai.com/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("venice"),
            Some("https://api.venice.ai/api/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("cohere"),
            Some("https://api.cohere.com/compatibility/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("moonshot"),
            Some("https://api.moonshot.ai/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("llamacpp"),
            Some("http://localhost:8080/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("llama.cpp"),
            Some("http://localhost:8080/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("sglang"),
            Some("http://localhost:30000/v1/models")
        );
        assert_eq!(
            models_endpoint_for_provider("vllm"),
            Some("http://localhost:8000/v1/models")
        );
        assert_eq!(models_endpoint_for_provider("perplexity"), None);
        assert_eq!(models_endpoint_for_provider("unknown-provider"), None);
    }

    #[test]
    fn resolve_live_models_endpoint_prefers_llamacpp_custom_url() {
        assert_eq!(
            resolve_live_models_endpoint("llamacpp", Some("http://127.0.0.1:8033/v1")),
            Some("http://127.0.0.1:8033/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("llama.cpp", Some("http://127.0.0.1:8033/v1/")),
            Some("http://127.0.0.1:8033/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("llamacpp", Some("http://127.0.0.1:8033/v1/models")),
            Some("http://127.0.0.1:8033/v1/models".to_string())
        );
    }

    #[test]
    fn resolve_live_models_endpoint_falls_back_to_provider_defaults() {
        assert_eq!(
            resolve_live_models_endpoint("llamacpp", None),
            Some("http://localhost:8080/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("sglang", None),
            Some("http://localhost:30000/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("vllm", None),
            Some("http://localhost:8000/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("venice", Some("http://localhost:9999/v1")),
            Some("https://api.venice.ai/api/v1/models".to_string())
        );
        assert_eq!(resolve_live_models_endpoint("unknown-provider", None), None);
    }

    #[test]
    fn resolve_live_models_endpoint_supports_custom_provider_urls() {
        assert_eq!(
            resolve_live_models_endpoint("custom:https://proxy.example.com/v1", None),
            Some("https://proxy.example.com/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("custom:https://proxy.example.com/v1/models", None),
            Some("https://proxy.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn normalize_ollama_endpoint_url_strips_api_suffix_and_trailing_slash() {
        assert_eq!(
            normalize_ollama_endpoint_url(" https://ollama.com/api/ "),
            "https://ollama.com".to_string()
        );
        assert_eq!(
            normalize_ollama_endpoint_url("https://ollama.com/"),
            "https://ollama.com".to_string()
        );
        assert_eq!(normalize_ollama_endpoint_url(""), "");
    }

    #[test]
    fn ollama_uses_remote_endpoint_distinguishes_local_and_remote_urls() {
        assert!(!ollama_uses_remote_endpoint(None));
        assert!(!ollama_uses_remote_endpoint(Some("http://localhost:11434")));
        assert!(!ollama_uses_remote_endpoint(Some(
            "http://127.0.0.1:11434/api"
        )));
        assert!(ollama_uses_remote_endpoint(Some("https://ollama.com")));
        assert!(ollama_uses_remote_endpoint(Some("https://ollama.com/api")));
    }

    #[test]
    fn resolve_live_models_endpoint_prefers_vllm_custom_url() {
        assert_eq!(
            resolve_live_models_endpoint("vllm", Some("http://127.0.0.1:9000/v1")),
            Some("http://127.0.0.1:9000/v1/models".to_string())
        );
        assert_eq!(
            resolve_live_models_endpoint("vllm", Some("http://127.0.0.1:9000/v1/models")),
            Some("http://127.0.0.1:9000/v1/models".to_string())
        );
    }

    #[test]
    fn parse_openai_model_ids_supports_data_array_payload() {
        let payload = json!({
            "data": [
                {"id": "  gpt-5.1  "},
                {"id": "gpt-5-mini"},
                {"id": "gpt-5.1"},
                {"id": ""}
            ]
        });

        let ids = parse_openai_compatible_model_ids(&payload);
        assert_eq!(ids, vec!["gpt-5-mini".to_string(), "gpt-5.1".to_string()]);
    }

    #[test]
    fn parse_openai_model_ids_supports_root_array_payload() {
        let payload = json!([
            {"id": "alpha"},
            {"id": "beta"},
            {"id": "alpha"}
        ]);

        let ids = parse_openai_compatible_model_ids(&payload);
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn normalize_model_ids_deduplicates_case_insensitively() {
        let ids = normalize_model_ids(vec![
            "GPT-5".to_string(),
            "gpt-5".to_string(),
            "gpt-5-mini".to_string(),
            " GPT-5-MINI ".to_string(),
        ]);
        assert_eq!(ids, vec!["GPT-5".to_string(), "gpt-5-mini".to_string()]);
    }

    #[test]
    fn parse_gemini_model_ids_filters_for_generate_content() {
        let payload = json!({
            "models": [
                {
                    "name": "models/gemini-2.5-pro",
                    "supportedGenerationMethods": ["generateContent", "countTokens"]
                },
                {
                    "name": "models/text-embedding-004",
                    "supportedGenerationMethods": ["embedContent"]
                },
                {
                    "name": "models/gemini-2.5-flash",
                    "supportedGenerationMethods": ["generateContent"]
                }
            ]
        });

        let ids = parse_gemini_model_ids(&payload);
        assert_eq!(
            ids,
            vec!["gemini-2.5-flash".to_string(), "gemini-2.5-pro".to_string()]
        );
    }

    #[test]
    fn parse_ollama_model_ids_extracts_and_deduplicates_names() {
        let payload = json!({
            "models": [
                {"name": "llama3.2:latest"},
                {"name": "mistral:latest"},
                {"name": "llama3.2:latest"}
            ]
        });

        let ids = parse_ollama_model_ids(&payload);
        assert_eq!(
            ids,
            vec!["llama3.2:latest".to_string(), "mistral:latest".to_string()]
        );
    }

    #[tokio::test]
    async fn model_cache_round_trip_returns_fresh_entry() {
        let tmp = TempDir::new().unwrap();
        let models = vec!["gpt-5.1".to_string(), "gpt-5-mini".to_string()];

        cache_live_models_for_provider(tmp.path(), "openai", &models)
            .await
            .unwrap();

        let cached = load_cached_models_for_provider(tmp.path(), "openai", MODEL_CACHE_TTL_SECS)
            .await
            .unwrap();
        let cached = cached.expect("expected fresh cached models");

        assert_eq!(cached.models.len(), 2);
        assert!(cached.models.contains(&"gpt-5.1".to_string()));
        assert!(cached.models.contains(&"gpt-5-mini".to_string()));
    }

    #[tokio::test]
    async fn model_cache_ttl_filters_stale_entries() {
        let tmp = TempDir::new().unwrap();
        let stale = ModelCacheState {
            entries: vec![ModelCacheEntry {
                provider: "openai".to_string(),
                fetched_at_unix: now_unix_secs().saturating_sub(MODEL_CACHE_TTL_SECS + 120),
                models: vec!["gpt-5.1".to_string()],
            }],
        };

        save_model_cache_state(tmp.path(), &stale).await.unwrap();

        let fresh = load_cached_models_for_provider(tmp.path(), "openai", MODEL_CACHE_TTL_SECS)
            .await
            .unwrap();
        assert!(fresh.is_none());

        let stale_any = load_any_cached_models_for_provider(tmp.path(), "openai")
            .await
            .unwrap();
        assert!(stale_any.is_some());
    }

    #[tokio::test]
    async fn run_models_refresh_uses_fresh_cache_without_network() {
        let tmp = TempDir::new().unwrap();

        cache_live_models_for_provider(tmp.path(), "openai", &["gpt-5.1".to_string()])
            .await
            .unwrap();

        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            default_provider: Some("openai".to_string()),
            ..Config::default()
        };

        run_models_refresh(&config, None, false).await.unwrap();
    }

    #[tokio::test]
    async fn run_models_refresh_rejects_unsupported_provider() {
        let tmp = TempDir::new().unwrap();

        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            // Use a non-provider key to keep this test deterministic and offline.
            default_provider: Some("not-a-real-provider".to_string()),
            ..Config::default()
        };

        let err = run_models_refresh(&config, None, true).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("does not support live model discovery"));
    }

    // ── provider_env_var ────────────────────────────────────────

    #[test]
    fn provider_env_var_known_providers() {
        assert_eq!(provider_env_var("openrouter"), "OPENROUTER_API_KEY");
        assert_eq!(provider_env_var("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(provider_env_var("openai-codex"), "OPENAI_API_KEY");
        assert_eq!(provider_env_var("openai"), "OPENAI_API_KEY");
        assert_eq!(provider_env_var("ollama"), "OLLAMA_API_KEY");
        assert_eq!(provider_env_var("llamacpp"), "LLAMACPP_API_KEY");
        assert_eq!(provider_env_var("llama.cpp"), "LLAMACPP_API_KEY");
        assert_eq!(provider_env_var("sglang"), "SGLANG_API_KEY");
        assert_eq!(provider_env_var("vllm"), "VLLM_API_KEY");
        assert_eq!(provider_env_var("xai"), "XAI_API_KEY");
        assert_eq!(provider_env_var("grok"), "XAI_API_KEY"); // alias
        assert_eq!(provider_env_var("together"), "TOGETHER_API_KEY"); // alias
        assert_eq!(provider_env_var("together-ai"), "TOGETHER_API_KEY");
        assert_eq!(provider_env_var("google"), "GEMINI_API_KEY"); // alias
        assert_eq!(provider_env_var("google-gemini"), "GEMINI_API_KEY"); // alias
        assert_eq!(provider_env_var("gemini"), "GEMINI_API_KEY");
        assert_eq!(provider_env_var("qwen"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("qwen-intl"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("dashscope-us"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("qwen-code"), "QWEN_OAUTH_TOKEN");
        assert_eq!(provider_env_var("qwen-oauth"), "QWEN_OAUTH_TOKEN");
        assert_eq!(provider_env_var("glm-cn"), "GLM_API_KEY");
        assert_eq!(provider_env_var("minimax-cn"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("kimi-code"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("kimi_coding"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("kimi_for_coding"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("minimax-oauth"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("minimax-oauth-cn"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("moonshot-intl"), "MOONSHOT_API_KEY");
        assert_eq!(provider_env_var("zai-cn"), "ZAI_API_KEY");
        assert_eq!(provider_env_var("nvidia"), "NVIDIA_API_KEY");
        assert_eq!(provider_env_var("nvidia-nim"), "NVIDIA_API_KEY"); // alias
        assert_eq!(provider_env_var("build.nvidia.com"), "NVIDIA_API_KEY"); // alias
        assert_eq!(provider_env_var("astrai"), "ASTRAI_API_KEY");
        assert_eq!(provider_env_var("hunyuan"), "HUNYUAN_API_KEY");
        assert_eq!(provider_env_var("tencent"), "HUNYUAN_API_KEY"); // alias
    }

    #[test]
    fn provider_supports_keyless_local_usage_for_local_providers() {
        assert!(provider_supports_keyless_local_usage("ollama"));
        assert!(provider_supports_keyless_local_usage("llamacpp"));
        assert!(provider_supports_keyless_local_usage("llama.cpp"));
        assert!(provider_supports_keyless_local_usage("sglang"));
        assert!(provider_supports_keyless_local_usage("vllm"));
        assert!(!provider_supports_keyless_local_usage("openai"));
    }

    #[test]
    fn provider_next_step_uses_guided_setup_when_provider_is_missing() {
        let mut config = Config::default();
        config.default_provider = None;

        assert_eq!(
            provider_next_step(&config),
            Some(ProviderNextStep::GuidedSetup)
        );
    }

    #[test]
    fn provider_next_step_uses_guided_setup_for_unknown_provider_value() {
        let config = Config {
            default_provider: Some("not-a-real-provider".to_string()),
            ..Config::default()
        };

        assert_eq!(
            provider_next_step(&config),
            Some(ProviderNextStep::GuidedSetup)
        );
    }

    #[test]
    fn provider_next_step_prefers_auth_guidance_for_known_provider() {
        let tmp = TempDir::new().unwrap();
        let config = Config {
            config_path: tmp.path().join("config.toml"),
            default_provider: Some("openai-codex".to_string()),
            ..Config::default()
        };

        assert_eq!(
            provider_next_step(&config),
            Some(ProviderNextStep::OpenAiCodexAuth)
        );
    }

    #[test]
    fn provider_next_step_is_none_for_keyless_local_provider() {
        let config = Config {
            default_provider: Some("ollama".to_string()),
            ..Config::default()
        };

        assert_eq!(provider_next_step(&config), None);
    }

    #[test]
    fn provider_supports_device_flow_copilot() {
        assert!(provider_supports_device_flow("copilot"));
        assert!(provider_supports_device_flow("github-copilot"));
        assert!(provider_supports_device_flow("gemini"));
        assert!(provider_supports_device_flow("openai-codex"));
        assert!(!provider_supports_device_flow("openai"));
        assert!(!provider_supports_device_flow("openrouter"));
    }

    #[test]
    fn default_existing_config_mode_prefers_channel_repair_when_no_channels_exist() {
        assert_eq!(default_existing_config_mode_index(false), 2);
        assert_eq!(default_existing_config_mode_index(true), 1);
    }

    #[test]
    fn provider_uses_oauth_without_api_key_only_for_codex() {
        assert!(provider_uses_oauth_without_api_key("openai-codex"));
        assert!(provider_uses_oauth_without_api_key("codex"));
        assert!(!provider_uses_oauth_without_api_key("openai"));
        assert!(!provider_uses_oauth_without_api_key("gemini"));
    }

    #[test]
    fn local_provider_choices_include_sglang() {
        let choices = local_provider_choices();
        assert!(choices.iter().any(|(provider, _)| *provider == "sglang"));
    }

    #[test]
    fn provider_env_var_unknown_falls_back() {
        assert_eq!(provider_env_var("some-new-provider"), "TOPCLAW_API_KEY");
    }

    #[test]
    fn memory_backend_profile_marks_lucid_as_optional_sqlite_backed() {
        let lucid = memory_backend_profile("lucid");
        assert!(lucid.auto_save_default);
        assert!(lucid.uses_sqlite_hygiene);
        assert!(lucid.sqlite_based);
        assert!(lucid.optional_dependency);

        let markdown = memory_backend_profile("markdown");
        assert!(markdown.auto_save_default);
        assert!(!markdown.uses_sqlite_hygiene);

        let none = memory_backend_profile("none");
        assert!(!none.auto_save_default);
        assert!(!none.uses_sqlite_hygiene);

        let custom = memory_backend_profile("custom-memory");
        assert!(custom.auto_save_default);
        assert!(!custom.uses_sqlite_hygiene);
    }

    #[test]
    fn memory_config_defaults_for_lucid_enable_sqlite_hygiene() {
        let config = memory_config_defaults_for_backend("lucid");
        assert_eq!(config.backend, "lucid");
        assert!(config.auto_save);
        assert!(config.hygiene_enabled);
        assert_eq!(config.archive_after_days, 7);
        assert_eq!(config.purge_after_days, 30);
        assert_eq!(config.embedding_cache_size, 10000);
    }

    #[test]
    fn memory_config_defaults_for_none_disable_sqlite_hygiene() {
        let config = memory_config_defaults_for_backend("none");
        assert_eq!(config.backend, "none");
        assert!(!config.auto_save);
        assert!(!config.hygiene_enabled);
        assert_eq!(config.archive_after_days, 0);
        assert_eq!(config.purge_after_days, 0);
        assert_eq!(config.embedding_cache_size, 0);
    }

    #[test]
    fn top_level_channel_menu_prioritizes_recommended_channels() {
        let labels = channel_menu_option_labels(&ChannelsConfig::default());

        assert!(
            labels
                .first()
                .is_some_and(|label| label.starts_with("Telegram")),
            "expected Telegram to be the first top-level channel option, got {labels:?}"
        );

        #[cfg(feature = "channel-discord")]
        assert!(
            labels
                .get(1)
                .is_some_and(|label| label.starts_with("Discord")),
            "expected Discord to be the second top-level channel option, got {labels:?}"
        );

        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("Other channels")),
            "expected an Other channels entry in the top-level menu, got {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.starts_with("Webhook")),
            "expected Webhook to be hidden behind Other channels, got {labels:?}"
        );
    }

    #[test]
    fn other_channel_menu_lists_advanced_channels() {
        let labels = other_channel_menu_option_labels(&ChannelsConfig::default());

        assert!(
            labels.iter().any(|label| label.starts_with("Webhook")),
            "expected Webhook to appear in the advanced channel menu, got {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.starts_with("Telegram")),
            "expected Telegram to stay in the top-level menu, got {labels:?}"
        );
        #[cfg(feature = "channel-discord")]
        assert!(
            !labels.iter().any(|label| label.starts_with("Discord")),
            "expected Discord to stay in the top-level menu, got {labels:?}"
        );
        assert!(
            labels.last().is_some_and(|label| label.starts_with("Back")),
            "expected the advanced menu to end with a Back option, got {labels:?}"
        );
    }

    #[test]
    fn channel_menu_choices_reuses_one_static_slice() {
        let first = channel_menu_choices();
        let second = channel_menu_choices();

        assert_eq!(first, second);
        assert_eq!(first.as_ptr(), second.as_ptr());
    }

    #[test]
    fn default_channel_menu_prefers_telegram() {
        let default_choice = channel_menu_choices()
            .get(default_channel_menu_index(&ChannelsConfig::default()))
            .copied();
        assert_eq!(default_choice, Some(ChannelMenuChoice::Telegram));
    }

    #[test]
    fn default_channel_menu_prefers_done_after_one_channel_is_configured() {
        let mut channels = ChannelsConfig::default();
        channels.telegram = Some(TelegramConfig {
            bot_token: "test-token".into(),
            allowed_users: vec!["topclaw_user".into()],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });

        let default_choice = channel_menu_choices()
            .get(default_channel_menu_index(&channels))
            .copied();
        assert_eq!(default_choice, Some(ChannelMenuChoice::Done));
    }

    #[test]
    fn lower_risk_builtin_skills_are_selected_by_default() {
        let find_skills = crate::skills::curated_skill_catalog()
            .iter()
            .find(|entry| entry.slug == "find-skills")
            .unwrap();
        let safe_web_search = crate::skills::curated_skill_catalog()
            .iter()
            .find(|entry| entry.slug == "safe-web-search")
            .unwrap();
        let desktop_use = crate::skills::curated_skill_catalog()
            .iter()
            .find(|entry| entry.slug == "desktop-computer-use")
            .unwrap();

        assert!(default_selected_onboarding_skill(find_skills));
        assert!(default_selected_onboarding_skill(safe_web_search));
        assert!(!default_selected_onboarding_skill(desktop_use));
    }

    #[test]
    fn onboarding_skill_labels_are_compact_and_descriptive() {
        let safe_web_search = crate::skills::curated_skill_catalog()
            .iter()
            .find(|entry| entry.slug == "safe-web-search")
            .unwrap();
        let browser_extension = crate::skills::curated_skill_catalog()
            .iter()
            .find(|entry| entry.slug == "agent-browser-extension")
            .unwrap();

        assert_eq!(
            format_onboarding_skill_label(safe_web_search),
            "safe-web-search — search the web"
        );
        assert_eq!(
            format_onboarding_skill_label(browser_extension),
            "agent-browser-extension — browser automation"
        );
    }

    #[test]
    fn onboarding_skill_selection_key_handler_supports_clear_all() {
        let mut checked = vec![true, true, false];
        let mut active = 1usize;

        assert!(!apply_onboarding_skill_selection_key(
            Key::Char('c'),
            &mut checked,
            &mut active
        ));

        assert_eq!(checked, vec![false, false, false]);
        assert_eq!(active, 1);
    }

    #[test]
    fn onboarding_skill_selection_key_handler_keeps_toggle_all_behavior() {
        let mut checked = vec![true, false, false];
        let mut active = 0usize;

        assert!(!apply_onboarding_skill_selection_key(
            Key::Char('a'),
            &mut checked,
            &mut active
        ));
        assert_eq!(checked, vec![true, true, true]);

        assert!(!apply_onboarding_skill_selection_key(
            Key::Char('a'),
            &mut checked,
            &mut active
        ));
        assert_eq!(checked, vec![false, false, false]);
    }

    #[test]
    fn onboarding_skill_selection_enables_required_web_tools() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec![
                "find-skills".into(),
                "safe-web-search".into(),
                "multi-search-engine".into(),
            ],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(config.web_search.enabled);
        assert!(config.web_fetch.enabled);
        assert!(!config.browser.enabled);
    }

    #[test]
    fn onboarding_skill_selection_sets_browser_backend_for_extension_skill() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec!["agent-browser-extension".into()],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(config.browser.enabled);
        assert_eq!(config.browser.backend, "agent_browser");
    }

    #[test]
    fn onboarding_skill_selection_sets_browser_backend_for_desktop_skill() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec!["desktop-computer-use".into()],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(config.browser.enabled);
        assert_eq!(config.browser.backend, "computer_use");
    }

    #[test]
    fn onboarding_skill_selection_uses_auto_backend_when_both_browser_skills_are_selected() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec![
                "agent-browser-extension".into(),
                "desktop-computer-use".into(),
            ],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(config.browser.enabled);
        assert_eq!(config.browser.backend, "auto");
    }

    #[test]
    fn onboarding_skill_selection_keeps_tools_disabled_for_prompt_only_and_local_skills() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec![
                "skill-creator".into(),
                "local-file-analyzer".into(),
                "workspace-search".into(),
                "code-explainer".into(),
                "change-summary".into(),
                "self-improving-agent".into(),
            ],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(!config.web_search.enabled);
        assert!(!config.web_fetch.enabled);
        assert!(!config.browser.enabled);
    }

    #[test]
    fn onboarding_skill_selection_adds_required_shell_commands_to_policy() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec![
                "workspace-search".into(),
                "code-explainer".into(),
                "change-summary".into(),
                "skill-creator".into(),
            ],
        };
        let mut config = Config::default();

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert_eq!(config.autonomy.allowed_commands, vec!["*"]);
        assert_eq!(
            config.autonomy.shell_redirect_policy,
            ShellRedirectPolicy::Allow
        );
        assert!(!config.autonomy.require_approval_for_medium_risk);
        assert!(config.autonomy.auto_approve.contains(&"shell".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"git_operations".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"file_write".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"file_edit".to_string()));
    }

    #[test]
    fn onboarding_skill_selection_preserves_existing_allowed_commands_when_adding_defaults() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec!["workspace-search".into(), "change-summary".into()],
        };
        let mut config = Config::default();
        config.autonomy.allowed_commands = vec!["custom-check".into()];

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert_eq!(config.autonomy.allowed_commands, vec!["custom-check", "*"]);
    }

    #[test]
    fn onboarding_skill_selection_auto_approves_selected_tools_and_clears_always_ask() {
        let selection = SkillOnboardingSelection {
            selected_curated_slugs: vec![
                "self-improving-agent".into(),
                "multi-search-engine".into(),
                "desktop-computer-use".into(),
            ],
        };
        let mut config = Config::default();
        config.autonomy.always_ask = vec![
            "memory_store".into(),
            "http_request".into(),
            "browser".into(),
            "screenshot".into(),
            "shell".into(),
        ];

        apply_onboarding_skill_tool_defaults(&mut config, &selection);

        assert!(config
            .autonomy
            .auto_approve
            .contains(&"memory_store".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"http_request".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"browser".to_string()));
        assert!(config
            .autonomy
            .auto_approve
            .contains(&"screenshot".to_string()));
        assert!(!config
            .autonomy
            .always_ask
            .contains(&"memory_store".to_string()));
        assert!(!config
            .autonomy
            .always_ask
            .contains(&"http_request".to_string()));
        assert!(!config.autonomy.always_ask.contains(&"browser".to_string()));
        assert!(!config
            .autonomy
            .always_ask
            .contains(&"screenshot".to_string()));
        assert!(config.autonomy.always_ask.contains(&"shell".to_string()));
    }

    #[test]
    fn launchable_channels_detects_configured_channels() {
        let mut channels = ChannelsConfig::default();
        assert!(!has_launchable_channels(&channels));

        channels.telegram = Some(TelegramConfig {
            bot_token: "test-token".into(),
            allowed_users: vec!["topclaw_user".into()],
            stream_mode: StreamMode::Off,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: false,
            group_reply: None,
            base_url: None,
        });
        assert!(has_launchable_channels(&channels));
    }
}
