//! Static registry of OpenAI-compatible provider endpoints.
//!
//! Most providers in the factory are thin wrappers around
//! [`OpenAiCompatibleProvider`] with different base URLs and minor flag
//! variations.  This module encodes those parameters in a lookup table,
//! replacing dozens of match arms with a single registry query.
//!
//! Providers that need custom implementations (Anthropic, Gemini, Ollama,
//! OpenAI, OpenRouter, Telnyx, Bedrock, Copilot, OpenAI-Codex) are **not**
//! in this registry — they keep their dedicated match arms in the factory.
//!
//! Regional providers with dynamic base URLs (Moonshot, GLM, MiniMax, Qwen,
//! Z.AI, Doubao, Qianfan) also stay outside the registry because their URL
//! depends on alias resolution at call time.

use super::compatible::AuthStyle;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Descriptor for an OpenAI-compatible provider endpoint.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct RegistryEntry {
    /// Display name shown in logs and diagnostics.
    pub display_name: &'static str,
    /// Base URL for the OpenAI-compatible API.
    pub base_url: &'static str,
    /// Authentication style (almost always Bearer).
    pub auth_style: AuthStyle,
    /// Whether the provider supports vision/multimodal.
    pub supports_vision: bool,
    /// Whether to try `/responses` before `/chat/completions`.
    pub supports_responses_fallback: bool,
    /// Whether to merge system messages into user messages.
    pub merge_system_into_user: bool,
    /// Optional custom User-Agent header.
    pub user_agent: Option<&'static str>,
    /// Default base URL for local providers when no api_url override is given.
    /// When set, `base_url` is this default and can be overridden by the user's
    /// `api_url` config.
    pub is_local: bool,
    /// Default placeholder credential for local providers that don't need auth.
    pub default_key: Option<&'static str>,
    /// Environment variable names to check for credentials (in priority order).
    pub env_key_names: &'static [&'static str],
}

/// All accepted name strings that map to a registry entry.
/// Format: `(alias, canonical_key)` where `canonical_key` is the key in `REGISTRY`.
static ALIAS_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for (canonical, entry_aliases) in ALIASES {
        for alias in *entry_aliases {
            m.insert(*alias, *canonical);
        }
        m.insert(*canonical, *canonical);
    }
    m
});

/// Alias definitions: `(canonical_name, &[aliases])`.
const ALIASES: &[(&str, &[&str])] = &[
    ("venice", &[]),
    ("vercel", &["vercel-ai"]),
    ("cloudflare", &["cloudflare-ai"]),
    ("synthetic", &[]),
    ("opencode", &["opencode-zen"]),
    ("groq", &[]),
    ("mistral", &[]),
    ("xai", &["grok"]),
    ("deepseek", &[]),
    ("together", &["together-ai"]),
    ("fireworks", &["fireworks-ai"]),
    ("perplexity", &[]),
    ("cohere", &[]),
    ("hunyuan", &["tencent"]),
    ("nvidia", &["nvidia-nim", "build.nvidia.com"]),
    ("astrai", &[]),
    ("lmstudio", &["lm-studio"]),
    ("llamacpp", &["llama.cpp"]),
    ("sglang", &[]),
    ("vllm", &[]),
    ("litellm", &["lite-llm"]),
    ("osaurus", &[]),
];

/// Static registry of OpenAI-compatible providers.
static REGISTRY: LazyLock<HashMap<&'static str, RegistryEntry>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // ── Cloud API providers ──────────────────────────────────────────
    m.insert(
        "venice",
        RegistryEntry::cloud("Venice", "https://api.venice.ai", &["VENICE_API_KEY"]),
    );
    m.insert(
        "vercel",
        RegistryEntry::cloud(
            "Vercel AI Gateway",
            "https://ai-gateway.vercel.sh/v1",
            &["VERCEL_API_KEY"],
        ),
    );
    m.insert(
        "cloudflare",
        RegistryEntry::cloud(
            "Cloudflare AI Gateway",
            "https://gateway.ai.cloudflare.com/v1",
            &["CLOUDFLARE_API_KEY"],
        ),
    );
    m.insert(
        "synthetic",
        RegistryEntry::cloud(
            "Synthetic",
            "https://api.synthetic.new/openai/v1",
            &["SYNTHETIC_API_KEY"],
        ),
    );
    m.insert(
        "opencode",
        RegistryEntry::cloud(
            "OpenCode Zen",
            "https://opencode.ai/zen/v1",
            &["OPENCODE_API_KEY"],
        ),
    );
    m.insert(
        "groq",
        RegistryEntry::cloud("Groq", "https://api.groq.com/openai/v1", &["GROQ_API_KEY"]),
    );
    m.insert(
        "mistral",
        RegistryEntry::cloud("Mistral", "https://api.mistral.ai/v1", &["MISTRAL_API_KEY"]),
    );
    m.insert(
        "xai",
        RegistryEntry::cloud("xAI", "https://api.x.ai", &["XAI_API_KEY"]),
    );
    m.insert(
        "deepseek",
        RegistryEntry::cloud(
            "DeepSeek",
            "https://api.deepseek.com",
            &["DEEPSEEK_API_KEY"],
        ),
    );
    m.insert(
        "together",
        RegistryEntry::cloud(
            "Together AI",
            "https://api.together.xyz",
            &["TOGETHER_API_KEY"],
        ),
    );
    m.insert(
        "fireworks",
        RegistryEntry::cloud(
            "Fireworks AI",
            "https://api.fireworks.ai/inference/v1",
            &["FIREWORKS_API_KEY"],
        ),
    );
    m.insert(
        "perplexity",
        RegistryEntry::cloud(
            "Perplexity",
            "https://api.perplexity.ai",
            &["PERPLEXITY_API_KEY"],
        ),
    );
    m.insert(
        "cohere",
        RegistryEntry::cloud(
            "Cohere",
            "https://api.cohere.com/compatibility",
            &["COHERE_API_KEY"],
        ),
    );
    m.insert(
        "hunyuan",
        RegistryEntry::cloud(
            "Hunyuan",
            "https://api.hunyuan.cloud.tencent.com/v1",
            &["HUNYUAN_API_KEY"],
        ),
    );
    m.insert(
        "astrai",
        RegistryEntry::cloud("Astrai", "https://as-trai.com/v1", &["ASTRAI_API_KEY"]),
    );

    // NVIDIA: cloud but no /responses fallback
    let mut nvidia = RegistryEntry::cloud(
        "NVIDIA NIM",
        "https://integrate.api.nvidia.com/v1",
        &["NVIDIA_API_KEY"],
    );
    nvidia.supports_responses_fallback = false;
    m.insert("nvidia", nvidia);

    // ── Local inference servers ──────────────────────────────────────
    m.insert(
        "lmstudio",
        RegistryEntry::local(
            "LM Studio",
            "http://localhost:1234/v1",
            Some("lm-studio"),
            &[],
        ),
    );
    m.insert(
        "llamacpp",
        RegistryEntry::local(
            "llama.cpp",
            "http://localhost:8080/v1",
            Some("llama.cpp"),
            &["LLAMACPP_API_KEY"],
        ),
    );
    m.insert(
        "sglang",
        RegistryEntry::local(
            "SGLang",
            "http://localhost:30000/v1",
            None,
            &["SGLANG_API_KEY"],
        ),
    );
    m.insert(
        "vllm",
        RegistryEntry::local("vLLM", "http://localhost:8000/v1", None, &["VLLM_API_KEY"]),
    );
    m.insert(
        "litellm",
        RegistryEntry::local(
            "LiteLLM",
            "http://localhost:4000/v1",
            None,
            &["LITELLM_API_KEY"],
        ),
    );
    m.insert(
        "osaurus",
        RegistryEntry::local(
            "Osaurus",
            "http://localhost:1337/v1",
            Some("osaurus"),
            &["OSAURUS_API_KEY"],
        ),
    );

    m
});

impl RegistryEntry {
    /// Create a standard cloud provider entry (Bearer auth, default flags).
    const fn cloud(
        display_name: &'static str,
        base_url: &'static str,
        env_key_names: &'static [&'static str],
    ) -> Self {
        Self {
            display_name,
            base_url,
            auth_style: AuthStyle::Bearer,
            supports_vision: false,
            supports_responses_fallback: true,
            merge_system_into_user: false,
            user_agent: None,
            is_local: false,
            default_key: None,
            env_key_names,
        }
    }

    /// Create a local inference server entry (Bearer auth, overridable URL).
    const fn local(
        display_name: &'static str,
        base_url: &'static str,
        default_key: Option<&'static str>,
        env_key_names: &'static [&'static str],
    ) -> Self {
        Self {
            display_name,
            base_url,
            auth_style: AuthStyle::Bearer,
            supports_vision: false,
            supports_responses_fallback: true,
            merge_system_into_user: false,
            user_agent: None,
            is_local: true,
            default_key,
            env_key_names,
        }
    }
}

/// Look up a provider name (or alias) in the static registry.
pub fn lookup(name: &str) -> Option<&RegistryEntry> {
    let canonical = ALIAS_MAP.get(name)?;
    REGISTRY.get(canonical)
}

/// Look up a provider name in the alias map and return the canonical key.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    ALIAS_MAP.get(name).copied()
}

/// Resolve environment variable credential for a registry entry.
pub fn resolve_env_credential(entry: &RegistryEntry) -> Option<String> {
    for env_var in entry.env_key_names {
        if let Ok(value) = std::env::var(env_var) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_canonical_name() {
        let entry = lookup("groq").expect("groq should be in registry");
        assert_eq!(entry.display_name, "Groq");
        assert_eq!(entry.base_url, "https://api.groq.com/openai/v1");
    }

    #[test]
    fn lookup_alias() {
        let entry = lookup("grok").expect("grok alias should resolve to xai");
        assert_eq!(entry.display_name, "xAI");
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("nonexistent-provider").is_none());
    }

    #[test]
    fn local_provider_has_default_key() {
        let entry = lookup("lmstudio").expect("lmstudio should be in registry");
        assert!(entry.is_local);
        assert_eq!(entry.default_key, Some("lm-studio"));
    }

    #[test]
    fn alias_maps_both_directions() {
        // "vercel-ai" → "vercel"
        assert_eq!(canonical_name("vercel-ai"), Some("vercel"));
        assert_eq!(canonical_name("vercel"), Some("vercel"));
    }

    #[test]
    fn all_aliases_resolve() {
        for (canonical, aliases) in ALIASES {
            assert!(
                REGISTRY.contains_key(canonical),
                "canonical name '{canonical}' not in REGISTRY"
            );
            for alias in *aliases {
                assert!(
                    lookup(alias).is_some(),
                    "alias '{alias}' for '{canonical}' does not resolve"
                );
            }
        }
    }
}
