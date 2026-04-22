//! Reusable in-process agent type and builder.
//!
//! [`Agent`] is the highest-level library entry point for driving the standard
//! TopClaw LLM loop from Rust. It owns a provider, tool registry, memory
//! backend, observer, prompt builder, and conversation history.
//!
//! For most applications, [`Agent::from_config`] is the easiest constructor.
//! Use [`AgentBuilder`] when you need custom provider, tool, or memory wiring.
/// Inline memory context loader (formerly in memory_loader.rs).
/// Loads relevant memory entries for the current user message.
async fn load_memory_context(
    memory: &dyn Memory,
    user_message: &str,
    limit: usize,
    min_relevance_score: f64,
) -> Result<String> {
    let entries = memory.recall(user_message, limit, None).await?;
    if entries.is_empty() {
        return Ok(String::new());
    }

    let mut context = String::from("[Memory context]\n");
    for entry in entries {
        if memory::is_assistant_autosave_key(&entry.key) {
            continue;
        }
        if let Some(score) = entry.score {
            if score < min_relevance_score {
                continue;
            }
        }
        let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
    }

    if context == "[Memory context]\n" {
        return Ok(String::new());
    }

    context.push('\n');
    Ok(context)
}

use crate::agent::dispatcher::{
    NativeToolDispatcher, ParsedToolCall, ToolDispatcher, ToolExecutionResult, XmlToolDispatcher,
};
use crate::agent::prompt::{PromptContext, SystemPromptBuilder};
use crate::agent::research;
use crate::agent::wiring;
use crate::config::{Config, ResearchPhaseConfig};
use crate::memory::{self, Memory, MemoryCategory};
use crate::observability::{Observer, ObserverEvent};
use crate::providers::{self, ChatMessage, ChatRequest, ConversationMessage, Provider};
use crate::tools::{Tool, ToolSpec};
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Write as IoWrite;
use std::sync::Arc;
use std::time::Instant;

/// Reusable agent instance that preserves conversation history across turns.
pub struct Agent {
    provider: Box<dyn Provider>,
    tools: Vec<Box<dyn Tool>>,
    tool_specs: Vec<ToolSpec>,
    memory: Arc<dyn Memory>,
    observer: Arc<dyn Observer>,
    prompt_builder: SystemPromptBuilder,
    tool_dispatcher: Box<dyn ToolDispatcher>,
    config: crate::config::AgentConfig,
    model_name: String,
    temperature: f64,
    workspace_dir: std::path::PathBuf,
    #[allow(dead_code)]
    identity_config: crate::config::IdentityConfig,
    skills: Vec<crate::skills::Skill>,
    skills_prompt_mode: crate::config::SkillsPromptInjectionMode,
    auto_save: bool,
    history: Vec<ConversationMessage>,
    classification_config: crate::config::QueryClassificationConfig,
    available_hints: Vec<String>,
    route_model_by_hint: HashMap<String, String>,
    research_config: ResearchPhaseConfig,
}

/// Builder for constructing an [`Agent`] from explicit dependencies.
///
/// Use this when embedding TopClaw into another Rust application and you want
/// to supply custom provider, tool, memory, or observer implementations.
pub struct AgentBuilder {
    provider: Option<Box<dyn Provider>>,
    tools: Option<Vec<Box<dyn Tool>>>,
    memory: Option<Arc<dyn Memory>>,
    observer: Option<Arc<dyn Observer>>,
    prompt_builder: Option<SystemPromptBuilder>,
    tool_dispatcher: Option<Box<dyn ToolDispatcher>>,
    config: Option<crate::config::AgentConfig>,
    model_name: Option<String>,
    temperature: Option<f64>,
    workspace_dir: Option<std::path::PathBuf>,
    identity_config: Option<crate::config::IdentityConfig>,
    skills: Option<Vec<crate::skills::Skill>>,
    skills_prompt_mode: Option<crate::config::SkillsPromptInjectionMode>,
    auto_save: Option<bool>,
    classification_config: Option<crate::config::QueryClassificationConfig>,
    available_hints: Option<Vec<String>>,
    route_model_by_hint: Option<HashMap<String, String>>,
    research_config: Option<ResearchPhaseConfig>,
}

impl AgentBuilder {
    /// Create an empty builder.
    pub fn new() -> Self {
        Self {
            provider: None,
            tools: None,
            memory: None,
            observer: None,
            prompt_builder: None,
            tool_dispatcher: None,
            config: None,
            model_name: None,
            temperature: None,
            workspace_dir: None,
            identity_config: None,
            skills: None,
            skills_prompt_mode: None,
            auto_save: None,
            classification_config: None,
            available_hints: None,
            route_model_by_hint: None,
            research_config: None,
        }
    }

    pub fn provider(mut self, provider: Box<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn prompt_builder(mut self, prompt_builder: SystemPromptBuilder) -> Self {
        self.prompt_builder = Some(prompt_builder);
        self
    }

    pub fn tool_dispatcher(mut self, tool_dispatcher: Box<dyn ToolDispatcher>) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    pub fn config(mut self, config: crate::config::AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn model_name(mut self, model_name: String) -> Self {
        self.model_name = Some(model_name);
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn workspace_dir(mut self, workspace_dir: std::path::PathBuf) -> Self {
        self.workspace_dir = Some(workspace_dir);
        self
    }

    pub fn identity_config(mut self, identity_config: crate::config::IdentityConfig) -> Self {
        self.identity_config = Some(identity_config);
        self
    }

    pub fn skills(mut self, skills: Vec<crate::skills::Skill>) -> Self {
        self.skills = Some(skills);
        self
    }

    pub fn skills_prompt_mode(
        mut self,
        skills_prompt_mode: crate::config::SkillsPromptInjectionMode,
    ) -> Self {
        self.skills_prompt_mode = Some(skills_prompt_mode);
        self
    }

    pub fn auto_save(mut self, auto_save: bool) -> Self {
        self.auto_save = Some(auto_save);
        self
    }

    pub fn classification_config(
        mut self,
        classification_config: crate::config::QueryClassificationConfig,
    ) -> Self {
        self.classification_config = Some(classification_config);
        self
    }

    pub fn available_hints(mut self, available_hints: Vec<String>) -> Self {
        self.available_hints = Some(available_hints);
        self
    }

    pub fn route_model_by_hint(mut self, route_model_by_hint: HashMap<String, String>) -> Self {
        self.route_model_by_hint = Some(route_model_by_hint);
        self
    }

    pub fn research_config(mut self, research_config: ResearchPhaseConfig) -> Self {
        self.research_config = Some(research_config);
        self
    }

    /// Finish constructing the agent.
    ///
    /// # Errors
    ///
    /// Returns an error when required dependencies such as the provider, tools,
    /// memory backend, or tool dispatcher were not supplied.
    pub fn build(self) -> Result<Agent> {
        let tools = self
            .tools
            .ok_or_else(|| anyhow::anyhow!("tools are required"))?;
        let tool_specs = tools.iter().map(|tool| tool.spec()).collect();

        Ok(Agent {
            provider: self
                .provider
                .ok_or_else(|| anyhow::anyhow!("provider is required"))?,
            tools,
            tool_specs,
            memory: self
                .memory
                .ok_or_else(|| anyhow::anyhow!("memory is required"))?,
            observer: self
                .observer
                .ok_or_else(|| anyhow::anyhow!("observer is required"))?,
            prompt_builder: self
                .prompt_builder
                .unwrap_or_else(SystemPromptBuilder::with_defaults),
            tool_dispatcher: self
                .tool_dispatcher
                .ok_or_else(|| anyhow::anyhow!("tool_dispatcher is required"))?,
            config: self.config.unwrap_or_default(),
            model_name: self
                .model_name
                .unwrap_or_else(|| "anthropic/claude-sonnet-4-20250514".into()),
            temperature: self.temperature.unwrap_or(0.7),
            workspace_dir: self
                .workspace_dir
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
            identity_config: self.identity_config.unwrap_or_default(),
            skills: self.skills.unwrap_or_default(),
            skills_prompt_mode: self.skills_prompt_mode.unwrap_or_default(),
            auto_save: self.auto_save.unwrap_or(false),
            history: Vec::new(),
            classification_config: self.classification_config.unwrap_or_default(),
            available_hints: self.available_hints.unwrap_or_default(),
            route_model_by_hint: self.route_model_by_hint.unwrap_or_default(),
            research_config: self.research_config.unwrap_or_default(),
        })
    }
}

impl Agent {
    /// Create a builder for explicit dependency injection.
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    /// Return the in-memory conversation history tracked by this instance.
    pub fn history(&self) -> &[ConversationMessage] {
        &self.history
    }

    /// Remove all retained conversation history from this instance.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    fn resolve_model_name_from_config(config: &Config) -> String {
        config
            .default_model
            .as_deref()
            .unwrap_or(crate::providers::DEFAULT_PROVIDER_MODEL)
            .to_string()
    }

    fn build_provider_from_config(config: &Config, model_name: &str) -> Result<Box<dyn Provider>> {
        let provider_name = config
            .default_provider
            .as_deref()
            .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME);
        providers::create_routed_provider_with_options(
            provider_name,
            config.api_key.as_deref(),
            config.api_url.as_deref(),
            &config.reliability,
            &config.model_routes,
            model_name,
            &providers::ProviderRuntimeOptions::from_config(config),
        )
    }

    fn build_tool_dispatcher_from_config(
        config: &Config,
        provider: &dyn Provider,
    ) -> Box<dyn ToolDispatcher> {
        match config.agent.tool_dispatcher.as_str() {
            "native" => Box::new(NativeToolDispatcher),
            "xml" => Box::new(XmlToolDispatcher),
            _ if provider.supports_native_tools() => Box::new(NativeToolDispatcher),
            _ => Box::new(XmlToolDispatcher),
        }
    }

    fn build_model_route_index_from_config(
        config: &Config,
    ) -> (HashMap<String, String>, Vec<String>) {
        let route_model_by_hint: HashMap<String, String> = config
            .model_routes
            .iter()
            .map(|route| (route.hint.clone(), route.model.clone()))
            .collect();
        let available_hints = route_model_by_hint.keys().cloned().collect();
        (route_model_by_hint, available_hints)
    }

    /// Construct a fully wired agent from the standard TopClaw config.
    ///
    /// This performs the same high-level dependency wiring used by the CLI:
    /// runtime creation, security policy setup, memory backend selection,
    /// provider routing, tool registration, and skill loading.
    pub fn from_config(config: &Config) -> Result<Self> {
        let observer = wiring::build_observer(config);
        let execution = wiring::build_execution_support(config, &config.embedding_routes)?;
        let model_name = Self::resolve_model_name_from_config(config);
        let provider = Self::build_provider_from_config(config, &model_name)?;
        let tool_dispatcher = Self::build_tool_dispatcher_from_config(config, provider.as_ref());
        let (route_model_by_hint, available_hints) =
            Self::build_model_route_index_from_config(config);
        let skills = wiring::load_skills(config);

        Agent::builder()
            .provider(provider)
            .tools(execution.tools)
            .memory(execution.memory)
            .observer(observer)
            .tool_dispatcher(tool_dispatcher)

            .prompt_builder(SystemPromptBuilder::with_defaults())
            .config(config.agent.clone())
            .model_name(model_name)
            .temperature(config.default_temperature)
            .workspace_dir(config.workspace_dir.clone())
            .classification_config(config.query_classification.clone())
            .available_hints(available_hints)
            .route_model_by_hint(route_model_by_hint)
            .identity_config(config.identity.clone())
            .skills(skills)
            .skills_prompt_mode(config.skills.prompt_injection_mode)
            .auto_save(config.memory.auto_save)
            .research_config(config.research.clone())
            .build()
    }

    fn trim_history(&mut self) {
        let max = self.config.max_history_messages;
        if self.history.len() <= max {
            return;
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in self.history.drain(..) {
            match &msg {
                ConversationMessage::Chat(chat) if chat.role == "system" => {
                    system_messages.push(msg);
                }
                _ => other_messages.push(msg),
            }
        }

        if other_messages.len() > max {
            let drop_count = other_messages.len() - max;
            other_messages.drain(0..drop_count);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    fn build_system_prompt(&self) -> Result<String> {
        let instructions = self.tool_dispatcher.prompt_instructions(&self.tools);
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            tools: &self.tools,
            skills: &self.skills,
            skills_prompt_mode: self.skills_prompt_mode,
            dispatcher_instructions: &instructions,
        };
        self.prompt_builder.build(&ctx)
    }

    async fn execute_tool_call(&self, call: &ParsedToolCall) -> ToolExecutionResult {
        let start = Instant::now();

        let (result, success) =
            if let Some(tool) = self.tools.iter().find(|t| t.name() == call.name) {
                match tool.execute(call.arguments.clone()).await {
                    Ok(r) => {
                        self.observer.record_event(&ObserverEvent::ToolCall {
                            tool: call.name.clone(),
                            duration: start.elapsed(),
                            success: r.success,
                        });
                        if r.success {
                            (r.output, true)
                        } else {
                            (format!("Error: {}", r.error.unwrap_or(r.output)), false)
                        }
                    }
                    Err(e) => {
                        self.observer.record_event(&ObserverEvent::ToolCall {
                            tool: call.name.clone(),
                            duration: start.elapsed(),
                            success: false,
                        });
                        (format!("Error executing {}: {e}", call.name), false)
                    }
                }
            } else {
                (format!("Unknown tool: {}", call.name), false)
            };

        ToolExecutionResult {
            name: call.name.clone(),
            output: result,
            success,
            tool_call_id: call.tool_call_id.clone(),
        }
    }

    async fn execute_tools(&self, calls: &[ParsedToolCall]) -> Vec<ToolExecutionResult> {
        if !self.config.parallel_tools {
            let mut results = Vec::with_capacity(calls.len());
            for call in calls {
                results.push(self.execute_tool_call(call).await);
            }
            return results;
        }

        let futs: Vec<_> = calls
            .iter()
            .map(|call| self.execute_tool_call(call))
            .collect();
        futures_util::future::join_all(futs).await
    }

    fn classify_model(&self, user_message: &str) -> String {
        if let Some(decision) =
            super::classifier::classify_with_decision(&self.classification_config, user_message)
        {
            if self.available_hints.contains(&decision.hint) {
                let resolved_model = self
                    .route_model_by_hint
                    .get(&decision.hint)
                    .map(String::as_str)
                    .unwrap_or("unknown");
                tracing::info!(
                    target: "query_classification",
                    hint = decision.hint.as_str(),
                    model = resolved_model,
                    rule_priority = decision.priority,
                    message_length = user_message.len(),
                    "Classified message route"
                );
                return format!("hint:{}", decision.hint);
            }
        }
        self.model_name.clone()
    }

    /// Execute one user turn, including provider inference, optional research,
    /// tool execution, memory writes, and history updates.
    pub async fn turn(&mut self, user_message: &str) -> Result<String> {
        if self.history.is_empty() {
            let system_prompt = self.build_system_prompt()?;
            self.history
                .push(ConversationMessage::Chat(ChatMessage::system(
                    system_prompt,
                )));
        }

        if self.auto_save {
            let _ = self
                .memory
                .store("user_msg", user_message, MemoryCategory::Conversation, None)
                .await;
        }

        let context = load_memory_context(
            self.memory.as_ref(),
            user_message,
            5,
            self.config.min_relevance_score,
        )
        .await
        .unwrap_or_default();

        // ── Research Phase ──────────────────────────────────────────────
        // If enabled and triggered, run a focused research turn to gather
        // information before the main response.
        let research_context = if research::should_trigger(&self.research_config, user_message) {
            if self.research_config.show_progress {
                println!("[Research] Gathering information...");
            }

            match research::run_research_phase(
                &self.research_config,
                self.provider.as_ref(),
                &self.tools,
                user_message,
                &self.model_name,
                self.temperature,
                self.observer.clone(),
            )
            .await
            {
                Ok(result) => {
                    if self.research_config.show_progress {
                        println!(
                            "[Research] Complete: {} tool calls, {} chars context",
                            result.tool_call_count,
                            result.context.len()
                        );
                        for summary in &result.tool_summaries {
                            println!("  - {}: {}", summary.tool_name, summary.result_preview);
                        }
                    }
                    if result.context.is_empty() {
                        None
                    } else {
                        Some(result.context)
                    }
                }
                Err(e) => {
                    tracing::warn!("Research phase failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
        let stamped_user_message = format!("[{now}] {user_message}");
        let enriched = match (&context, &research_context) {
            (c, Some(r)) if !c.is_empty() => {
                format!("{c}\n\n{r}\n\n{stamped_user_message}")
            }
            (_, Some(r)) => format!("{r}\n\n{stamped_user_message}"),
            (c, None) if !c.is_empty() => format!("{c}{stamped_user_message}"),
            _ => stamped_user_message,
        };

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(enriched)));

        let effective_model = self.classify_model(user_message);

        for _ in 0..self.config.max_tool_iterations {
            let messages = self.tool_dispatcher.to_provider_messages(&self.history);
            let response = match self
                .provider
                .chat(
                    ChatRequest {
                        messages: &messages,
                        tools: if self.tool_dispatcher.should_send_tool_specs() {
                            Some(&self.tool_specs)
                        } else {
                            None
                        },
                    },
                    &effective_model,
                    self.temperature,
                )
                .await
            {
                Ok(resp) => resp,
                Err(err) => return Err(err),
            };

            let (text, calls) = self.tool_dispatcher.parse_response(&response);
            if calls.is_empty() {
                let final_text = if text.is_empty() {
                    response.text.unwrap_or_default()
                } else {
                    text
                };

                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        final_text.clone(),
                    )));
                self.trim_history();

                return Ok(final_text);
            }

            if !text.is_empty() {
                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        text.clone(),
                    )));
                print!("{text}");
                let _ = std::io::stdout().flush();
            }

            self.history.push(ConversationMessage::AssistantToolCalls {
                text: response.text.clone(),
                tool_calls: response.tool_calls.clone(),
                reasoning_content: response.reasoning_content.clone(),
            });

            let results = self.execute_tools(&calls).await;
            let formatted = self.tool_dispatcher.format_results(&results);
            self.history.push(formatted);
            self.trim_history();
        }

        anyhow::bail!(
            "Agent exceeded maximum tool iterations ({})",
            self.config.max_tool_iterations
        )
    }

    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        self.turn(message).await
    }

    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 TopClaw Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            let response = match self.turn(&msg.content).await {
                Ok(resp) => resp,
                Err(e) => {
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            };
            println!("\n{response}\n");
        }

        listen_handle.abort();
        Ok(())
    }
}

/// Convenience helper used by the CLI to load an agent from config and either
/// run a single-shot message or enter the interactive loop.
pub async fn run(
    config: Config,
    message: Option<String>,
    provider_override: Option<String>,
    model_override: Option<String>,
    temperature: f64,
) -> Result<()> {
    let start = Instant::now();

    let mut effective_config = config;
    if let Some(p) = provider_override {
        effective_config.default_provider = Some(p);
    }
    if let Some(m) = model_override {
        effective_config.default_model = Some(m);
    }
    effective_config.default_temperature = temperature;

    let mut agent = Agent::from_config(&effective_config)?;

    let provider_name = effective_config
        .default_provider
        .as_deref()
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_NAME)
        .to_string();
    let model_name = effective_config
        .default_model
        .as_deref()
        .unwrap_or(crate::providers::DEFAULT_PROVIDER_MODEL)
        .to_string();

    agent.observer.record_event(&ObserverEvent::AgentStart {
        provider: provider_name.clone(),
        model: model_name.clone(),
    });

    if let Some(msg) = message {
        let response = agent.run_single(&msg).await?;
        println!("{response}");
    } else {
        agent.run_interactive().await?;
    }

    agent.observer.record_event(&ObserverEvent::AgentEnd {
        provider: provider_name,
        model: model_name,
        duration: start.elapsed(),
        tokens_used: None,
        cost_usd: None,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;

    struct MockProvider {
        responses: Mutex<Vec<crate::providers::ChatResponse>>,
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> Result<crate::providers::ChatResponse> {
            let mut guard = self.responses.lock();
            if guard.is_empty() {
                return Ok(crate::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }
            Ok(guard.remove(0))
        }
    }

    struct ModelCaptureProvider {
        responses: Mutex<Vec<crate::providers::ChatResponse>>,
        seen_models: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Provider for ModelCaptureProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> Result<String> {
            Ok("ok".into())
        }

        async fn chat(
            &self,
            _request: ChatRequest<'_>,
            model: &str,
            _temperature: f64,
        ) -> Result<crate::providers::ChatResponse> {
            self.seen_models.lock().push(model.to_string());
            let mut guard = self.responses.lock();
            if guard.is_empty() {
                return Ok(crate::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                });
            }
            Ok(guard.remove(0))
        }
    }

    struct MockTool;

    struct ToolResultFailureTool;

    struct ToolExecutionErrorTool;

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "echo"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<crate::tools::ToolResult> {
            Ok(crate::tools::ToolResult {
                success: true,
                output: "tool-out".into(),
                error: None,
            })
        }
    }

    #[async_trait]
    impl Tool for ToolResultFailureTool {
        fn name(&self) -> &str {
            "tool_result_failure"
        }

        fn description(&self) -> &str {
            "returns a failed tool result"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<crate::tools::ToolResult> {
            Ok(crate::tools::ToolResult {
                success: false,
                output: String::new(),
                error: Some("intentional failure".into()),
            })
        }
    }

    #[async_trait]
    impl Tool for ToolExecutionErrorTool {
        fn name(&self) -> &str {
            "tool_execution_error"
        }

        fn description(&self) -> &str {
            "throws an execution error"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value) -> Result<crate::tools::ToolResult> {
            anyhow::bail!("catastrophic tool failure")
        }
    }

    fn build_test_agent_with_tools(tools: Vec<Box<dyn Tool>>) -> Agent {
        let memory_cfg = crate::config::MemoryConfig {
            backend: "none".into(),
            ..crate::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::memory::create_memory(&memory_cfg, std::path::Path::new("/tmp"), None)
                .expect("memory creation should succeed with valid config"),
        );

        let observer: Arc<dyn Observer> = Arc::from(crate::observability::NoopObserver {});
        Agent::builder()
            .provider(Box::new(MockProvider {
                responses: Mutex::new(vec![]),
            }))
            .tools(tools)
            .memory(mem)
            .observer(observer)
            .tool_dispatcher(Box::new(XmlToolDispatcher))
            .workspace_dir(std::path::PathBuf::from("/tmp"))
            .build()
            .expect("agent builder should succeed with valid config")
    }

    #[tokio::test]
    async fn execute_tool_call_marks_failed_tool_result_as_unsuccessful() {
        let agent = build_test_agent_with_tools(vec![Box::new(ToolResultFailureTool)]);
        let result = agent
            .execute_tool_call(&ParsedToolCall {
                name: "tool_result_failure".into(),
                arguments: serde_json::json!({}),
                tool_call_id: Some("tc_fail".into()),
            })
            .await;

        assert!(!result.success);
        assert!(result.output.contains("intentional failure"));
        assert_eq!(result.tool_call_id.as_deref(), Some("tc_fail"));
    }

    #[tokio::test]
    async fn execute_tool_call_marks_execution_error_as_unsuccessful() {
        let agent = build_test_agent_with_tools(vec![Box::new(ToolExecutionErrorTool)]);
        let result = agent
            .execute_tool_call(&ParsedToolCall {
                name: "tool_execution_error".into(),
                arguments: serde_json::json!({}),
                tool_call_id: Some("tc_err".into()),
            })
            .await;

        assert!(!result.success);
        assert!(result
            .output
            .contains("Error executing tool_execution_error"));
        assert_eq!(result.tool_call_id.as_deref(), Some("tc_err"));
    }

    #[tokio::test]
    async fn turn_without_tools_returns_text() {
        let provider = Box::new(MockProvider {
            responses: Mutex::new(vec![crate::providers::ChatResponse {
                text: Some("hello".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }]),
        });

        let memory_cfg = crate::config::MemoryConfig {
            backend: "none".into(),
            ..crate::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::memory::create_memory(&memory_cfg, std::path::Path::new("/tmp"), None)
                .expect("memory creation should succeed with valid config"),
        );

        let observer: Arc<dyn Observer> = Arc::from(crate::observability::NoopObserver {});
        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .observer(observer)
            .tool_dispatcher(Box::new(XmlToolDispatcher))
            .workspace_dir(std::path::PathBuf::from("/tmp"))
            .build()
            .expect("agent builder should succeed with valid config");

        let response = agent.turn("hi").await.unwrap();
        assert_eq!(response, "hello");
    }

    #[tokio::test]
    async fn turn_with_native_dispatcher_handles_tool_results_variant() {
        let provider = Box::new(MockProvider {
            responses: Mutex::new(vec![
                crate::providers::ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![crate::providers::ToolCall {
                        id: "tc1".into(),
                        name: "echo".into(),
                        arguments: "{}".into(),
                    }],
                    usage: None,
                    reasoning_content: None,
                },
                crate::providers::ChatResponse {
                    text: Some("done".into()),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                },
            ]),
        });

        let memory_cfg = crate::config::MemoryConfig {
            backend: "none".into(),
            ..crate::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::memory::create_memory(&memory_cfg, std::path::Path::new("/tmp"), None)
                .expect("memory creation should succeed with valid config"),
        );

        let observer: Arc<dyn Observer> = Arc::from(crate::observability::NoopObserver {});
        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .observer(observer)
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(std::path::PathBuf::from("/tmp"))
            .build()
            .expect("agent builder should succeed with valid config");

        let response = agent.turn("hi").await.unwrap();
        assert_eq!(response, "done");
        assert!(agent
            .history()
            .iter()
            .any(|msg| matches!(msg, ConversationMessage::ToolResults(_))));
    }

    #[tokio::test]
    async fn load_memory_context_skips_legacy_autosave_entries() {
        // Verify the inline loader skips assistant autosave keys
        struct MockMemoryWithEntries {
            entries: Arc<Vec<crate::memory::MemoryEntry>>,
        }

        #[async_trait]
        impl Memory for MockMemoryWithEntries {
            async fn store(&self, _: &str, _: &str, _: MemoryCategory, _: Option<&str>) -> Result<()> {
                Ok(())
            }
            async fn recall(&self, _: &str, _: usize, _: Option<&str>) -> Result<Vec<crate::memory::MemoryEntry>> {
                Ok(self.entries.as_ref().clone())
            }
            async fn get(&self, _: &str) -> Result<Option<crate::memory::MemoryEntry>> {
                Ok(None)
            }
            async fn list(&self, _: Option<&MemoryCategory>, _: Option<&str>) -> Result<Vec<crate::memory::MemoryEntry>> {
                Ok(vec![])
            }
            async fn forget(&self, _: &str) -> Result<bool> { Ok(true) }
            async fn count(&self) -> Result<usize> { Ok(self.entries.len()) }
            async fn health_check(&self) -> bool { true }
            fn name(&self) -> &str { "mock" }
        }

        let memory = MockMemoryWithEntries {
            entries: Arc::new(vec![
                crate::memory::MemoryEntry {
                    id: "1".into(),
                    key: "assistant_resp_legacy".into(),
                    content: "should be skipped".into(),
                    category: MemoryCategory::Daily,
                    timestamp: "now".into(),
                    session_id: None,
                    score: Some(0.95),
                },
                crate::memory::MemoryEntry {
                    id: "2".into(),
                    key: "user_fact".into(),
                    content: "User prefers concise answers".into(),
                    category: MemoryCategory::Conversation,
                    timestamp: "now".into(),
                    session_id: None,
                    score: Some(0.9),
                },
            ]),
        };

        let context = load_memory_context(&memory, "answer style", 5, 0.0).await;
        assert!(context.contains("user_fact"));
        assert!(!context.contains("assistant_resp_legacy"));
        assert!(!context.contains("should be skipped"));
    }

    #[tokio::test]
    async fn turn_routes_with_hint_when_query_classification_matches() {
        let seen_models = Arc::new(Mutex::new(Vec::new()));
        let provider = Box::new(ModelCaptureProvider {
            responses: Mutex::new(vec![crate::providers::ChatResponse {
                text: Some("classified".into()),
                tool_calls: vec![],
                usage: None,
                reasoning_content: None,
            }]),
            seen_models: seen_models.clone(),
        });

        let memory_cfg = crate::config::MemoryConfig {
            backend: "none".into(),
            ..crate::config::MemoryConfig::default()
        };
        let mem: Arc<dyn Memory> = Arc::from(
            crate::memory::create_memory(&memory_cfg, std::path::Path::new("/tmp"), None)
                .expect("memory creation should succeed with valid config"),
        );

        let observer: Arc<dyn Observer> = Arc::from(crate::observability::NoopObserver {});
        let mut route_model_by_hint = HashMap::new();
        route_model_by_hint.insert("fast".to_string(), "anthropic/claude-haiku-4-5".to_string());
        let mut agent = Agent::builder()
            .provider(provider)
            .tools(vec![Box::new(MockTool)])
            .memory(mem)
            .observer(observer)
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(std::path::PathBuf::from("/tmp"))
            .classification_config(crate::config::QueryClassificationConfig {
                enabled: true,
                rules: vec![crate::config::ClassificationRule {
                    hint: "fast".to_string(),
                    keywords: vec!["quick".to_string()],
                    patterns: vec![],
                    min_length: None,
                    max_length: None,
                    priority: 10,
                }],
            })
            .available_hints(vec!["fast".to_string()])
            .route_model_by_hint(route_model_by_hint)
            .build()
            .expect("agent builder should succeed with valid config");

        let response = agent.turn("quick summary please").await.unwrap();
        assert_eq!(response, "classified");
        let seen = seen_models.lock();
        assert_eq!(seen.as_slice(), &["hint:fast".to_string()]);
    }
}
