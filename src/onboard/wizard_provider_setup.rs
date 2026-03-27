use super::{
    canonical_provider_name, is_glm_alias, is_glm_cn_alias, is_minimax_alias, is_moonshot_alias,
    is_qianfan_alias, is_qwen_alias, is_zai_alias, is_zai_cn_alias, local_provider_choices,
    maybe_prompt_openai_codex_login, normalize_ollama_endpoint_url, print_bullet,
    prompt_for_default_model, provider_env_var, provider_supports_keyless_local_usage,
    provider_uses_oauth_without_api_key,
};
use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, Input};
use std::path::Path;

pub(super) async fn setup_simple_custom_provider(
    workspace_dir: &Path,
    config_path: &Path,
    encrypt_secrets: bool,
) -> Result<(String, String, String, Option<String>)> {
    println!();
    print_bullet("Enter the base URL for any OpenAI-compatible endpoint.");
    let base_url: String = Input::new().with_prompt("  API base URL").interact_text()?;

    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        anyhow::bail!("Custom provider requires a base URL.");
    }

    let api_key: String = Input::new()
        .with_prompt("  API key (or Enter to skip if not needed)")
        .allow_empty(true)
        .interact_text()?;

    let model = prompt_for_default_model(workspace_dir, "openai", &api_key, None).await?;

    let provider_name = format!("custom:{base_url}");
    maybe_prompt_openai_codex_login(&provider_name, config_path, encrypt_secrets).await?;
    Ok((provider_name, api_key, model, None))
}

pub(super) async fn setup_simple_named_provider(
    workspace_dir: &Path,
    config_path: &Path,
    encrypt_secrets: bool,
    provider_name: &str,
) -> Result<(String, String, String, Option<String>)> {
    let mut provider_api_url: Option<String> = None;
    let api_key = if provider_uses_oauth_without_api_key(provider_name) {
        print_bullet("OpenAI Codex uses ChatGPT OAuth, not an API key.");
        print_bullet(
            "TopClaw will start OAuth automatically during onboarding if needed, or you can run `topclaw auth login --provider openai-codex` later.",
        );
        String::new()
    } else if provider_supports_keyless_local_usage(provider_name) {
        if provider_name == "ollama" {
            print_bullet("Using local Ollama at http://localhost:11434.");
        }
        String::new()
    } else {
        let env_var = provider_env_var(provider_name);
        Input::new()
            .with_prompt(format!("  API key ({env_var})"))
            .interact_text()?
    };

    if provider_name == "ollama" {
        provider_api_url = Some("http://localhost:11434".into());
    }

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

    Ok((provider_name.to_string(), api_key, model, provider_api_url))
}

pub(super) fn advanced_provider_choices(tier_idx: usize) -> Vec<(&'static str, &'static str)> {
    match tier_idx {
        0 => vec![
            (
                "openai-codex",
                "OpenAI Codex — primary path (ChatGPT OAuth, no API key)",
            ),
            (
                "openrouter",
                "OpenRouter — secondary path (broad hosted model access)",
            ),
            ("ollama", "Ollama — tertiary local path (no API key)"),
            ("anthropic", "Anthropic — Claude Sonnet & Opus (direct)"),
            ("openai", "OpenAI — GPT-5 direct API"),
            ("gemini", "Google Gemini — supports CLI auth"),
            ("venice", "Venice AI — privacy-first (Llama, Opus)"),
            ("deepseek", "DeepSeek — V3 & R1 (affordable)"),
            ("mistral", "Mistral — Large & Codestral"),
            ("xai", "xAI — Grok 3 & 4"),
            ("perplexity", "Perplexity — search-augmented AI"),
        ],
        1 => vec![
            ("groq", "Groq — ultra-fast LPU inference"),
            ("fireworks", "Fireworks AI — fast open-source inference"),
            ("novita", "Novita AI — affordable open-source inference"),
            ("together-ai", "Together AI — open-source model hosting"),
            ("nvidia", "NVIDIA NIM — DeepSeek, Llama, & more"),
        ],
        2 => vec![
            ("vercel", "Vercel AI Gateway"),
            ("cloudflare", "Cloudflare AI Gateway"),
            (
                "astrai",
                "Astrai — compliant AI routing (PII stripping, cost optimization)",
            ),
            ("bedrock", "Amazon Bedrock — AWS managed models"),
        ],
        3 => vec![
            (
                "kimi-code",
                "Kimi Code — coding-optimized Kimi API (KimiCLI)",
            ),
            (
                "qwen-code",
                "Qwen Code — OAuth tokens reused from ~/.qwen/oauth_creds.json",
            ),
            ("moonshot", "Moonshot — Kimi API (China endpoint)"),
            (
                "moonshot-intl",
                "Moonshot — Kimi API (international endpoint)",
            ),
            ("glm", "GLM — ChatGLM / Zhipu (international endpoint)"),
            ("glm-cn", "GLM — ChatGLM / Zhipu (China endpoint)"),
            (
                "minimax",
                "MiniMax — international endpoint (api.minimax.io)",
            ),
            ("minimax-cn", "MiniMax — China endpoint (api.minimaxi.com)"),
            ("qwen", "Qwen — DashScope China endpoint"),
            ("qwen-intl", "Qwen — DashScope international endpoint"),
            ("qwen-us", "Qwen — DashScope US endpoint"),
            ("hunyuan", "Hunyuan — Tencent large models (T1, Turbo, Pro)"),
            ("qianfan", "Qianfan — Baidu AI models (China endpoint)"),
            ("zai", "Z.AI — global coding endpoint"),
            ("zai-cn", "Z.AI — China coding endpoint (open.bigmodel.cn)"),
            ("synthetic", "Synthetic — Synthetic AI models"),
            ("opencode", "OpenCode Zen — code-focused AI"),
            ("cohere", "Cohere — Command R+ & embeddings"),
        ],
        4 => local_provider_choices(),
        _ => vec![],
    }
}

pub(super) async fn setup_advanced_custom_provider(
    config_path: &Path,
    encrypt_secrets: bool,
) -> Result<(String, String, String, Option<String>)> {
    println!();
    println!(
        "  {} {}",
        style("Custom Provider Setup").white().bold(),
        style("— any OpenAI-compatible API").dim()
    );
    print_bullet("TopClaw works with ANY API that speaks the OpenAI chat completions format.");
    print_bullet("Examples: LiteLLM, LocalAI, vLLM, text-generation-webui, LM Studio, etc.");
    println!();

    let base_url: String = Input::new()
        .with_prompt("  API base URL (e.g. http://localhost:1234 or https://my-api.com)")
        .interact_text()?;

    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        anyhow::bail!("Custom provider requires a base URL.");
    }

    let api_key: String = Input::new()
        .with_prompt("  API key (or Enter to skip if not needed)")
        .allow_empty(true)
        .interact_text()?;

    let model: String = Input::new()
        .with_prompt("  Model name (e.g. llama3, gpt-4o, mistral)")
        .default("default".into())
        .interact_text()?;

    let provider_name = format!("custom:{base_url}");

    println!(
        "  {} Provider: {} | Model: {}",
        style("✓").green().bold(),
        style(&provider_name).green(),
        style(&model).green()
    );

    maybe_prompt_openai_codex_login(&provider_name, config_path, encrypt_secrets).await?;
    Ok((provider_name, api_key, model, None))
}

#[allow(clippy::too_many_lines)]
pub(super) async fn prompt_advanced_provider_credentials(
    provider_name: &str,
) -> Result<(String, Option<String>)> {
    let mut provider_api_url: Option<String> = None;
    let api_key = if provider_name == "ollama" {
        let use_remote_ollama = Confirm::new()
            .with_prompt("  Use a remote Ollama endpoint (for example Ollama Cloud)?")
            .default(false)
            .interact()?;

        if use_remote_ollama {
            let raw_url: String = Input::new()
                .with_prompt("  Remote Ollama endpoint URL")
                .default("https://ollama.com".into())
                .interact_text()?;

            let normalized_url = normalize_ollama_endpoint_url(&raw_url);
            if normalized_url.is_empty() {
                anyhow::bail!("Remote Ollama endpoint URL cannot be empty.");
            }
            let parsed = reqwest::Url::parse(&normalized_url)
                .context("Remote Ollama endpoint URL must be a valid URL")?;
            if !matches!(parsed.scheme(), "http" | "https") {
                anyhow::bail!("Remote Ollama endpoint URL must use http:// or https://");
            }

            provider_api_url = Some(normalized_url.clone());

            print_bullet(&format!(
                "Remote endpoint configured: {}",
                style(&normalized_url).cyan()
            ));
            if raw_url.trim().trim_end_matches('/') != normalized_url {
                print_bullet("Normalized endpoint to base URL (removed trailing /api).");
            }
            print_bullet(&format!(
                "If you use cloud-only models, append {} to the model ID.",
                style(":cloud").yellow()
            ));

            let key: String = Input::new()
                .with_prompt("  API key for remote Ollama endpoint (or Enter to skip)")
                .allow_empty(true)
                .interact_text()?;

            if key.trim().is_empty() {
                print_bullet(&format!(
                    "No API key provided. Set {} later if required by your endpoint.",
                    style("OLLAMA_API_KEY").yellow()
                ));
            }

            key
        } else {
            print_bullet("Using local Ollama at http://localhost:11434 (no API key needed).");
            String::new()
        }
    } else if matches!(provider_name, "llamacpp" | "llama.cpp") {
        let raw_url: String = Input::new()
            .with_prompt("  llama.cpp server endpoint URL")
            .default("http://localhost:8080/v1".into())
            .interact_text()?;

        let normalized_url = raw_url.trim().trim_end_matches('/').to_string();
        if normalized_url.is_empty() {
            anyhow::bail!("llama.cpp endpoint URL cannot be empty.");
        }
        provider_api_url = Some(normalized_url.clone());

        print_bullet(&format!(
            "Using llama.cpp server endpoint: {}",
            style(&normalized_url).cyan()
        ));
        print_bullet("No API key needed unless your llama.cpp server is started with --api-key.");

        let key: String = Input::new()
            .with_prompt("  API key for llama.cpp server (or Enter to skip)")
            .allow_empty(true)
            .interact_text()?;

        if key.trim().is_empty() {
            print_bullet(&format!(
                "No API key provided. Set {} later only if your server requires authentication.",
                style("LLAMACPP_API_KEY").yellow()
            ));
        }

        key
    } else if provider_name == "sglang" {
        let raw_url: String = Input::new()
            .with_prompt("  SGLang server endpoint URL")
            .default("http://localhost:30000/v1".into())
            .interact_text()?;

        let normalized_url = raw_url.trim().trim_end_matches('/').to_string();
        if normalized_url.is_empty() {
            anyhow::bail!("SGLang endpoint URL cannot be empty.");
        }
        provider_api_url = Some(normalized_url.clone());

        print_bullet(&format!(
            "Using SGLang server endpoint: {}",
            style(&normalized_url).cyan()
        ));
        print_bullet("No API key needed unless your SGLang server requires authentication.");

        let key: String = Input::new()
            .with_prompt("  API key for SGLang server (or Enter to skip)")
            .allow_empty(true)
            .interact_text()?;

        if key.trim().is_empty() {
            print_bullet(&format!(
                "No API key provided. Set {} later only if your server requires authentication.",
                style("SGLANG_API_KEY").yellow()
            ));
        }

        key
    } else if provider_name == "vllm" {
        let raw_url: String = Input::new()
            .with_prompt("  vLLM server endpoint URL")
            .default("http://localhost:8000/v1".into())
            .interact_text()?;

        let normalized_url = raw_url.trim().trim_end_matches('/').to_string();
        if normalized_url.is_empty() {
            anyhow::bail!("vLLM endpoint URL cannot be empty.");
        }
        provider_api_url = Some(normalized_url.clone());

        print_bullet(&format!(
            "Using vLLM server endpoint: {}",
            style(&normalized_url).cyan()
        ));
        print_bullet("No API key needed unless your vLLM server requires authentication.");

        let key: String = Input::new()
            .with_prompt("  API key for vLLM server (or Enter to skip)")
            .allow_empty(true)
            .interact_text()?;

        if key.trim().is_empty() {
            print_bullet(&format!(
                "No API key provided. Set {} later only if your server requires authentication.",
                style("VLLM_API_KEY").yellow()
            ));
        }

        key
    } else if provider_name == "osaurus" {
        let raw_url: String = Input::new()
            .with_prompt("  Osaurus server endpoint URL")
            .default("http://localhost:1337/v1".into())
            .interact_text()?;

        let normalized_url = raw_url.trim().trim_end_matches('/').to_string();
        if normalized_url.is_empty() {
            anyhow::bail!("Osaurus endpoint URL cannot be empty.");
        }
        provider_api_url = Some(normalized_url.clone());

        print_bullet(&format!(
            "Using Osaurus server endpoint: {}",
            style(&normalized_url).cyan()
        ));
        print_bullet("No API key needed unless your Osaurus server requires authentication.");

        let key: String = Input::new()
            .with_prompt("  API key for Osaurus server (or Enter to skip)")
            .allow_empty(true)
            .interact_text()?;

        if key.trim().is_empty() {
            print_bullet(&format!(
                "No API key provided. Set {} later only if your server requires authentication.",
                style("OSAURUS_API_KEY").yellow()
            ));
        }

        key
    } else if canonical_provider_name(provider_name) == "openai-codex" {
        print_bullet("OpenAI Codex uses ChatGPT OAuth, not an API key.");
        print_bullet(
            "TopClaw will start OAuth automatically during onboarding if needed, or you can run `topclaw auth login --provider openai-codex` later.",
        );
        print_bullet("TopClaw will save the provider and model now, then use OAuth after login.");
        String::new()
    } else if canonical_provider_name(provider_name) == "gemini" {
        if crate::providers::gemini::GeminiProvider::has_cli_credentials() {
            print_bullet(&format!(
                "{} Gemini CLI credentials detected! You can skip the API key.",
                style("✓").green().bold()
            ));
            print_bullet("TopClaw will reuse your existing Gemini CLI authentication.");
            println!();

            let use_cli: bool = Confirm::new()
                .with_prompt("  Use existing Gemini CLI authentication?")
                .default(true)
                .interact()?;

            if use_cli {
                println!(
                    "  {} Using Gemini CLI OAuth tokens",
                    style("✓").green().bold()
                );
                String::new()
            } else {
                print_bullet("Get your API key at: https://aistudio.google.com/app/apikey");
                Input::new()
                    .with_prompt("  Paste your Gemini API key")
                    .allow_empty(true)
                    .interact_text()?
            }
        } else if std::env::var("GEMINI_API_KEY").is_ok() {
            print_bullet(&format!(
                "{} GEMINI_API_KEY environment variable detected!",
                style("✓").green().bold()
            ));
            String::new()
        } else {
            print_bullet("Get your API key at: https://aistudio.google.com/app/apikey");
            print_bullet("Or run `gemini` CLI to authenticate (tokens will be reused).");
            println!();

            Input::new()
                .with_prompt("  Paste your Gemini API key (or press Enter to skip)")
                .allow_empty(true)
                .interact_text()?
        }
    } else if canonical_provider_name(provider_name) == "anthropic" {
        if std::env::var("ANTHROPIC_OAUTH_TOKEN").is_ok() {
            print_bullet(&format!(
                "{} ANTHROPIC_OAUTH_TOKEN environment variable detected!",
                style("✓").green().bold()
            ));
            String::new()
        } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            print_bullet(&format!(
                "{} ANTHROPIC_API_KEY environment variable detected!",
                style("✓").green().bold()
            ));
            String::new()
        } else {
            print_bullet(&format!(
                "Get your API key at: {}",
                style("https://console.anthropic.com/settings/keys")
                    .cyan()
                    .underlined()
            ));
            print_bullet("Or run `claude setup-token` to get an OAuth setup-token.");
            println!();

            let key: String = Input::new()
                .with_prompt("  Paste your API key or setup-token (or press Enter to skip)")
                .allow_empty(true)
                .interact_text()?;

            if key.is_empty() {
                print_bullet(&format!(
                    "Skipped. Set {} or {} or edit config.toml later.",
                    style("ANTHROPIC_API_KEY").yellow(),
                    style("ANTHROPIC_OAUTH_TOKEN").yellow()
                ));
            }

            key
        }
    } else if canonical_provider_name(provider_name) == "qwen-code" {
        if std::env::var("QWEN_OAUTH_TOKEN").is_ok() {
            print_bullet(&format!(
                "{} QWEN_OAUTH_TOKEN environment variable detected!",
                style("✓").green().bold()
            ));
            "qwen-oauth".to_string()
        } else {
            print_bullet(
                "Qwen Code OAuth credentials are usually stored in ~/.qwen/oauth_creds.json.",
            );
            print_bullet(
                "Run `qwen` once and complete OAuth login to populate cached credentials.",
            );
            print_bullet("You can also set QWEN_OAUTH_TOKEN directly.");
            println!();

            let key: String = Input::new()
                .with_prompt(
                    "  Paste your Qwen OAuth token (or press Enter to auto-detect cached OAuth)",
                )
                .allow_empty(true)
                .interact_text()?;

            if key.trim().is_empty() {
                print_bullet(&format!(
                    "Using OAuth auto-detection. Set {} and optional {} if needed.",
                    style("QWEN_OAUTH_TOKEN").yellow(),
                    style("QWEN_OAUTH_RESOURCE_URL").yellow()
                ));
                "qwen-oauth".to_string()
            } else {
                key
            }
        }
    } else {
        let key_url = if is_moonshot_alias(provider_name)
            || canonical_provider_name(provider_name) == "kimi-code"
        {
            "https://platform.moonshot.cn/console/api-keys"
        } else if canonical_provider_name(provider_name) == "qwen-code" {
            "https://qwen.readthedocs.io/en/latest/getting_started/installation.html"
        } else if is_glm_cn_alias(provider_name) || is_zai_cn_alias(provider_name) {
            "https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys"
        } else if is_glm_alias(provider_name) || is_zai_alias(provider_name) {
            "https://platform.z.ai/"
        } else if is_minimax_alias(provider_name) {
            "https://www.minimaxi.com/user-center/basic-information"
        } else if is_qwen_alias(provider_name) {
            "https://help.aliyun.com/zh/model-studio/developer-reference/get-api-key"
        } else if is_qianfan_alias(provider_name) {
            "https://cloud.baidu.com/doc/WENXINWORKSHOP/s/7lm0vxo78"
        } else {
            match provider_name {
                "openrouter" => "https://openrouter.ai/keys",
                "openai" => "https://platform.openai.com/api-keys",
                "venice" => "https://venice.ai/settings/api",
                "groq" => "https://console.groq.com/keys",
                "mistral" => "https://console.mistral.ai/api-keys",
                "deepseek" => "https://platform.deepseek.com/api_keys",
                "together-ai" => "https://api.together.xyz/settings/api-keys",
                "fireworks" => "https://fireworks.ai/account/api-keys",
                "novita" => "https://novita.ai/settings/key-management",
                "perplexity" => "https://www.perplexity.ai/settings/api",
                "xai" => "https://console.x.ai",
                "cohere" => "https://dashboard.cohere.com/api-keys",
                "vercel" => "https://vercel.com/account/tokens",
                "cloudflare" => "https://dash.cloudflare.com/profile/api-tokens",
                "nvidia" | "nvidia-nim" | "build.nvidia.com" => "https://build.nvidia.com/",
                "bedrock" => "https://console.aws.amazon.com/iam",
                "gemini" => "https://aistudio.google.com/app/apikey",
                "astrai" => "https://as-trai.com",
                _ => "",
            }
        };

        println!();
        if matches!(provider_name, "bedrock" | "aws-bedrock") {
            print_bullet("Bedrock uses AWS credentials (not a single API key).");
            print_bullet(&format!(
                "Set {} and {} environment variables.",
                style("AWS_ACCESS_KEY_ID").yellow(),
                style("AWS_SECRET_ACCESS_KEY").yellow(),
            ));
            print_bullet(&format!(
                "Optionally set {} for the region (default: us-east-1).",
                style("AWS_REGION").yellow(),
            ));
            if !key_url.is_empty() {
                print_bullet(&format!(
                    "Manage IAM credentials at: {}",
                    style(key_url).cyan().underlined()
                ));
            }
            println!();
            String::new()
        } else {
            if !key_url.is_empty() {
                print_bullet(&format!(
                    "Get your API key at: {}",
                    style(key_url).cyan().underlined()
                ));
            }
            print_bullet("You can also set it later via env var or config file.");
            println!();

            let key: String = Input::new()
                .with_prompt("  Paste your API key (or press Enter to skip)")
                .allow_empty(true)
                .interact_text()?;

            if key.is_empty() {
                let env_var = provider_env_var(provider_name);
                print_bullet(&format!(
                    "Skipped. Set {} or edit config.toml later.",
                    style(env_var).yellow()
                ));
            }

            key
        }
    };

    Ok((api_key, provider_api_url))
}
