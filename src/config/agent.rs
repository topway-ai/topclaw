use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const fn default_agent_max_tool_iterations() -> usize {
    100
}

const fn default_agent_max_history_messages() -> usize {
    50
}

fn default_agent_tool_dispatcher() -> String {
    "auto".into()
}

fn default_min_relevance_score() -> f64 {
    0.4
}

/// Agent orchestration configuration (`[agent]` section).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfig {
    /// When true: bootstrap_max_chars=6000, rag_chunk_limit=2. Use for 13B or smaller models.
    #[serde(default)]
    pub compact_context: bool,
    /// Maximum tool-call loop turns per user message. Default: `100`.
    /// Setting to `0` falls back to the safe default of `100`.
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: usize,
    /// Minimum relevance score threshold for memory context loading. Default: `0.4`.
    #[serde(default = "default_min_relevance_score")]
    pub min_relevance_score: f64,
    /// Maximum conversation history messages retained per session. Default: `50`.
    #[serde(default = "default_agent_max_history_messages")]
    pub max_history_messages: usize,
    /// Enable parallel tool execution within a single iteration. Default: `false`.
    #[serde(default)]
    pub parallel_tools: bool,
    /// Tool dispatch strategy (e.g. `"auto"`). Default: `"auto"`.
    #[serde(default = "default_agent_tool_dispatcher")]
    pub tool_dispatcher: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            compact_context: false,
            max_tool_iterations: default_agent_max_tool_iterations(),
            min_relevance_score: default_min_relevance_score(),
            max_history_messages: default_agent_max_history_messages(),
            parallel_tools: false,
            tool_dispatcher: default_agent_tool_dispatcher(),
        }
    }
}
