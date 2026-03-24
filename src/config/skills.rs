use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Skills loading configuration (`[skills]` section).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillsPromptInjectionMode {
    /// Inline only compact skill metadata (name/description/location) and load details on demand.
    #[default]
    Compact,
    /// Inline full skill instructions and tool metadata into the system prompt.
    Full,
}

pub fn parse_skills_prompt_injection_mode(raw: &str) -> Option<SkillsPromptInjectionMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "full" => Some(SkillsPromptInjectionMode::Full),
        "compact" => Some(SkillsPromptInjectionMode::Compact),
        _ => None,
    }
}

/// Skills loading configuration (`[skills]` section).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SkillsConfig {
    /// Enable loading and syncing the community open-skills repository.
    /// Default: `false` (opt-in).
    #[serde(default)]
    pub open_skills_enabled: bool,
    /// Optional path to a local open-skills repository.
    /// If unset, defaults to `$HOME/open-skills` when enabled.
    #[serde(default)]
    pub open_skills_dir: Option<String>,
    /// Controls how skills are injected into the system prompt.
    /// `compact` (default) keeps context small and loads skills on demand.
    /// `full` preserves legacy behavior as an opt-in.
    #[serde(default)]
    pub prompt_injection_mode: SkillsPromptInjectionMode,
}
