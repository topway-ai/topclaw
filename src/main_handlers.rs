use crate::{
    AuthCommands, Cli, CompletionShell, EstopLevelArg, EstopSubcommands,
    ServiceCommands, WorkspaceCommands, WorkspaceTokenCommands,
};
use anyhow::{bail, Context, Result};
use dialoguer::{Input, Password};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use topclaw::{auth, security, service, workspace, Config};
use tracing::warn;

pub(crate) fn handle_uninstall_command(purge: bool) -> Result<()> {
    let config_dir = topclaw::config::default_config_dir()?;
    let config_path = config_dir.join("config.toml");
    let workspace_dir = config_dir.join("workspace");

    let mut config = Config::default();
    config.config_path = config_path;
    config.workspace_dir = workspace_dir;

    let init_system = service::InitSystem::Auto;
    let _ = service::handle_command(&ServiceCommands::Stop, &config, init_system);
    let _ = service::handle_command(&ServiceCommands::Uninstall, &config, init_system);

    let current_exe = std::env::current_exe().ok();
    if let Some(path) = current_exe.as_ref() {
        remove_topclaw_binary(path)?;
    }

    // The .cargo/bin path must always use the actual home directory,
    // regardless of where the config directory lives (e.g. if
    // TOPCLAW_CONFIG_DIR=/opt/topclaw, parent would be /opt, not ~).
    let home_dir = directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    let cargo_bin = home_dir
        .join(".cargo")
        .join("bin")
        .join(topclaw_binary_name());
    if current_exe.as_ref() != Some(&cargo_bin) && cargo_bin.exists() {
        remove_topclaw_binary(&cargo_bin)?;
    }

    if purge && config_dir.exists() {
        fs::remove_dir_all(&config_dir).with_context(|| {
            format!(
                "Failed to remove TopClaw data directory {}",
                config_dir.display()
            )
        })?;
        println!("✅ Removed {}", config_dir.display());
    }

    println!("✅ TopClaw uninstall complete");
    if !purge {
        println!("Run `topclaw uninstall --purge` to remove ~/.topclaw data too.");
    }
    Ok(())
}

fn remove_topclaw_binary(path: &std::path::Path) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name != "topclaw" && file_name != "topclaw.exe" {
        bail!(
            "Refusing to remove unexpected executable path: {}",
            path.display()
        );
    }

    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("Failed to remove TopClaw binary {}", path.display()))?;
        println!("✅ Removed {}", path.display());
    }

    Ok(())
}

fn topclaw_binary_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "topclaw.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "topclaw"
    }
}

pub(crate) fn handle_workspace_command(
    workspace_command: WorkspaceCommands,
    config: &Config,
) -> Result<()> {
    if !config.workspaces.enabled {
        bail!(
            "workspace registry is disabled. Enable [workspaces].enabled = true in {}",
            config.config_path.display()
        );
    }

    let root = workspace::registry_root_from_config(config)?;
    let mut registry = workspace::WorkspaceRegistry::load(&root)?;

    match workspace_command {
        WorkspaceCommands::Create { name } => {
            let display_name = name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("workspace-{}", uuid::Uuid::new_v4()));
            let (workspace_id, token) = registry.create(&display_name)?;
            println!("Workspace created.");
            println!("  id: {workspace_id}");
            println!("  display_name: {display_name}");
            println!("  root: {}", registry.root().display());
            println!("  token: {token}");
            println!("Store this token securely; it is shown only once.");
            Ok(())
        }
        WorkspaceCommands::List => {
            let workspaces = registry.list();
            if workspaces.is_empty() {
                println!("No workspaces found under {}", registry.root().display());
                return Ok(());
            }

            println!("Workspace root: {}", registry.root().display());
            println!(
                "{:<36}  {:<7}  {:<20}  DISPLAY_NAME",
                "ID", "ENABLED", "CREATED_AT"
            );
            for workspace in workspaces {
                println!(
                    "{:<36}  {:<7}  {:<20}  {}",
                    workspace.id, workspace.enabled, workspace.created_at, workspace.display_name
                );
            }
            Ok(())
        }
        WorkspaceCommands::Disable { workspace_id } => {
            registry.disable(&workspace_id)?;
            println!("Workspace disabled: {workspace_id}");
            Ok(())
        }
        WorkspaceCommands::Delete {
            workspace_id,
            confirm,
        } => {
            if !confirm {
                bail!(
                    "workspace delete is destructive. Re-run with --confirm to delete {}",
                    workspace_id
                );
            }
            registry.delete(&workspace_id)?;
            println!("Workspace deleted: {workspace_id}");
            Ok(())
        }
        WorkspaceCommands::Token { token_command } => match token_command {
            WorkspaceTokenCommands::Rotate { workspace_id } => {
                let token = registry.rotate_token(&workspace_id)?;
                println!("Workspace token rotated.");
                println!("  id: {workspace_id}");
                println!("  token: {token}");
                println!("Store this token securely; it is shown only once.");
                Ok(())
            }
        },
    }
}

pub(crate) fn handle_estop_command(
    config: &Config,
    estop_command: Option<EstopSubcommands>,
    level: Option<EstopLevelArg>,
    domains: Vec<String>,
    tools: Vec<String>,
) -> Result<()> {
    if !config.security.estop.enabled {
        bail!("Emergency stop is disabled. Enable [security.estop].enabled = true in config.toml");
    }

    let config_dir = config
        .config_path
        .parent()
        .context("Config path must have a parent directory")?;
    let mut manager = security::EstopManager::load(&config.security.estop, config_dir)?;

    match estop_command {
        Some(EstopSubcommands::Status) => {
            print_estop_status(&manager.status());
            Ok(())
        }
        Some(EstopSubcommands::Resume {
            network,
            domains,
            tools,
            otp,
        }) => {
            let selector = build_resume_selector(network, domains, tools)?;
            let mut otp_code = otp;
            let otp_validator = if config.security.estop.require_otp_to_resume {
                if !config.security.otp.enabled {
                    bail!(
                        "security.estop.require_otp_to_resume=true but security.otp.enabled=false"
                    );
                }
                if otp_code.is_none() {
                    let entered = Password::new()
                        .with_prompt("Enter OTP code")
                        .allow_empty_password(false)
                        .interact()?;
                    otp_code = Some(entered);
                }

                let store = security::SecretStore::new(config_dir, config.secrets.encrypt);
                let (validator, enrollment_uri) =
                    security::OtpValidator::from_config(&config.security.otp, config_dir, &store)?;
                if let Some(uri) = enrollment_uri {
                    println!("Initialized OTP secret for TopClaw.");
                    println!("Enrollment URI: {uri}");
                }
                Some(validator)
            } else {
                None
            };

            manager.resume(selector, otp_code.as_deref(), otp_validator.as_ref())?;
            println!("Estop resume completed.");
            print_estop_status(&manager.status());
            Ok(())
        }
        None => {
            let engage_level = build_engage_level(level, domains, tools)?;
            manager.engage(engage_level)?;
            println!("Estop engaged.");
            print_estop_status(&manager.status());
            Ok(())
        }
    }
}

fn build_engage_level(
    level: Option<EstopLevelArg>,
    domains: Vec<String>,
    tools: Vec<String>,
) -> Result<security::EstopLevel> {
    let requested = level.unwrap_or(EstopLevelArg::KillAll);
    match requested {
        EstopLevelArg::KillAll => {
            if !domains.is_empty() || !tools.is_empty() {
                bail!("--domain/--tool are only valid with --level domain-block/tool-freeze");
            }
            Ok(security::EstopLevel::KillAll)
        }
        EstopLevelArg::NetworkKill => {
            if !domains.is_empty() || !tools.is_empty() {
                bail!("--domain/--tool are not valid with --level network-kill");
            }
            Ok(security::EstopLevel::NetworkKill)
        }
        EstopLevelArg::DomainBlock => {
            if domains.is_empty() {
                bail!("--level domain-block requires at least one --domain");
            }
            if !tools.is_empty() {
                bail!("--tool is not valid with --level domain-block");
            }
            Ok(security::EstopLevel::DomainBlock(domains))
        }
        EstopLevelArg::ToolFreeze => {
            if tools.is_empty() {
                bail!("--level tool-freeze requires at least one --tool");
            }
            if !domains.is_empty() {
                bail!("--domain is not valid with --level tool-freeze");
            }
            Ok(security::EstopLevel::ToolFreeze(tools))
        }
    }
}

fn build_resume_selector(
    network: bool,
    domains: Vec<String>,
    tools: Vec<String>,
) -> Result<security::ResumeSelector> {
    let selected =
        usize::from(network) + usize::from(!domains.is_empty()) + usize::from(!tools.is_empty());
    if selected > 1 {
        bail!("Use only one of --network, --domain, or --tool for estop resume");
    }
    if network {
        return Ok(security::ResumeSelector::Network);
    }
    if !domains.is_empty() {
        return Ok(security::ResumeSelector::Domains(domains));
    }
    if !tools.is_empty() {
        return Ok(security::ResumeSelector::Tools(tools));
    }
    Ok(security::ResumeSelector::KillAll)
}

fn print_estop_status(state: &security::EstopState) {
    println!("Estop status:");
    println!(
        "  engaged:        {}",
        if state.is_engaged() { "yes" } else { "no" }
    );
    println!(
        "  kill_all:       {}",
        if state.kill_all { "active" } else { "inactive" }
    );
    println!(
        "  network_kill:   {}",
        if state.network_kill {
            "active"
        } else {
            "inactive"
        }
    );
    if state.blocked_domains.is_empty() {
        println!("  domain_blocks:  (none)");
    } else {
        println!("  domain_blocks:  {}", state.blocked_domains.join(", "));
    }
    if state.frozen_tools.is_empty() {
        println!("  tool_freeze:    (none)");
    } else {
        println!("  tool_freeze:    {}", state.frozen_tools.join(", "));
    }
    if let Some(updated_at) = &state.updated_at {
        println!("  updated_at:     {updated_at}");
    }
}

pub(crate) fn write_shell_completion<W: Write>(
    shell: CompletionShell,
    writer: &mut W,
) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::generate;
    use clap_complete::shells;

    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();

    match shell {
        CompletionShell::Bash => generate(shells::Bash, &mut cmd, bin_name.clone(), writer),
        CompletionShell::Fish => generate(shells::Fish, &mut cmd, bin_name.clone(), writer),
        CompletionShell::Zsh => generate(shells::Zsh, &mut cmd, bin_name.clone(), writer),
        CompletionShell::PowerShell => {
            generate(shells::PowerShell, &mut cmd, bin_name.clone(), writer);
        }
        CompletionShell::Elvish => generate(shells::Elvish, &mut cmd, bin_name, writer),
    }

    writer.flush()?;
    Ok(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthLogin {
    provider: String,
    profile: String,
    code_verifier: String,
    state: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingOAuthLoginFile {
    #[serde(default)]
    provider: Option<String>,
    profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_verifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encrypted_code_verifier: Option<String>,
    state: String,
    created_at: String,
}

fn pending_oauth_login_path(config: &Config, provider: &str) -> std::path::PathBuf {
    let filename = format!("auth-{}-pending.json", provider);
    auth::state_dir_from_config(config).join(filename)
}

fn pending_oauth_secret_store(config: &Config) -> security::secrets::SecretStore {
    security::secrets::SecretStore::new(
        &auth::state_dir_from_config(config),
        config.secrets.encrypt,
    )
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

fn save_pending_oauth_login(config: &Config, pending: &PendingOAuthLogin) -> Result<()> {
    let path = pending_oauth_login_path(config, &pending.provider);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let secret_store = pending_oauth_secret_store(config);
    let encrypted_code_verifier = secret_store.encrypt(&pending.code_verifier)?;
    let persisted = PendingOAuthLoginFile {
        provider: Some(pending.provider.clone()),
        profile: pending.profile.clone(),
        code_verifier: None,
        encrypted_code_verifier: Some(encrypted_code_verifier),
        state: pending.state.clone(),
        created_at: pending.created_at.clone(),
    };
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let json = serde_json::to_vec_pretty(&persisted)?;
    std::fs::write(&tmp, json)?;
    set_owner_only_permissions(&tmp)?;
    std::fs::rename(tmp, &path)?;
    set_owner_only_permissions(&path)?;
    Ok(())
}

fn load_pending_oauth_login(config: &Config, provider: &str) -> Result<Option<PendingOAuthLogin>> {
    let path = pending_oauth_login_path(config, provider);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let persisted: PendingOAuthLoginFile = serde_json::from_slice(&bytes)?;
    let secret_store = pending_oauth_secret_store(config);
    let code_verifier = if let Some(encrypted) = persisted.encrypted_code_verifier {
        secret_store.decrypt(&encrypted)?
    } else if let Some(plaintext) = persisted.code_verifier {
        plaintext
    } else {
        bail!("Pending {} login is missing code verifier", provider);
    };
    Ok(Some(PendingOAuthLogin {
        provider: persisted.provider.unwrap_or_else(|| provider.to_string()),
        profile: persisted.profile,
        code_verifier,
        state: persisted.state,
        created_at: persisted.created_at,
    }))
}

fn clear_pending_oauth_login(config: &Config, provider: &str) {
    let path = pending_oauth_login_path(config, provider);
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path) {
        let _ = file.set_len(0);
        let _ = file.sync_all();
    }
    let _ = std::fs::remove_file(path);
}

fn read_auth_input(prompt: &str) -> Result<String> {
    let input = Password::new()
        .with_prompt(prompt)
        .allow_empty_password(false)
        .interact()?;
    Ok(input.trim().to_string())
}

fn read_plain_input(prompt: &str) -> Result<String> {
    let input: String = Input::new().with_prompt(prompt).interact_text()?;
    Ok(input.trim().to_string())
}

fn extract_openai_account_id_for_profile(access_token: &str) -> Option<String> {
    let account_id = auth::openai_oauth::extract_account_id_from_jwt(access_token);
    if account_id.is_none() {
        warn!(
            "Could not extract OpenAI account id from OAuth access token; \
             requests may fail until re-authentication."
        );
    }
    account_id
}

fn format_expiry(profile: &auth::profiles::AuthProfile) -> String {
    match profile
        .token_set
        .as_ref()
        .and_then(|token_set| token_set.expires_at)
    {
        Some(ts) => {
            let now = chrono::Utc::now();
            if ts <= now {
                format!("expired at {}", ts.to_rfc3339())
            } else {
                let mins = (ts - now).num_minutes();
                format!("expires in {mins}m ({})", ts.to_rfc3339())
            }
        }
        None => "n/a".to_string(),
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn handle_auth_command(auth_command: AuthCommands, config: &Config) -> Result<()> {
    let auth_service = auth::AuthService::from_config(config);

    match auth_command {
        AuthCommands::Login {
            provider,
            profile,
            device_code,
        } => {
            let provider = auth::normalize_provider(&provider)?;
            let client = reqwest::Client::new();

            match provider.as_str() {
                "gemini" => {
                    if device_code {
                        match auth::gemini_oauth::start_device_code_flow(&client).await {
                            Ok(device) => {
                                println!("Google/Gemini device-code login started.");
                                println!("Visit: {}", device.verification_uri);
                                println!("Code:  {}", device.user_code);
                                if let Some(uri_complete) = &device.verification_uri_complete {
                                    println!("Fast link: {uri_complete}");
                                }

                                let token_set =
                                    auth::gemini_oauth::poll_device_code_tokens(&client, &device)
                                        .await?;
                                let account_id = token_set.id_token.as_deref().and_then(
                                    auth::gemini_oauth::extract_account_email_from_id_token,
                                );

                                auth_service
                                    .store_gemini_tokens(&profile, token_set, account_id, true)
                                    .await?;

                                println!("Saved profile {profile}");
                                println!("Active profile for gemini: {profile}");
                                return Ok(());
                            }
                            Err(e) => {
                                println!(
                                    "Device-code flow unavailable: {e}. Falling back to browser flow."
                                );
                            }
                        }
                    }

                    let pkce = auth::gemini_oauth::generate_pkce_state();
                    let authorize_url = auth::gemini_oauth::build_authorize_url(&pkce)?;
                    let pending = PendingOAuthLogin {
                        provider: "gemini".to_string(),
                        profile: profile.clone(),
                        code_verifier: pkce.code_verifier.clone(),
                        state: pkce.state.clone(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    save_pending_oauth_login(config, &pending)?;

                    println!("Open this URL in your browser and authorize access:");
                    println!("{authorize_url}");
                    println!();

                    let code = match auth::gemini_oauth::receive_loopback_code(
                        &pkce.state,
                        std::time::Duration::from_secs(180),
                    )
                    .await
                    {
                        Ok(code) => {
                            clear_pending_oauth_login(config, "gemini");
                            code
                        }
                        Err(e) => {
                            println!("Callback capture failed: {e}");
                            println!(
                                "Run `topclaw auth paste-redirect --provider gemini --profile {profile}`"
                            );
                            return Ok(());
                        }
                    };

                    let token_set =
                        auth::gemini_oauth::exchange_code_for_tokens(&client, &code, &pkce).await?;
                    let account_id = token_set
                        .id_token
                        .as_deref()
                        .and_then(auth::gemini_oauth::extract_account_email_from_id_token);

                    auth_service
                        .store_gemini_tokens(&profile, token_set, account_id, true)
                        .await?;

                    println!("Saved profile {profile}");
                    println!("Active profile for gemini: {profile}");
                    Ok(())
                }
                "openai-codex" => {
                    if device_code {
                        match auth::openai_oauth::start_device_code_flow(&client).await {
                            Ok(device) => {
                                println!("OpenAI device-code login started.");
                                println!("Visit: {}", device.verification_uri);
                                println!("Code:  {}", device.user_code);
                                if let Some(uri_complete) = &device.verification_uri_complete {
                                    println!("Fast link: {uri_complete}");
                                }
                                if let Some(message) = &device.message {
                                    println!("{message}");
                                }

                                let token_set =
                                    auth::openai_oauth::poll_device_code_tokens(&client, &device)
                                        .await?;
                                let account_id =
                                    extract_openai_account_id_for_profile(&token_set.access_token);

                                auth_service
                                    .store_openai_tokens(&profile, token_set, account_id, true)
                                    .await?;
                                clear_pending_oauth_login(config, "openai");

                                println!("Saved profile {profile}");
                                println!("Active profile for openai-codex: {profile}");
                                return Ok(());
                            }
                            Err(e) => {
                                println!(
                                    "Device-code flow unavailable: {e}. Falling back to browser flow."
                                );
                            }
                        }
                    }

                    let pkce = auth::openai_oauth::generate_pkce_state();
                    let pending = PendingOAuthLogin {
                        provider: "openai".to_string(),
                        profile: profile.clone(),
                        code_verifier: pkce.code_verifier.clone(),
                        state: pkce.state.clone(),
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    save_pending_oauth_login(config, &pending)?;

                    let authorize_url = auth::openai_oauth::build_authorize_url(&pkce);
                    println!("Open this URL in your browser and authorize access:");
                    println!("{authorize_url}");
                    println!();
                    println!("Waiting for callback at http://localhost:1455/auth/callback ...");

                    let code = match auth::openai_oauth::receive_loopback_code(
                        &pkce.state,
                        std::time::Duration::from_secs(180),
                    )
                    .await
                    {
                        Ok(code) => code,
                        Err(e) => {
                            println!("Callback capture failed: {e}");
                            println!(
                                "Run `topclaw auth paste-redirect --provider openai-codex --profile {profile}`"
                            );
                            return Ok(());
                        }
                    };

                    let token_set =
                        auth::openai_oauth::exchange_code_for_tokens(&client, &code, &pkce).await?;
                    let account_id = extract_openai_account_id_for_profile(&token_set.access_token);

                    auth_service
                        .store_openai_tokens(&profile, token_set, account_id, true)
                        .await?;
                    clear_pending_oauth_login(config, "openai");

                    println!("Saved profile {profile}");
                    println!("Active profile for openai-codex: {profile}");
                    Ok(())
                }
                _ => {
                    bail!(
                        "`auth login` supports --provider openai-codex or gemini, got: {provider}"
                    );
                }
            }
        }

        AuthCommands::PasteRedirect {
            provider,
            profile,
            input,
        } => {
            let provider = auth::normalize_provider(&provider)?;

            match provider.as_str() {
                "openai-codex" => {
                    let pending = load_pending_oauth_login(config, "openai")?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "No pending OpenAI login found. Run `topclaw auth login --provider openai-codex` first."
                        )
                    })?;

                    if pending.profile != profile {
                        bail!(
                            "Pending login profile mismatch: pending={}, requested={}",
                            pending.profile,
                            profile
                        );
                    }

                    let redirect_input = match input {
                        Some(value) => value,
                        None => read_plain_input("Paste redirect URL or OAuth code")?,
                    };

                    let code = auth::openai_oauth::parse_code_from_redirect(
                        &redirect_input,
                        Some(&pending.state),
                    )?;

                    let pkce = auth::openai_oauth::PkceState {
                        code_verifier: pending.code_verifier.clone(),
                        code_challenge: String::new(),
                        state: pending.state.clone(),
                    };

                    let client = reqwest::Client::new();
                    let token_set =
                        auth::openai_oauth::exchange_code_for_tokens(&client, &code, &pkce).await?;
                    let account_id = extract_openai_account_id_for_profile(&token_set.access_token);

                    auth_service
                        .store_openai_tokens(&profile, token_set, account_id, true)
                        .await?;
                    clear_pending_oauth_login(config, "openai");

                    println!("Saved profile {profile}");
                    println!("Active profile for openai-codex: {profile}");
                }
                "gemini" => {
                    let pending = load_pending_oauth_login(config, "gemini")?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "No pending Gemini login found. Run `topclaw auth login --provider gemini` first."
                        )
                    })?;

                    if pending.profile != profile {
                        bail!(
                            "Pending login profile mismatch: pending={}, requested={}",
                            pending.profile,
                            profile
                        );
                    }

                    let redirect_input = match input {
                        Some(value) => value,
                        None => read_plain_input("Paste redirect URL or OAuth code")?,
                    };

                    let code = auth::gemini_oauth::parse_code_from_redirect(
                        &redirect_input,
                        Some(&pending.state),
                    )?;

                    let pkce = auth::gemini_oauth::PkceState {
                        code_verifier: pending.code_verifier.clone(),
                        code_challenge: String::new(),
                        state: pending.state.clone(),
                    };

                    let client = reqwest::Client::new();
                    let token_set =
                        auth::gemini_oauth::exchange_code_for_tokens(&client, &code, &pkce).await?;
                    let account_id = token_set
                        .id_token
                        .as_deref()
                        .and_then(auth::gemini_oauth::extract_account_email_from_id_token);

                    auth_service
                        .store_gemini_tokens(&profile, token_set, account_id, true)
                        .await?;
                    clear_pending_oauth_login(config, "gemini");

                    println!("Saved profile {profile}");
                    println!("Active profile for gemini: {profile}");
                }
                _ => {
                    bail!("`auth paste-redirect` supports --provider openai-codex or gemini");
                }
            }
            Ok(())
        }

        AuthCommands::PasteToken {
            provider,
            profile,
            token,
            auth_kind,
        } => {
            let provider = auth::normalize_provider(&provider)?;
            let token = match token {
                Some(token) => token.trim().to_string(),
                None => read_auth_input("Paste token")?,
            };
            if token.is_empty() {
                bail!("Token cannot be empty");
            }

            let kind = auth::anthropic_token::detect_auth_kind(&token, auth_kind.as_deref());
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "auth_kind".to_string(),
                kind.as_metadata_value().to_string(),
            );

            auth_service
                .store_provider_token(&provider, &profile, &token, metadata, true)
                .await?;
            println!("Saved profile {profile}");
            println!("Active profile for {provider}: {profile}");
            Ok(())
        }

        AuthCommands::Refresh { provider, profile } => {
            let provider = auth::normalize_provider(&provider)?;

            match provider.as_str() {
                "openai-codex" => {
                    match auth_service
                        .get_valid_openai_access_token(profile.as_deref())
                        .await?
                    {
                        Some(_) => {
                            println!("OpenAI Codex token is valid (refresh completed if needed).");
                            Ok(())
                        }
                        None => {
                            bail!(
                                "No OpenAI Codex auth profile found. Run `topclaw auth login --provider openai-codex`."
                            )
                        }
                    }
                }
                "gemini" => match auth_service
                    .get_valid_gemini_access_token(profile.as_deref())
                    .await?
                {
                    Some(_) => {
                        let profile_name = profile.as_deref().unwrap_or("default");
                        println!("✓ Gemini token refreshed successfully");
                        println!("  Profile: gemini:{}", profile_name);
                        Ok(())
                    }
                    None => {
                        bail!(
                            "No Gemini auth profile found. Run `topclaw auth login --provider gemini`."
                        )
                    }
                },
                _ => bail!("`auth refresh` supports --provider openai-codex or gemini"),
            }
        }

        AuthCommands::Logout { provider, profile } => {
            let provider = auth::normalize_provider(&provider)?;
            let removed = auth_service.remove_profile(&provider, &profile).await?;
            if removed {
                println!("Removed auth profile {provider}:{profile}");
            } else {
                println!("Auth profile not found: {provider}:{profile}");
            }
            Ok(())
        }

        AuthCommands::Use { provider, profile } => {
            let provider = auth::normalize_provider(&provider)?;
            auth_service.set_active_profile(&provider, &profile).await?;
            println!("Active profile for {provider}: {profile}");
            Ok(())
        }

        AuthCommands::List => {
            let data = auth_service.load_profiles().await?;
            if data.profiles.is_empty() {
                println!("No auth profiles configured.");
                return Ok(());
            }

            for (id, profile) in &data.profiles {
                let active = data
                    .active_profiles
                    .get(&profile.provider)
                    .is_some_and(|active_id| active_id == id);
                let marker = if active { "*" } else { " " };
                println!("{marker} {id}");
            }

            Ok(())
        }

        AuthCommands::Status => {
            let data = auth_service.load_profiles().await?;
            if data.profiles.is_empty() {
                println!("No auth profiles configured.");
                return Ok(());
            }

            for (id, profile) in &data.profiles {
                let active = data
                    .active_profiles
                    .get(&profile.provider)
                    .is_some_and(|active_id| active_id == id);
                let marker = if active { "*" } else { " " };
                println!(
                    "{} {} kind={:?} account={} expires={}",
                    marker,
                    id,
                    profile.kind,
                    security::redact(profile.account_id.as_deref().unwrap_or("unknown")),
                    format_expiry(profile)
                );
            }

            println!();
            println!("Active profiles:");
            for (provider, profile_id) in &data.active_profiles {
                println!("  {provider}: {profile_id}");
            }

            Ok(())
        }
    }
}
