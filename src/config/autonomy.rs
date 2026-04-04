use crate::security::{AutonomyLevel, ShellRedirectPolicy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Natural-language behavior for non-CLI approval-management commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NonCliNaturalLanguageApprovalMode {
    /// Directly apply approval-management commands when authorized.
    #[default]
    Direct,
    /// Create a pending request that requires explicit confirmation.
    RequestConfirm,
    /// Ignore natural-language approval commands (slash commands only).
    Disabled,
}

/// Controls what the agent is allowed to do: shell commands, filesystem access,
/// risk approval gates, and per-policy budgets.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutonomyConfig {
    /// Autonomy level: `read_only`, `supervised` (default), or `full`.
    pub level: AutonomyLevel,
    /// Restrict absolute filesystem paths to workspace-relative references. Default: `true`.
    /// Resolved paths outside the workspace still require `allowed_roots`.
    pub workspace_only: bool,
    /// Allowlist of executable names permitted for shell execution.
    pub allowed_commands: Vec<String>,
    /// Explicit path denylist. Default includes system-critical paths and sensitive dotdirs.
    pub forbidden_paths: Vec<String>,
    /// Maximum actions allowed per hour per policy. Default: `20`.
    pub max_actions_per_hour: u32,
    /// Maximum cost per day in cents per policy. Default: `500`.
    pub max_cost_per_day_cents: u32,
    /// Require explicit approval for medium-risk shell commands.
    #[serde(default = "default_true")]
    pub require_approval_for_medium_risk: bool,
    /// Block high-risk shell commands even if allowlisted.
    #[serde(default = "default_true")]
    pub block_high_risk_commands: bool,
    /// Redirect handling mode for shell commands.
    #[serde(default)]
    pub shell_redirect_policy: ShellRedirectPolicy,
    /// Additional environment variables allowed for shell tool subprocesses.
    #[serde(default)]
    pub shell_env_passthrough: Vec<String>,
    /// Tools that never require approval (biased toward read-only investigation/navigation).
    #[serde(default = "default_auto_approve")]
    pub auto_approve: Vec<String>,
    /// Tools that always require interactive approval, even after "Always".
    #[serde(default = "default_always_ask")]
    pub always_ask: Vec<String>,
    /// Extra directory roots the agent may read/write outside the workspace.
    #[serde(default)]
    pub allowed_roots: Vec<String>,
    /// Optional denylist for non-CLI channels (e.g. Telegram, Discord). Empty by default.
    #[serde(default = "default_non_cli_excluded_tools")]
    pub non_cli_excluded_tools: Vec<String>,
    /// Optional allowlist for who can manage non-CLI approval commands.
    #[serde(default)]
    pub non_cli_approval_approvers: Vec<String>,
    /// Natural-language handling mode for non-CLI approval-management commands.
    #[serde(default)]
    pub non_cli_natural_language_approval_mode: NonCliNaturalLanguageApprovalMode,
    /// Optional per-channel override for natural-language approval mode.
    #[serde(default)]
    pub non_cli_natural_language_approval_mode_by_channel:
        HashMap<String, NonCliNaturalLanguageApprovalMode>,
}

fn default_true() -> bool {
    true
}

pub(crate) fn default_auto_approve() -> Vec<String> {
    #[cfg(feature = "rag-pdf")]
    let mut tools = vec![
        "file_read".to_string(),
        "glob_search".to_string(),
        "content_search".to_string(),
        "lossless_search".to_string(),
        "lossless_describe".to_string(),
        "memory_recall".to_string(),
    ];
    #[cfg(not(feature = "rag-pdf"))]
    let tools = vec![
        "file_read".to_string(),
        "glob_search".to_string(),
        "content_search".to_string(),
        "lossless_search".to_string(),
        "lossless_describe".to_string(),
        "memory_recall".to_string(),
    ];
    #[cfg(feature = "rag-pdf")]
    tools.push("pdf_read".to_string());
    tools
}

pub(crate) fn default_always_ask() -> Vec<String> {
    vec![]
}

pub(crate) fn default_non_cli_excluded_tools() -> Vec<String> {
    ["proxy_config", "model_routing_config"]
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect()
}

pub(crate) fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            workspace_only: true,
            allowed_commands: default_allowed_commands(),
            forbidden_paths: vec![
                "/etc".into(),
                "/root".into(),
                "/home".into(),
                "/usr".into(),
                "/bin".into(),
                "/sbin".into(),
                "/lib".into(),
                "/opt".into(),
                "/boot".into(),
                "/dev".into(),
                "/proc".into(),
                "/sys".into(),
                "/var".into(),
                "/tmp".into(),
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
                "~/.config/secrets".into(),
            ],
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            shell_redirect_policy: ShellRedirectPolicy::Block,
            shell_env_passthrough: vec![],
            auto_approve: default_auto_approve(),
            always_ask: default_always_ask(),
            allowed_roots: Vec::new(),
            non_cli_excluded_tools: default_non_cli_excluded_tools(),
            non_cli_approval_approvers: Vec::new(),
            non_cli_natural_language_approval_mode: NonCliNaturalLanguageApprovalMode::default(),
            non_cli_natural_language_approval_mode_by_channel: HashMap::new(),
        }
    }
}

fn default_allowed_commands() -> Vec<String> {
    [
        // File navigation and inspection
        "ls", "cat", "head", "tail", "wc", "less", "more", "file", "stat", "tree", "du", "df",
        // File manipulation (approval-gated for medium/high-risk via security policy)
        "touch", "mkdir", "cp", "mv", "rm", "ln", "chmod", // Search
        "grep", "find", "rg", "ag", "fd",  // Git workflows
        "git", // Build and package managers
        "cargo", "rustc", "npm", "pnpm", "yarn", "pip", "pip3", "python3", "python", "node", "go",
        "make", "cmake", // Common utilities
        "echo", "date", "pwd", "whoami", "uname", "hostname", "id", "env", "printenv", "sort",
        "uniq", "cut", "tr", "awk", "sed", "xargs", // Network (approval-gated)
        "curl", "wget", // Compression
        "tar", "gzip", "gunzip", "zip", "unzip", // Diff and patch
        "diff", "patch", // Line count and code analysis
        "cloc", "tokei", // Process inspection
        "ps", "top", // Container (approval-gated)
        "docker", "podman",
    ]
    .into_iter()
    .map(std::string::ToString::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_auto_approve_favors_read_only_navigation_tools() {
        let defaults = default_auto_approve();
        for tool in [
            "file_read",
            "glob_search",
            "content_search",
            "lossless_search",
            "lossless_describe",
            "memory_recall",
        ] {
            assert!(
                defaults.contains(&tool.to_string()),
                "expected `{tool}` in default auto-approve set"
            );
        }
        #[cfg(feature = "rag-pdf")]
        assert!(defaults.contains(&"pdf_read".to_string()));
        #[cfg(not(feature = "rag-pdf"))]
        assert!(!defaults.contains(&"pdf_read".to_string()));
        assert!(!defaults.contains(&"shell".to_string()));
        assert!(!defaults.contains(&"file_write".to_string()));
    }

    #[test]
    fn default_non_cli_exclusions_focus_channels_on_work_tools() {
        let defaults = default_non_cli_excluded_tools();
        assert!(defaults.contains(&"proxy_config".to_string()));
        assert!(defaults.contains(&"model_routing_config".to_string()));
        assert_eq!(AutonomyConfig::default().non_cli_excluded_tools, defaults);
    }
}
