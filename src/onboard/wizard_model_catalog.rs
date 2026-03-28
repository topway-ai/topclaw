use anyhow::{bail, Context, Result};
use console::style;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;

pub(super) const MODEL_CACHE_TTL_SECS: u64 = 12 * 60 * 60;
const MODEL_PREVIEW_LIMIT: usize = 20;
const MODEL_CACHE_FILE: &str = "models_cache.json";

pub(super) fn supports_live_model_fetch(provider_name: &str) -> bool {
    if provider_name.trim().starts_with("custom:") {
        return true;
    }

    matches!(
        super::canonical_provider_name(provider_name),
        "openrouter"
            | "openai"
            | "anthropic"
            | "groq"
            | "mistral"
            | "deepseek"
            | "xai"
            | "together-ai"
            | "gemini"
            | "ollama"
            | "llamacpp"
            | "sglang"
            | "vllm"
            | "osaurus"
            | "astrai"
            | "venice"
            | "fireworks"
            | "novita"
            | "cohere"
            | "moonshot"
            | "glm"
            | "zai"
            | "qwen"
            | "nvidia"
    )
}

pub(super) fn models_endpoint_for_provider(provider_name: &str) -> Option<&'static str> {
    match provider_name {
        "qwen-intl" => Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models"),
        "dashscope-us" => Some("https://dashscope-us.aliyuncs.com/compatible-mode/v1/models"),
        "moonshot-cn" | "kimi-cn" => Some("https://api.moonshot.cn/v1/models"),
        "glm-cn" | "bigmodel" => Some("https://open.bigmodel.cn/api/paas/v4/models"),
        "zai-cn" | "z.ai-cn" => Some("https://open.bigmodel.cn/api/coding/paas/v4/models"),
        _ => match super::canonical_provider_name(provider_name) {
            "openai-codex" | "openai" => Some("https://api.openai.com/v1/models"),
            "venice" => Some("https://api.venice.ai/api/v1/models"),
            "groq" => Some("https://api.groq.com/openai/v1/models"),
            "mistral" => Some("https://api.mistral.ai/v1/models"),
            "deepseek" => Some("https://api.deepseek.com/v1/models"),
            "xai" => Some("https://api.x.ai/v1/models"),
            "together-ai" => Some("https://api.together.xyz/v1/models"),
            "fireworks" => Some("https://api.fireworks.ai/inference/v1/models"),
            "novita" => Some("https://api.novita.ai/openai/v1/models"),
            "cohere" => Some("https://api.cohere.com/compatibility/v1/models"),
            "moonshot" => Some("https://api.moonshot.ai/v1/models"),
            "glm" => Some("https://api.z.ai/api/paas/v4/models"),
            "zai" => Some("https://api.z.ai/api/coding/paas/v4/models"),
            "qwen" => Some("https://dashscope.aliyuncs.com/compatible-mode/v1/models"),
            "nvidia" => Some("https://integrate.api.nvidia.com/v1/models"),
            "astrai" => Some("https://as-trai.com/v1/models"),
            "llamacpp" => Some("http://localhost:8080/v1/models"),
            "sglang" => Some("http://localhost:30000/v1/models"),
            "vllm" => Some("http://localhost:8000/v1/models"),
            "osaurus" => Some("http://localhost:1337/v1/models"),
            _ => None,
        },
    }
}

fn build_model_fetch_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .build()
        .context("failed to build model-fetch HTTP client")
}

fn run_model_fetch_in_thread<T, F>(fetch: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    std::thread::spawn(fetch)
        .join()
        .map_err(|_| anyhow::anyhow!("model fetch thread panicked"))?
}

pub(super) fn normalize_model_ids(ids: Vec<String>) -> Vec<String> {
    let mut unique = BTreeMap::new();
    for id in ids {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            unique
                .entry(trimmed.to_ascii_lowercase())
                .or_insert_with(|| trimmed.to_string());
        }
    }
    unique.into_values().collect()
}

pub(super) fn parse_openai_compatible_model_ids(payload: &Value) -> Vec<String> {
    let mut models = Vec::new();

    if let Some(data) = payload.get("data").and_then(Value::as_array) {
        for model in data {
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                models.push(id.to_string());
            }
        }
    } else if let Some(data) = payload.as_array() {
        for model in data {
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                models.push(id.to_string());
            }
        }
    }

    normalize_model_ids(models)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenRouterModelSummary {
    pub(super) id: String,
    pub(super) name: String,
}

fn parse_openrouter_model_summaries(payload: &Value) -> Vec<OpenRouterModelSummary> {
    let entries = payload
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| payload.as_array());

    let Some(entries) = entries else {
        return Vec::new();
    };

    let mut unique = BTreeMap::new();
    for model in entries {
        let Some(id) = model.get("id").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let name = model
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id);

        unique
            .entry(id.to_ascii_lowercase())
            .or_insert_with(|| OpenRouterModelSummary {
                id: id.to_string(),
                name: name.to_string(),
            });
    }

    unique.into_values().collect()
}

fn normalize_openrouter_ranking_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut last_was_space = false;

    for ch in name.chars() {
        let mapped = match ch {
            ':' | '/' | '-' | '_' => ' ',
            _ => ch,
        };

        if mapped.is_ascii_alphanumeric() {
            normalized.push(mapped.to_ascii_lowercase());
            last_was_space = false;
        } else if mapped.is_whitespace() && !last_was_space && !normalized.is_empty() {
            normalized.push(' ');
            last_was_space = true;
        }
    }

    normalized.trim().to_string()
}

fn openrouter_model_name_aliases(entry: &OpenRouterModelSummary) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut push_alias = |value: &str| {
        let normalized = normalize_openrouter_ranking_name(value);
        if !normalized.is_empty() && !aliases.iter().any(|existing| existing == &normalized) {
            aliases.push(normalized);
        }
    };

    push_alias(&entry.name);
    push_alias(&entry.id);

    if let Some((_, rest)) = entry.name.split_once(':') {
        push_alias(rest);
    }
    if let Some(last_segment) = entry.id.rsplit('/').next() {
        push_alias(last_segment);
    }

    aliases
}

pub(super) fn parse_openrouter_rankings_model_names(html: &str, limit: usize) -> Vec<String> {
    let Some(leaderboard_start) = html.find("LLM Leaderboard") else {
        return Vec::new();
    };
    let leaderboard_html = &html[leaderboard_start..];
    let link_regex = Regex::new(r#"<a[^>]+href="/[^"/?#]+/[^"?#]+"[^>]*>([^<]+)</a>"#)
        .expect("valid OpenRouter rankings link regex");

    let mut names = Vec::new();
    for capture in link_regex.captures_iter(leaderboard_html) {
        let Some(name) = capture.get(1).map(|value| value.as_str().trim()) else {
            continue;
        };
        if name.is_empty()
            || name.eq_ignore_ascii_case("Top Apps")
            || name.eq_ignore_ascii_case("View all")
        {
            continue;
        }
        if names.iter().any(|existing| existing == name) {
            continue;
        }
        names.push(name.to_string());
        if names.len() >= limit {
            break;
        }
    }

    names
}

pub(super) fn match_openrouter_rankings_to_model_ids(
    ranked_names: &[String],
    catalog: &[OpenRouterModelSummary],
    limit: usize,
) -> Vec<String> {
    let mut matched = Vec::new();
    let mut catalog_by_name = BTreeMap::new();
    for entry in catalog {
        for alias in openrouter_model_name_aliases(entry) {
            catalog_by_name.insert(alias, entry.id.clone());
        }
    }

    for ranked_name in ranked_names {
        let normalized = normalize_openrouter_ranking_name(ranked_name);
        if normalized.is_empty() {
            continue;
        }
        let resolved = catalog_by_name.get(&normalized).cloned().or_else(|| {
            catalog_by_name
                .iter()
                .find(|(alias, _)| {
                    alias.starts_with(&normalized)
                        || normalized.starts_with(alias.as_str())
                        || alias.contains(&normalized)
                })
                .map(|(_, model_id)| model_id.clone())
        });

        if let Some(model_id) = resolved {
            if !matched.iter().any(|existing| existing == &model_id) {
                matched.push(model_id);
            }
        }
        if matched.len() >= limit {
            break;
        }
    }

    matched
}

pub(super) fn parse_gemini_model_ids(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    for model in models {
        let supports_generate_content = model
            .get("supportedGenerationMethods")
            .and_then(Value::as_array)
            .is_none_or(|methods| {
                methods
                    .iter()
                    .any(|method| method.as_str() == Some("generateContent"))
            });

        if !supports_generate_content {
            continue;
        }

        if let Some(name) = model.get("name").and_then(Value::as_str) {
            ids.push(name.trim_start_matches("models/").to_string());
        }
    }

    normalize_model_ids(ids)
}

pub(super) fn parse_ollama_model_ids(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    for model in models {
        if let Some(name) = model.get("name").and_then(Value::as_str) {
            ids.push(name.to_string());
        }
    }

    normalize_model_ids(ids)
}

fn fetch_openai_compatible_models(
    endpoint: &str,
    api_key: Option<&str>,
    allow_unauthenticated: bool,
) -> Result<Vec<String>> {
    let endpoint = endpoint.to_string();
    let api_key = api_key.map(str::to_string);
    run_model_fetch_in_thread(move || {
        let client = build_model_fetch_client()?;
        let mut request = client.get(&endpoint);

        if let Some(api_key) = api_key.as_deref() {
            request = request.bearer_auth(api_key);
        } else if !allow_unauthenticated {
            bail!("model fetch requires API key for endpoint {endpoint}");
        }

        let payload: Value = request
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .with_context(|| format!("model fetch failed: GET {endpoint}"))?
            .json()
            .context("failed to parse model list response")?;

        Ok(parse_openai_compatible_model_ids(&payload))
    })
}

fn fetch_openrouter_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let fetch_once = |api_key: Option<String>| {
        run_model_fetch_in_thread(move || {
            let client = build_model_fetch_client()?;
            let mut request = client.get("https://openrouter.ai/api/v1/models");
            if let Some(api_key) = api_key.as_deref() {
                request = request.bearer_auth(api_key);
            }

            let payload: Value = request
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .context("model fetch failed: GET https://openrouter.ai/api/v1/models")?
                .json()
                .context("failed to parse OpenRouter model list response")?;

            Ok(parse_openai_compatible_model_ids(&payload))
        })
    };

    let api_key = api_key.map(str::to_string);
    match fetch_once(api_key.clone()) {
        Ok(models) => Ok(models),
        Err(err) if api_key.is_some() => fetch_once(None).or(Err(err)),
        Err(err) => Err(err),
    }
}

pub(super) fn fetch_openrouter_top_onboarding_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let fetch_once = |api_key: Option<String>| {
        run_model_fetch_in_thread(move || {
            let client = build_model_fetch_client()?;
            let mut models_request = client.get("https://openrouter.ai/api/v1/models");
            if let Some(api_key) = api_key.as_deref() {
                models_request = models_request.bearer_auth(api_key);
            }

            let models_payload: Value = models_request
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .context("model fetch failed: GET https://openrouter.ai/api/v1/models")?
                .json()
                .context("failed to parse OpenRouter model list response")?;
            let catalog = parse_openrouter_model_summaries(&models_payload);
            if catalog.is_empty() {
                bail!("OpenRouter model list did not include any ids");
            }

            let rankings_html = client
                .get("https://openrouter.ai/rankings")
                .send()
                .and_then(reqwest::blocking::Response::error_for_status)
                .context("model fetch failed: GET https://openrouter.ai/rankings")?
                .text()
                .context("failed to read OpenRouter rankings page")?;

            let ranked_names = parse_openrouter_rankings_model_names(
                &rankings_html,
                super::OPENROUTER_ONBOARDING_MODEL_LIMIT,
            );
            if ranked_names.is_empty() {
                bail!("OpenRouter rankings page did not include a leaderboard");
            }

            let ranked_ids = match_openrouter_rankings_to_model_ids(
                &ranked_names,
                &catalog,
                super::OPENROUTER_ONBOARDING_MODEL_LIMIT,
            );
            if ranked_ids.is_empty() {
                bail!("OpenRouter rankings models did not match the fetched catalog");
            }

            Ok(ranked_ids)
        })
    };

    let api_key = api_key.map(str::to_string);
    match fetch_once(api_key.clone()) {
        Ok(models) => Ok(models),
        Err(err) if api_key.is_some() => fetch_once(None).or(Err(err)),
        Err(err) => Err(err),
    }
}

pub(super) fn interactive_model_labels(model_options: &[(String, String)]) -> Vec<String> {
    model_options
        .iter()
        .map(|(_, label)| label.clone())
        .collect()
}

fn fetch_anthropic_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let api_key = api_key.map(str::to_string);
    run_model_fetch_in_thread(move || {
        let client = build_model_fetch_client()?;
        let mut request = client.get("https://api.anthropic.com/v1/models");
        if let Some(api_key) = api_key.as_deref() {
            request = request.header("x-api-key", api_key);
        }

        let payload: Value = request
            .header("anthropic-version", "2023-06-01")
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .context("model fetch failed: GET https://api.anthropic.com/v1/models")?
            .json()
            .context("failed to parse Anthropic model list response")?;

        Ok(parse_openai_compatible_model_ids(&payload))
    })
}

fn fetch_gemini_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let api_key = api_key.map(str::to_string);
    run_model_fetch_in_thread(move || {
        let client = build_model_fetch_client()?;
        let api_key = api_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("model fetch requires GEMINI_API_KEY"))?;

        let payload: Value = client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={api_key}"
            ))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .context(
                "model fetch failed: GET https://generativelanguage.googleapis.com/v1beta/models",
            )?
            .json()
            .context("failed to parse Gemini model list response")?;

        Ok(parse_gemini_model_ids(&payload))
    })
}

fn fetch_ollama_models() -> Result<Vec<String>> {
    run_model_fetch_in_thread(move || {
        let client = build_model_fetch_client()?;
        let payload: Value = client
            .get("http://localhost:11434/api/tags")
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .context("model fetch failed: GET http://localhost:11434/api/tags")?
            .json()
            .context("failed to parse Ollama model list response")?;

        Ok(parse_ollama_model_ids(&payload))
    })
}

pub(super) fn normalize_ollama_endpoint_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .strip_suffix("/api")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_string()
}

fn ollama_endpoint_is_local(endpoint_url: &str) -> bool {
    reqwest::Url::parse(endpoint_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "0.0.0.0"))
}

pub(super) fn ollama_uses_remote_endpoint(provider_api_url: Option<&str>) -> bool {
    let Some(endpoint) = provider_api_url else {
        return false;
    };

    let normalized = normalize_ollama_endpoint_url(endpoint);
    if normalized.is_empty() {
        return false;
    }

    !ollama_endpoint_is_local(&normalized)
}

pub(super) fn resolve_live_models_endpoint(
    provider_name: &str,
    provider_api_url: Option<&str>,
) -> Option<String> {
    if let Some(raw_base) = provider_name.strip_prefix("custom:") {
        let normalized = raw_base.trim().trim_end_matches('/');
        if normalized.is_empty() {
            return None;
        }
        if normalized.ends_with("/models") {
            return Some(normalized.to_string());
        }
        return Some(format!("{normalized}/models"));
    }

    if matches!(
        super::canonical_provider_name(provider_name),
        "llamacpp" | "sglang" | "vllm" | "osaurus"
    ) {
        if let Some(url) = provider_api_url
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            let normalized = url.trim_end_matches('/');
            if normalized.ends_with("/models") {
                return Some(normalized.to_string());
            }
            return Some(format!("{normalized}/models"));
        }
    }

    if super::canonical_provider_name(provider_name) == "openai-codex" {
        if let Some(url) = provider_api_url
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            let normalized = url.trim_end_matches('/');
            if normalized.ends_with("/models") {
                return Some(normalized.to_string());
            }
            return Some(format!("{normalized}/models"));
        }
    }

    models_endpoint_for_provider(provider_name).map(str::to_string)
}

pub(super) fn fetch_live_models_for_provider(
    provider_name: &str,
    api_key: &str,
    provider_api_url: Option<&str>,
) -> Result<Vec<String>> {
    let requested_provider_name = provider_name;
    let provider_name = super::canonical_provider_name(provider_name);
    let ollama_remote = provider_name == "ollama" && ollama_uses_remote_endpoint(provider_api_url);
    let api_key = if api_key.trim().is_empty() {
        if provider_name == "ollama" && !ollama_remote {
            None
        } else {
            std::env::var(super::provider_env_var(provider_name))
                .ok()
                .or_else(|| {
                    if provider_name == "anthropic" {
                        std::env::var("ANTHROPIC_OAUTH_TOKEN").ok()
                    } else if provider_name == "minimax" {
                        std::env::var("MINIMAX_OAUTH_TOKEN").ok()
                    } else {
                        None
                    }
                })
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        }
    } else {
        Some(api_key.trim().to_string())
    };

    let models = match provider_name {
        "openrouter" => fetch_openrouter_models(api_key.as_deref())?,
        "anthropic" => fetch_anthropic_models(api_key.as_deref())?,
        "gemini" => fetch_gemini_models(api_key.as_deref())?,
        "ollama" => {
            if ollama_remote {
                vec![
                    "glm-5:cloud".to_string(),
                    "glm-4.7:cloud".to_string(),
                    "gpt-oss:20b:cloud".to_string(),
                    "gpt-oss:120b:cloud".to_string(),
                    "gemini-3-flash-preview:cloud".to_string(),
                    "qwen3-coder-next:cloud".to_string(),
                    "qwen3-coder:480b:cloud".to_string(),
                    "kimi-k2.5:cloud".to_string(),
                    "minimax-m2.5:cloud".to_string(),
                    "deepseek-v3.1:671b:cloud".to_string(),
                ]
            } else {
                fetch_ollama_models()?
                    .into_iter()
                    .filter(|model_id| !model_id.ends_with(":cloud"))
                    .collect()
            }
        }
        _ => {
            if let Some(endpoint) =
                resolve_live_models_endpoint(requested_provider_name, provider_api_url)
            {
                let allow_unauthenticated =
                    super::allows_unauthenticated_model_fetch(requested_provider_name);
                fetch_openai_compatible_models(
                    &endpoint,
                    api_key.as_deref(),
                    allow_unauthenticated,
                )?
            } else {
                Vec::new()
            }
        }
    };

    Ok(models)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ModelCacheEntry {
    pub(super) provider: String,
    pub(super) fetched_at_unix: u64,
    pub(super) models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ModelCacheState {
    pub(super) entries: Vec<ModelCacheEntry>,
}

#[derive(Debug, Clone)]
pub(super) struct CachedModels {
    pub(super) models: Vec<String>,
    pub(super) age_secs: u64,
}

fn model_cache_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("state").join(MODEL_CACHE_FILE)
}

pub(super) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

async fn load_model_cache_state(workspace_dir: &Path) -> Result<ModelCacheState> {
    let path = model_cache_path(workspace_dir);
    if !path.exists() {
        return Ok(ModelCacheState::default());
    }

    let raw = fs::read_to_string(&path)
        .await
        .with_context(|| format!("failed to read model cache at {}", path.display()))?;

    match serde_json::from_str::<ModelCacheState>(&raw) {
        Ok(state) => Ok(state),
        Err(_) => Ok(ModelCacheState::default()),
    }
}

pub(super) async fn save_model_cache_state(
    workspace_dir: &Path,
    state: &ModelCacheState,
) -> Result<()> {
    let path = model_cache_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.with_context(|| {
            format!(
                "failed to create model cache directory {}",
                parent.display()
            )
        })?;
    }

    let json = serde_json::to_vec_pretty(state).context("failed to serialize model cache")?;
    fs::write(&path, json)
        .await
        .with_context(|| format!("failed to write model cache at {}", path.display()))?;

    Ok(())
}

pub(super) async fn cache_live_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
    models: &[String],
) -> Result<()> {
    let normalized_models = normalize_model_ids(models.to_vec());
    if normalized_models.is_empty() {
        return Ok(());
    }

    let mut state = load_model_cache_state(workspace_dir).await?;
    let now = now_unix_secs();

    if let Some(entry) = state
        .entries
        .iter_mut()
        .find(|entry| entry.provider == provider_name)
    {
        entry.fetched_at_unix = now;
        entry.models = normalized_models;
    } else {
        state.entries.push(ModelCacheEntry {
            provider: provider_name.to_string(),
            fetched_at_unix: now,
            models: normalized_models,
        });
    }

    save_model_cache_state(workspace_dir, &state).await
}

async fn load_cached_models_for_provider_internal(
    workspace_dir: &Path,
    provider_name: &str,
    ttl_secs: Option<u64>,
) -> Result<Option<CachedModels>> {
    let state = load_model_cache_state(workspace_dir).await?;
    let now = now_unix_secs();

    let Some(entry) = state
        .entries
        .into_iter()
        .find(|entry| entry.provider == provider_name)
    else {
        return Ok(None);
    };

    if entry.models.is_empty() {
        return Ok(None);
    }

    let age_secs = now.saturating_sub(entry.fetched_at_unix);
    if ttl_secs.is_some_and(|ttl| age_secs > ttl) {
        return Ok(None);
    }

    Ok(Some(CachedModels {
        models: entry.models,
        age_secs,
    }))
}

pub(super) async fn load_cached_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
    ttl_secs: u64,
) -> Result<Option<CachedModels>> {
    load_cached_models_for_provider_internal(workspace_dir, provider_name, Some(ttl_secs)).await
}

pub(super) async fn load_any_cached_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
) -> Result<Option<CachedModels>> {
    load_cached_models_for_provider_internal(workspace_dir, provider_name, None).await
}

pub(super) fn humanize_age(age_secs: u64) -> String {
    if age_secs < 60 {
        format!("{age_secs}s")
    } else if age_secs < 60 * 60 {
        format!("{}m", age_secs / 60)
    } else {
        format!("{}h", age_secs / (60 * 60))
    }
}

pub(super) fn build_model_options(model_ids: Vec<String>, source: &str) -> Vec<(String, String)> {
    model_ids
        .into_iter()
        .map(|model_id| {
            let label = format!("{model_id} ({source})");
            (model_id, label)
        })
        .collect()
}

fn print_model_preview(models: &[String]) {
    for model in models.iter().take(MODEL_PREVIEW_LIMIT) {
        println!("  {} {model}", style("-"));
    }

    if models.len() > MODEL_PREVIEW_LIMIT {
        println!(
            "  {} ... and {} more",
            style("-"),
            models.len() - MODEL_PREVIEW_LIMIT
        );
    }
}

pub async fn run_models_refresh(
    config: &crate::config::Config,
    provider_override: Option<&str>,
    force: bool,
) -> Result<()> {
    let provider_name = provider_override
        .or(config.default_provider.as_deref())
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME)
        .trim()
        .to_string();

    if provider_name.is_empty() {
        anyhow::bail!("Provider name cannot be empty");
    }

    if !supports_live_model_fetch(&provider_name) {
        anyhow::bail!("Provider '{provider_name}' does not support live model discovery yet");
    }

    if !force {
        if let Some(cached) = load_cached_models_for_provider(
            &config.workspace_dir,
            &provider_name,
            MODEL_CACHE_TTL_SECS,
        )
        .await?
        {
            println!(
                "Using cached model list for '{}' (updated {} ago):",
                provider_name,
                humanize_age(cached.age_secs)
            );
            print_model_preview(&cached.models);
            println!();
            println!(
                "Tip: run `topclaw models refresh --force --provider {}` to fetch latest now.",
                provider_name
            );
            return Ok(());
        }
    }

    let api_key = config.api_key.clone().unwrap_or_default();

    match fetch_live_models_for_provider(&provider_name, &api_key, config.api_url.as_deref()) {
        Ok(models) if !models.is_empty() => {
            cache_live_models_for_provider(&config.workspace_dir, &provider_name, &models).await?;
            println!(
                "Refreshed '{}' model cache with {} models.",
                provider_name,
                models.len()
            );
            print_model_preview(&models);
            Ok(())
        }
        Ok(_) => {
            if let Some(stale_cache) =
                load_any_cached_models_for_provider(&config.workspace_dir, &provider_name).await?
            {
                println!(
                    "Provider returned no models; using stale cache (updated {} ago):",
                    humanize_age(stale_cache.age_secs)
                );
                print_model_preview(&stale_cache.models);
                return Ok(());
            }

            anyhow::bail!("Provider '{}' returned an empty model list", provider_name)
        }
        Err(error) => {
            if let Some(stale_cache) =
                load_any_cached_models_for_provider(&config.workspace_dir, &provider_name).await?
            {
                println!(
                    "Live refresh failed ({}). Falling back to stale cache (updated {} ago):",
                    error,
                    humanize_age(stale_cache.age_secs)
                );
                print_model_preview(&stale_cache.models);
                return Ok(());
            }

            Err(error)
                .with_context(|| format!("failed to refresh models for provider '{provider_name}'"))
        }
    }
}

pub async fn run_models_list(
    config: &crate::config::Config,
    provider_override: Option<&str>,
) -> Result<()> {
    let provider_name = provider_override
        .or(config.default_provider.as_deref())
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME);

    let cached = load_any_cached_models_for_provider(&config.workspace_dir, provider_name).await?;

    let Some(cached) = cached else {
        println!();
        println!(
            "  No cached models for '{provider_name}'. Run: topclaw models refresh --provider {provider_name}"
        );
        println!();
        return Ok(());
    };

    println!();
    println!(
        "  {} models for '{}' (cached {} ago):",
        cached.models.len(),
        provider_name,
        humanize_age(cached.age_secs)
    );
    println!();
    for model in &cached.models {
        let marker = if config.default_model.as_deref() == Some(model.as_str()) {
            "* "
        } else {
            "  "
        };
        println!("  {marker}{model}");
    }
    println!();
    Ok(())
}

pub async fn run_models_set(config: &crate::config::Config, model: &str) -> Result<()> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("Model name cannot be empty");
    }

    let mut updated = config.clone();
    updated.default_model = Some(model.to_string());
    updated.save().await?;

    println!();
    println!("  Default model set to '{}'.", style(model).green().bold());
    println!();
    Ok(())
}

pub async fn run_models_status(config: &crate::config::Config) -> Result<()> {
    let provider = config
        .default_provider
        .as_deref()
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME);
    let model = config.default_model.as_deref().unwrap_or("(not set)");

    println!();
    println!("  Provider:  {}", style(provider).cyan());
    println!("  Model:     {}", style(model).cyan());
    println!(
        "  Temp:      {}",
        style(format!("{:.1}", config.default_temperature)).cyan()
    );

    match load_any_cached_models_for_provider(&config.workspace_dir, provider).await? {
        Some(cached) => {
            println!(
                "  Cache:     {} models (updated {} ago)",
                cached.models.len(),
                humanize_age(cached.age_secs)
            );
            let fresh = cached.age_secs < MODEL_CACHE_TTL_SECS;
            if fresh {
                println!("  Freshness: {}", style("fresh").green());
            } else {
                println!("  Freshness: {}", style("stale").yellow());
            }
        }
        None => {
            println!("  Cache:     {}", style("none").yellow());
        }
    }

    println!();
    Ok(())
}

pub async fn cached_model_catalog_stats(
    config: &crate::config::Config,
    provider_name: &str,
) -> Result<Option<(usize, u64)>> {
    let Some(cached) =
        load_any_cached_models_for_provider(&config.workspace_dir, provider_name).await?
    else {
        return Ok(None);
    };
    Ok(Some((cached.models.len(), cached.age_secs)))
}

pub(super) fn default_live_model_refresh_targets() -> Vec<String> {
    crate::providers::FIRST_CLASS_PROVIDER_PRIORITY
        .iter()
        .map(|provider| (*provider).to_string())
        .collect()
}

pub(super) fn all_live_model_refresh_targets() -> Vec<String> {
    let mut targets: Vec<String> = crate::providers::list_providers()
        .into_iter()
        .map(|provider| provider.name.to_string())
        .filter(|name| supports_live_model_fetch(name))
        .collect();

    targets.sort();
    targets.dedup();
    targets
}

pub async fn run_models_refresh_all(
    config: &crate::config::Config,
    force: bool,
    all_known_providers: bool,
) -> Result<()> {
    let targets = if all_known_providers {
        all_live_model_refresh_targets()
    } else {
        default_live_model_refresh_targets()
    };

    if targets.is_empty() {
        anyhow::bail!("No providers support live model discovery");
    }

    println!(
        "Refreshing model catalogs for {} providers (scope: {}, force: {})",
        targets.len(),
        if all_known_providers {
            "all live-discovery providers"
        } else {
            "first-class priority set"
        },
        if force { "yes" } else { "no" }
    );
    println!();

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    let mut skipped_count = 0usize;

    for provider_name in &targets {
        println!("== {} ==", provider_name);
        if !supports_live_model_fetch(provider_name) {
            skipped_count += 1;
            println!("  skipped: provider does not support live model discovery yet");
            println!();
            continue;
        }
        match run_models_refresh(config, Some(provider_name), force).await {
            Ok(()) => {
                ok_count += 1;
            }
            Err(error) => {
                fail_count += 1;
                println!("  failed: {error}");
            }
        }
        println!();
    }

    println!(
        "Summary: {} succeeded, {} skipped, {} failed",
        ok_count, skipped_count, fail_count
    );

    if ok_count == 0 {
        anyhow::bail!("Model refresh failed for all providers")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_live_model_refresh_targets_follow_first_class_priority() {
        assert_eq!(
            default_live_model_refresh_targets(),
            crate::providers::FIRST_CLASS_PROVIDER_PRIORITY
                .iter()
                .map(|provider| (*provider).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn all_live_model_refresh_targets_include_advanced_live_fetch_providers_only() {
        let targets = all_live_model_refresh_targets();
        assert!(targets.contains(&"anthropic".to_string()));
        assert!(targets.contains(&"openrouter".to_string()));
        assert!(targets.contains(&"ollama".to_string()));
        assert!(!targets.contains(&"openai-codex".to_string()));
    }
}
