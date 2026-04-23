use super::runtime_commands::{approval_target_label, non_cli_natural_language_mode_label};
use super::ChannelRuntimeContext;
use crate::config::{Config, NonCliNaturalLanguageApprovalMode};
use crate::providers::{self, Provider};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

// ── Provider resolution helpers ───────────────────────────────────

pub(super) fn canonical_known_provider_name(name: &str) -> Option<String> {
    let candidate = name.trim();
    if candidate.is_empty() {
        return None;
    }

    let providers_list = providers::list_providers();
    for provider in providers_list {
        if provider.name.eq_ignore_ascii_case(candidate)
            || provider
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(candidate))
        {
            return Some(provider.name.to_string());
        }
    }

    None
}

pub(super) fn resolve_product_priority_provider_alias(name: &str) -> Option<String> {
    let provider_name = canonical_known_provider_name(name)?;
    providers::is_product_priority_provider(&provider_name).then_some(provider_name)
}

pub(super) fn resolved_default_provider(config: &Config) -> String {
    config
        .default_provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| providers::DEFAULT_PROVIDER_NAME.to_string())
}

pub(super) fn resolved_default_model(config: &Config) -> String {
    config
        .default_model
        .clone()
        .unwrap_or_else(|| providers::DEFAULT_PROVIDER_MODEL.to_string())
}

// ── Tool exclusion helpers ─────────────────────────────────────────

pub(super) fn snapshot_non_cli_excluded_tools(ctx: &ChannelRuntimeContext) -> Vec<String> {
    ctx.non_cli_excluded_tools
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub(crate) fn is_tool_excluded(excluded_tools: &[String], tool_name: &str) -> bool {
    let normalized = tool_name.trim().to_ascii_lowercase();
    excluded_tools
        .iter()
        .any(|ex| ex.trim().eq_ignore_ascii_case(&normalized))
}

pub(crate) fn exclusion_set(excluded_tools: &[String]) -> HashSet<String> {
    excluded_tools
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .collect()
}

pub(super) fn filtered_tool_specs_for_runtime(
    tools_registry: &[Box<dyn crate::tools::Tool>],
    excluded_tools: &[String],
) -> Vec<crate::tools::ToolSpec> {
    let excluded = exclusion_set(excluded_tools);
    tools_registry
        .iter()
        .map(|tool| tool.spec())
        .filter(|spec| !excluded.contains(&spec.name.to_ascii_lowercase()))
        .collect()
}

pub(super) fn is_non_cli_tool_excluded(ctx: &ChannelRuntimeContext, tool_name: &str) -> bool {
    let excluded = ctx
        .non_cli_excluded_tools
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    is_tool_excluded(&excluded, tool_name)
}

pub(super) async fn auto_unexclude_tool(ctx: &ChannelRuntimeContext, tool_name: &str) -> String {
    {
        let mut excluded = ctx
            .non_cli_excluded_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        excluded.retain(|t| t != tool_name);
    }
    match remove_non_cli_exclusion_from_config(ctx, tool_name).await {
        Ok(Some(_)) => format!(
            "Also removed `{tool_name}` from `autonomy.non_cli_excluded_tools` (persisted to config)."
        ),
        Ok(None) => format!(
            "Also removed `{tool_name}` from the runtime exclusion list (config path not found, runtime-only)."
        ),
        Err(err) => format!(
            "Removed `{tool_name}` from the runtime exclusion list, but failed to persist: {err}"
        ),
    }
}

pub(super) fn build_runtime_tool_visibility_prompt(
    tools_registry: &[Box<dyn crate::tools::Tool>],
    excluded_tools: &[String],
    native_tools: bool,
) -> String {
    let mut prompt = String::new();
    let mut specs = filtered_tool_specs_for_runtime(tools_registry, excluded_tools);
    specs.sort_by(|a, b| a.name.cmp(&b.name));

    prompt.push_str("\n## Runtime Tool Availability (Authoritative)\n\n");
    prompt.push_str(
        "This section is generated from current runtime policy for this message. \
         Only the listed tools may be called in this turn.\n\n",
    );

    if specs.is_empty() {
        prompt.push_str("- Allowed tools: (none)\n");
    } else {
        let _ = writeln!(prompt, "- Allowed tools ({}):", specs.len());
        for spec in &specs {
            let _ = writeln!(prompt, " - `{}`", spec.name);
        }
    }

    if excluded_tools.is_empty() {
        prompt.push_str("- Excluded by runtime policy: (none)\n\n");
    } else {
        let mut excluded_sorted = excluded_tools.to_vec();
        excluded_sorted.sort();
        let _ = writeln!(
            prompt,
            "- Excluded by runtime policy: {}\n",
            excluded_sorted.join(", ")
        );
    }

    prompt.push_str(
        "- Do not claim tools are unavailable when they are listed above; call the appropriate tool directly.\n",
    );
    prompt.push_str(
        "- If the user asks what you can do, answer from the allowed tool list above plus loaded skills and channel capabilities below.\n",
    );
    prompt.push_str(
        "- Distinguish clearly between actions available now, actions that still require approval, and workflows that remain operator-controlled.\n",
    );
    prompt.push_str(
        "- Self-improvement is not automatic by default; candidate preparation, validation, and promotion remain manual/operator-controlled unless a dedicated workflow was explicitly configured.\n",
    );
    if specs.is_empty() {
        prompt.push_str(
            "- No tool calls are allowed in this turn; reply from current context or ask one brief clarifying question.\n",
        );
        return prompt;
    }
    if specs
        .iter()
        .any(|spec| matches!(spec.name.as_str(), "file_write" | "file_edit"))
    {
        prompt.push_str(
            "- File changes are supported in this turn (`file_write`/`file_edit`) when requested and policy permits.\n",
        );
    }
    if specs.iter().any(|spec| spec.name == "shell") {
        prompt.push_str(
            "- For exact repository metrics such as line counts or language breakdowns, use runtime tools (for example `shell` with `cloc`) instead of estimating from remote hosting metadata or language-byte APIs.\n",
        );
    }

    if native_tools {
        prompt.push_str(
            "Tool calling for this turn uses native provider function-calling. \
             Do not emit `<tool_call>` XML tags.\n",
        );
    } else {
        prompt.push_str(
            "Tool calling for this turn uses XML tool protocol below. \
             This protocol block is generated from the same runtime policy snapshot.\n",
        );
        prompt.push_str(&crate::agent::loop_::build_tool_instructions_from_specs(
            &specs,
        ));
    }

    prompt
}

// ── Runtime config state and persistence ───────────────────────────

#[derive(Debug, Clone)]
pub(super) struct ChannelRuntimeDefaults {
    pub(super) default_provider: String,
    pub(super) model: String,
    pub(super) temperature: f64,
    pub(super) api_key: Option<String>,
    pub(super) api_url: Option<String>,
    pub(super) reliability: crate::config::ReliabilityConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConfigFileStamp {
    pub(super) modified: SystemTime,
    pub(super) len: u64,
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeConfigState {
    pub(super) defaults: ChannelRuntimeDefaults,
    pub(super) last_applied_stamp: Option<ConfigFileStamp>,
}

#[derive(Debug, Clone)]
pub(super) struct RuntimeAutonomyPolicy {
    pub(super) auto_approve: Vec<String>,
    pub(super) always_ask: Vec<String>,
    pub(super) non_cli_excluded_tools: Vec<String>,
    pub(super) non_cli_approval_approvers: Vec<String>,
    pub(super) non_cli_natural_language_approval_mode: NonCliNaturalLanguageApprovalMode,
    pub(super) non_cli_natural_language_approval_mode_by_channel:
        HashMap<String, NonCliNaturalLanguageApprovalMode>,
}

pub(super) fn runtime_config_store() -> &'static Mutex<HashMap<PathBuf, RuntimeConfigState>> {
    static STORE: OnceLock<Mutex<HashMap<PathBuf, RuntimeConfigState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn runtime_defaults_from_config(config: &Config) -> ChannelRuntimeDefaults {
    ChannelRuntimeDefaults {
        default_provider: resolved_default_provider(config),
        model: resolved_default_model(config),
        temperature: config.default_temperature,
        api_key: config.api_key.clone(),
        api_url: config.api_url.clone(),
        reliability: config.reliability.clone(),
    }
}

fn runtime_autonomy_policy_from_config(config: &Config) -> RuntimeAutonomyPolicy {
    RuntimeAutonomyPolicy {
        auto_approve: config.autonomy.auto_approve.clone(),
        always_ask: config.autonomy.always_ask.clone(),
        non_cli_excluded_tools: config.autonomy.non_cli_excluded_tools.clone(),
        non_cli_approval_approvers: config.autonomy.non_cli_approval_approvers.clone(),
        non_cli_natural_language_approval_mode: config
            .autonomy
            .non_cli_natural_language_approval_mode,
        non_cli_natural_language_approval_mode_by_channel: config
            .autonomy
            .non_cli_natural_language_approval_mode_by_channel
            .clone(),
    }
}

pub(super) fn runtime_config_path(ctx: &ChannelRuntimeContext) -> Option<PathBuf> {
    ctx.provider_runtime_options
        .topclaw_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
}

pub(super) fn runtime_defaults_snapshot(ctx: &ChannelRuntimeContext) -> ChannelRuntimeDefaults {
    if let Some(config_path) = runtime_config_path(ctx) {
        let store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(state) = store.get(&config_path) {
            return state.defaults.clone();
        }
    }

    ChannelRuntimeDefaults {
        default_provider: ctx.default_provider.as_str().to_string(),
        model: ctx.model.as_str().to_string(),
        temperature: ctx.temperature,
        api_key: ctx.api_key.clone(),
        api_url: ctx.api_url.clone(),
        reliability: (*ctx.reliability).clone(),
    }
}

pub(super) async fn config_file_stamp(path: &Path) -> Option<ConfigFileStamp> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let modified = metadata.modified().ok()?;
    Some(ConfigFileStamp {
        modified,
        len: metadata.len(),
    })
}

fn decrypt_optional_secret_for_runtime_reload(
    store: &crate::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if crate::security::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .decrypt(&raw)
                    .with_context(|| format!("Failed to decrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

pub(super) async fn load_runtime_defaults_from_config_file(
    path: &Path,
) -> Result<(ChannelRuntimeDefaults, RuntimeAutonomyPolicy)> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut parsed: Config =
        toml::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))?;
    parsed.config_path = path.to_path_buf();

    if let Some(topclaw_dir) = path.parent() {
        let store = crate::security::SecretStore::new(topclaw_dir, parsed.secrets.encrypt);
        decrypt_optional_secret_for_runtime_reload(&store, &mut parsed.api_key, "config.api_key")?;
    }

    parsed.apply_env_overrides();
    Ok((
        runtime_defaults_from_config(&parsed),
        runtime_autonomy_policy_from_config(&parsed),
    ))
}

pub(super) async fn persist_non_cli_approval_to_config(
    ctx: &ChannelRuntimeContext,
    tool_name: &str,
) -> Result<Option<PathBuf>> {
    let Some(config_path) = runtime_config_path(ctx) else {
        return Ok(None);
    };

    let contents = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let mut parsed: Config = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    parsed.config_path = config_path.clone();

    let mut changed = false;
    if !parsed
        .autonomy
        .auto_approve
        .iter()
        .any(|entry| entry == tool_name)
    {
        parsed.autonomy.auto_approve.push(tool_name.to_string());
        changed = true;
    }

    let before_always_ask = parsed.autonomy.always_ask.len();
    parsed
        .autonomy
        .always_ask
        .retain(|entry| entry != tool_name);
    if parsed.autonomy.always_ask.len() != before_always_ask {
        changed = true;
    }

    if changed {
        parsed.save().await?;
    }

    Ok(Some(config_path))
}

pub(super) async fn remove_non_cli_approval_from_config(
    ctx: &ChannelRuntimeContext,
    tool_name: &str,
) -> Result<Option<(PathBuf, bool)>> {
    let Some(config_path) = runtime_config_path(ctx) else {
        return Ok(None);
    };

    let contents = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let mut parsed: Config = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    parsed.config_path = config_path.clone();

    let before_auto_approve = parsed.autonomy.auto_approve.len();
    parsed
        .autonomy
        .auto_approve
        .retain(|entry| entry != tool_name);
    let removed = parsed.autonomy.auto_approve.len() != before_auto_approve;
    if removed {
        parsed.save().await?;
    }

    Ok(Some((config_path, removed)))
}

pub(super) async fn describe_non_cli_approvals(
    ctx: &ChannelRuntimeContext,
    sender: &str,
    channel: &str,
    reply_target: &str,
    excluded_tools: &[String],
) -> Result<String> {
    let mut response = String::new();
    response.push_str("Supervised non-CLI tool approvals:\n");

    let mut runtime_auto = ctx
        .approval_manager
        .auto_approve_tools()
        .into_iter()
        .collect::<Vec<_>>();
    runtime_auto.sort();
    if runtime_auto.is_empty() {
        response.push_str("- Runtime auto_approve (effective): (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime auto_approve (effective): {}",
            runtime_auto.join(", ")
        );
    }

    let mut runtime_always = ctx
        .approval_manager
        .always_ask_tools()
        .into_iter()
        .collect::<Vec<_>>();
    runtime_always.sort();
    if runtime_always.is_empty() {
        response.push_str("- Runtime always_ask (effective): (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime always_ask (effective): {}",
            runtime_always.join(", ")
        );
    }

    let mut session_grants = ctx
        .approval_manager
        .non_cli_session_allowlist()
        .into_iter()
        .collect::<Vec<_>>();
    session_grants.sort();
    if session_grants.is_empty() {
        response.push_str("- Runtime session grants: (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime session grants: {}",
            session_grants.join(", ")
        );
    }
    let one_time_all_tools_tokens = ctx.approval_manager.non_cli_allow_all_once_remaining();
    let _ = writeln!(
        response,
        "- Runtime one-time all-tools bypass tokens: {}",
        one_time_all_tools_tokens
    );

    let mut approval_approvers = ctx
        .approval_manager
        .non_cli_approval_approvers()
        .into_iter()
        .collect::<Vec<_>>();
    approval_approvers.sort();
    if approval_approvers.is_empty() {
        response.push_str("- Runtime non_cli_approval_approvers: (any channel-allowed sender)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime non_cli_approval_approvers: {}",
            approval_approvers.join(", ")
        );
    }

    let default_mode = non_cli_natural_language_mode_label(
        ctx.approval_manager
            .non_cli_natural_language_approval_mode(),
    );
    let effective_mode = non_cli_natural_language_mode_label(
        ctx.approval_manager
            .non_cli_natural_language_approval_mode_for_channel(channel),
    );
    let _ = writeln!(
        response,
        "- Runtime non_cli_natural_language_approval_mode: {}",
        default_mode
    );
    let _ = writeln!(
        response,
        "- Runtime non_cli_natural_language_approval_mode (current channel `{channel}`): {}",
        effective_mode
    );
    let mut mode_overrides = ctx
        .approval_manager
        .non_cli_natural_language_approval_mode_by_channel()
        .into_iter()
        .map(|(ch, mode)| format!("{ch}={}", non_cli_natural_language_mode_label(mode)))
        .collect::<Vec<_>>();
    mode_overrides.sort();
    if mode_overrides.is_empty() {
        response.push_str("- Runtime non_cli_natural_language_approval_mode_by_channel: (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime non_cli_natural_language_approval_mode_by_channel: {}",
            mode_overrides.join(", ")
        );
    }

    let mut pending_requests = ctx.approval_manager.list_non_cli_pending_requests(
        Some(sender),
        Some(channel),
        Some(reply_target),
    );
    pending_requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    if pending_requests.is_empty() {
        response.push_str("- Pending approvals (sender+chat/channel scoped): (none)\n");
    } else {
        response.push_str("- Pending approvals (sender+chat/channel scoped):\n");
        for req in pending_requests {
            let reason = req
                .reason
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("n/a");
            let _ = writeln!(
                response,
                " - {}: tool={}, expires_at={}, reason={}",
                req.request_id,
                approval_target_label(&req.tool_name),
                req.expires_at,
                reason
            );
        }
    }

    let mut excluded = excluded_tools.to_vec();
    excluded.sort();
    if excluded.is_empty() {
        response.push_str("- Runtime non_cli_excluded_tools: (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Runtime non_cli_excluded_tools: {}",
            excluded.join(", ")
        );
    }

    let Some(config_path) = runtime_config_path(ctx) else {
        response.push_str(
            "- Persisted config approvals: unavailable (runtime config path not resolved)\n",
        );
        return Ok(response);
    };

    let contents = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let parsed: Config = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;

    let mut auto_approve = parsed.autonomy.auto_approve;
    auto_approve.sort();
    if auto_approve.is_empty() {
        response.push_str("- Persisted autonomy.auto_approve: (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Persisted autonomy.auto_approve: {}",
            auto_approve.join(", ")
        );
    }

    let mut always_ask = parsed.autonomy.always_ask;
    always_ask.sort();
    if always_ask.is_empty() {
        response.push_str("- Persisted autonomy.always_ask: (none)\n");
    } else {
        let _ = writeln!(
            response,
            "- Persisted autonomy.always_ask: {}",
            always_ask.join(", ")
        );
    }

    let _ = writeln!(response, "- Config path: {}", config_path.display());
    Ok(response)
}

pub(super) async fn remove_non_cli_exclusion_from_config(
    ctx: &ChannelRuntimeContext,
    tool_name: &str,
) -> Result<Option<PathBuf>> {
    let Some(config_path) = runtime_config_path(ctx) else {
        return Ok(None);
    };

    let contents = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let mut parsed: Config = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", config_path.display()))?;
    parsed.config_path = config_path.clone();

    let before_len = parsed.autonomy.non_cli_excluded_tools.len();
    parsed
        .autonomy
        .non_cli_excluded_tools
        .retain(|entry| entry != tool_name);
    if parsed.autonomy.non_cli_excluded_tools.len() != before_len {
        parsed.save().await?;
    }

    Ok(Some(config_path))
}

pub(super) async fn maybe_apply_runtime_config_update(ctx: &ChannelRuntimeContext) -> Result<()> {
    let Some(config_path) = runtime_config_path(ctx) else {
        return Ok(());
    };

    let Some(stamp) = config_file_stamp(&config_path).await else {
        return Ok(());
    };

    {
        let store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(state) = store.get(&config_path) {
            if state.last_applied_stamp == Some(stamp) {
                return Ok(());
            }
        }
    }

    let (next_defaults, next_autonomy_policy) =
        load_runtime_defaults_from_config_file(&config_path).await?;
    let next_default_provider = providers::create_resilient_provider_with_options(
        &next_defaults.default_provider,
        next_defaults.api_key.as_deref(),
        next_defaults.api_url.as_deref(),
        &next_defaults.reliability,
        &ctx.provider_runtime_options,
    )?;
    let next_default_provider: Arc<dyn Provider> = Arc::from(next_default_provider);

    if let Err(err) = next_default_provider.warmup().await {
        tracing::warn!(
            provider = %next_defaults.default_provider,
            "Provider warmup failed after config reload: {err}"
        );
    }

    {
        let mut cache = ctx.provider_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.clear();
        cache.insert(
            next_defaults.default_provider.clone(),
            Arc::clone(&next_default_provider),
        );
    }

    {
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(
            config_path.clone(),
            RuntimeConfigState {
                defaults: next_defaults.clone(),
                last_applied_stamp: Some(stamp),
            },
        );
    }

    ctx.approval_manager.replace_runtime_non_cli_policy(
        &next_autonomy_policy.auto_approve,
        &next_autonomy_policy.always_ask,
        &next_autonomy_policy.non_cli_approval_approvers,
        next_autonomy_policy.non_cli_natural_language_approval_mode,
        &next_autonomy_policy.non_cli_natural_language_approval_mode_by_channel,
    );
    {
        let mut excluded = ctx
            .non_cli_excluded_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *excluded = next_autonomy_policy.non_cli_excluded_tools.clone();
    }

    tracing::info!(
        path = %config_path.display(),
        provider = %next_defaults.default_provider,
        model = %next_defaults.model,
        temperature = next_defaults.temperature,
        non_cli_approval_mode = %non_cli_natural_language_mode_label(
            next_autonomy_policy.non_cli_natural_language_approval_mode
        ),
        non_cli_excluded_tools_count = next_autonomy_policy.non_cli_excluded_tools.len(),
        "Applied updated channel runtime config from disk"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn exclusion_set_normalizes_case_and_whitespace() {
        let tools = vec!["  Shell  ".to_string(), "File_Write".to_string()];
        let set = exclusion_set(&tools);
        assert!(set.contains("shell"));
        assert!(set.contains("file_write"));
        assert!(!set.contains("Shell"));
    }

    #[test]
    fn is_tool_excluded_is_case_insensitive() {
        let excluded = vec!["shell".to_string()];
        assert!(is_tool_excluded(&excluded, "Shell"));
        assert!(is_tool_excluded(&excluded, "  shell  "));
        assert!(!is_tool_excluded(&excluded, "file_write"));
    }

    #[test]
    fn canonical_known_provider_name_matches_case_insensitive() {
        assert_eq!(
            canonical_known_provider_name("OpenAI"),
            Some("openai".to_string())
        );
        assert!(canonical_known_provider_name("nonexistent_provider_xyz").is_none());
    }

    #[test]
    fn resolved_default_provider_falls_back_to_constant() {
        let config = Config::default();
        let provider = resolved_default_provider(&config);
        assert_eq!(provider, providers::DEFAULT_PROVIDER_NAME);
    }

    #[test]
    fn resolved_default_model_falls_back_to_constant() {
        let config = Config::default();
        let model = resolved_default_model(&config);
        assert_eq!(model, providers::DEFAULT_PROVIDER_MODEL);
    }

    #[test]
    fn filtered_tool_specs_excludes_by_name() {
        use crate::tools::ToolSpec;
        let empty_schema = serde_json::Value::Object(serde_json::Map::new());
        let specs = vec![
            ToolSpec {
                name: "shell".to_string(),
                description: "shell".to_string(),
                parameters: empty_schema.clone(),
            },
            ToolSpec {
                name: "file_write".to_string(),
                description: "file_write".to_string(),
                parameters: empty_schema.clone(),
            },
            ToolSpec {
                name: "web_fetch".to_string(),
                description: "web_fetch".to_string(),
                parameters: empty_schema,
            },
        ];
        let excluded = vec!["shell".to_string()];
        let excluded_set = exclusion_set(&excluded);
        let filtered: Vec<&ToolSpec> = specs
            .iter()
            .filter(|s| !excluded_set.contains(&s.name.to_ascii_lowercase()))
            .collect();
        let names: Vec<&str> = filtered.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"shell"));
        assert!(names.contains(&"file_write"));
        assert!(names.contains(&"web_fetch"));
    }
}
