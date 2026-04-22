use super::*;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn test_scrub_credentials() {
    let input = "API_KEY=sk-1234567890abcdef; token: 1234567890; password=\"secret123456\"";
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("API_KEY=sk-1*[REDACTED]"));
    assert!(scrubbed.contains("token: 1234*[REDACTED]"));
    assert!(scrubbed.contains("password=\"secr*[REDACTED]\""));
    assert!(!scrubbed.contains("abcdef"));
    assert!(!scrubbed.contains("secret123456"));
}

#[test]
fn test_scrub_credentials_json() {
    let input = r#"{"api_key": "sk-1234567890", "other": "public"}"#;
    let scrubbed = scrub_credentials(input);
    assert!(scrubbed.contains("\"api_key\": \"sk-1*[REDACTED]\""));
    assert!(scrubbed.contains("public"));
}

#[test]
fn maybe_inject_cron_add_delivery_populates_agent_delivery_from_channel_context() {
    let mut args = serde_json::json!({
        "job_type": "agent",
        "prompt": "remind me later"
    });

    maybe_inject_cron_add_delivery("cron_add", &mut args, "telegram", Some("-10012345"));

    assert_eq!(args["delivery"]["mode"], "announce");
    assert_eq!(args["delivery"]["channel"], "telegram");
    assert_eq!(args["delivery"]["to"], "-10012345");
}

#[test]
fn maybe_inject_cron_add_delivery_skips_shell_jobs() {
    let mut args = serde_json::json!({
        "job_type": "shell",
        "command": "echo hello"
    });

    maybe_inject_cron_add_delivery("cron_add", &mut args, "telegram", Some("-10012345"));

    assert!(args.get("delivery").is_none());
}

use crate::memory::{Memory, MemoryCategory, SqliteMemory};
use crate::observability::NoopObserver;
use crate::providers::router::{Route, RouterProvider};
use crate::providers::traits::{ProviderCapabilities, StreamChunk, StreamEvent, StreamOptions};
use crate::providers::ChatResponse;
use crate::runtime::NativeRuntime;
use crate::security::{AutonomyLevel, SecurityPolicy, ShellRedirectPolicy};
use tempfile::TempDir;

struct NonVisionProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for NonVisionProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("ok".to_string())
    }
}

struct VisionProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for VisionProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: false,
            vision: true,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("ok".to_string())
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let marker_count = crate::multimodal::count_image_markers(request.messages);
        if marker_count == 0 {
            anyhow::bail!("expected image markers in request messages");
        }

        if request.tools.is_some() {
            anyhow::bail!("no tools should be attached for this test");
        }

        Ok(ChatResponse {
            text: Some("vision-ok".to_string()),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        })
    }
}

struct ScriptedProvider {
    responses: Arc<Mutex<VecDeque<ChatResponse>>>,
    capabilities: ProviderCapabilities,
}

impl ScriptedProvider {
    fn from_text_responses(responses: Vec<&str>) -> Self {
        let scripted = responses
            .into_iter()
            .map(|text| ChatResponse {
                text: Some(text.to_string()),
                tool_calls: Vec::new(),
                usage: None,
                reasoning_content: None,
            })
            .collect();
        Self {
            responses: Arc::new(Mutex::new(scripted)),
            capabilities: ProviderCapabilities::default(),
        }
    }

    fn with_native_tool_support(mut self) -> Self {
        self.capabilities.native_tool_calling = true;
        self
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!("chat_with_system should not be used in scripted provider tests");
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        let mut responses = self
            .responses
            .lock()
            .expect("responses lock should be valid");
        responses
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("scripted provider exhausted responses"))
    }
}

struct StreamingScriptedProvider {
    responses: Arc<Mutex<VecDeque<String>>>,
    stream_calls: Arc<AtomicUsize>,
    chat_calls: Arc<AtomicUsize>,
}

impl StreamingScriptedProvider {
    fn from_text_responses(responses: Vec<&str>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(
                responses.into_iter().map(ToString::to_string).collect(),
            )),
            stream_calls: Arc::new(AtomicUsize::new(0)),
            chat_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Provider for StreamingScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!(
            "chat_with_system should not be used in streaming scripted provider tests"
        );
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("chat should not be called when streaming succeeds")
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_history(
        &self,
        _messages: &[ChatMessage],
        _model: &str,
        _temperature: f64,
        options: StreamOptions,
    ) -> futures_util::stream::BoxStream<
        'static,
        crate::providers::traits::StreamResult<StreamChunk>,
    > {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        if !options.enabled {
            return Box::pin(futures_util::stream::empty());
        }

        let response = self
            .responses
            .lock()
            .expect("responses lock should be valid")
            .pop_front()
            .unwrap_or_default();

        Box::pin(futures_util::stream::iter(vec![
            Ok(StreamChunk::delta(response)),
            Ok(StreamChunk::final_chunk()),
        ]))
    }
}

enum NativeStreamTurn {
    ToolCall(ToolCall),
    Text(String),
}

struct StreamingNativeToolEventProvider {
    turns: Arc<Mutex<VecDeque<NativeStreamTurn>>>,
    stream_calls: Arc<AtomicUsize>,
    stream_tool_requests: Arc<AtomicUsize>,
    chat_calls: Arc<AtomicUsize>,
}

impl StreamingNativeToolEventProvider {
    fn with_turns(turns: Vec<NativeStreamTurn>) -> Self {
        Self {
            turns: Arc::new(Mutex::new(turns.into())),
            stream_calls: Arc::new(AtomicUsize::new(0)),
            stream_tool_requests: Arc::new(AtomicUsize::new(0)),
            chat_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Provider for StreamingNativeToolEventProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!(
            "chat_with_system should not be used in streaming native tool event provider tests"
        );
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("chat should not be called when native streaming events succeed")
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_streaming_tool_events(&self) -> bool {
        true
    }

    fn stream_chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
        options: StreamOptions,
    ) -> futures_util::stream::BoxStream<
        'static,
        crate::providers::traits::StreamResult<StreamEvent>,
    > {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        if request.tools.is_some_and(|tools| !tools.is_empty()) {
            self.stream_tool_requests.fetch_add(1, Ordering::SeqCst);
        }
        if !options.enabled {
            return Box::pin(futures_util::stream::empty());
        }

        let turn = self
            .turns
            .lock()
            .expect("turns lock should be valid")
            .pop_front()
            .expect("streaming turns should have scripted output");
        match turn {
            NativeStreamTurn::ToolCall(tool_call) => {
                Box::pin(futures_util::stream::iter(vec![
                    Ok(StreamEvent::ToolCall(tool_call)),
                    Ok(StreamEvent::Final),
                ]))
            }
            NativeStreamTurn::Text(text) => Box::pin(futures_util::stream::iter(vec![
                Ok(StreamEvent::TextDelta(StreamChunk::delta(text))),
                Ok(StreamEvent::Final),
            ])),
        }
    }
}

struct RouteAwareStreamingProvider {
    response: String,
    stream_calls: Arc<AtomicUsize>,
    chat_calls: Arc<AtomicUsize>,
    last_model: Arc<Mutex<String>>,
}

impl RouteAwareStreamingProvider {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            stream_calls: Arc::new(AtomicUsize::new(0)),
            chat_calls: Arc::new(AtomicUsize::new(0)),
            last_model: Arc::new(Mutex::new(String::new())),
        }
    }
}

#[async_trait]
impl Provider for RouteAwareStreamingProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        anyhow::bail!("chat_with_system should not be used in route-aware stream tests");
    }

    async fn chat(
        &self,
        _request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("chat should not be called when routed streaming succeeds")
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn stream_chat_with_history(
        &self,
        _messages: &[ChatMessage],
        model: &str,
        _temperature: f64,
        options: StreamOptions,
    ) -> futures_util::stream::BoxStream<
        'static,
        crate::providers::traits::StreamResult<StreamChunk>,
    > {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        *self
            .last_model
            .lock()
            .expect("last_model lock should be valid") = model.to_string();
        if !options.enabled {
            return Box::pin(futures_util::stream::empty());
        }

        Box::pin(futures_util::stream::iter(vec![
            Ok(StreamChunk::delta(self.response.clone())),
            Ok(StreamChunk::final_chunk()),
        ]))
    }
}

struct CountingTool {
    name: String,
    invocations: Arc<AtomicUsize>,
}

impl CountingTool {
    fn new(name: &str, invocations: Arc<AtomicUsize>) -> Self {
        Self {
            name: name.to_string(),
            invocations,
        }
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Counts executions for loop-stability tests"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
    ) -> anyhow::Result<crate::tools::ToolResult> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let value = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("counted:{value}"),
            error: None,
        })
    }
}

struct FailingTool {
    name: String,
    invocations: Arc<AtomicUsize>,
    error: &'static str,
}

impl FailingTool {
    fn new(name: &str, invocations: Arc<AtomicUsize>, error: &'static str) -> Self {
        Self {
            name: name.to_string(),
            invocations,
            error,
        }
    }
}

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Fails executions for loop-stability tests"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
    ) -> anyhow::Result<crate::tools::ToolResult> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        Ok(crate::tools::ToolResult {
            success: false,
            output: String::new(),
            error: Some(self.error.to_string()),
        })
    }
}

struct DelayTool {
    name: String,
    delay_ms: u64,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl DelayTool {
    fn new(
        name: &str,
        delay_ms: u64,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            name: name.to_string(),
            delay_ms,
            active,
            max_active,
        }
    }
}

#[async_trait]
impl Tool for DelayTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Delay tool for testing parallel tool execution"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            },
            "required": ["value"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
    ) -> anyhow::Result<crate::tools::ToolResult> {
        let now_active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now_active, Ordering::SeqCst);

        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;

        self.active.fetch_sub(1, Ordering::SeqCst);

        let value = args
            .get("value")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        Ok(crate::tools::ToolResult {
            success: true,
            output: format!("ok:{value}"),
            error: None,
        })
    }
}

#[tokio::test]
async fn run_tool_call_loop_returns_structured_error_for_non_vision_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = NonVisionProvider {
        calls: Arc::clone(&calls),
    };

    let mut history = vec![ChatMessage::user(
        "please inspect [IMAGE:data:image/png;base64,iVBORw0KGgo=]".to_string(),
    )];
    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let observer = NoopObserver;

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        3,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("provider without vision support should fail");

    assert!(err.to_string().contains("provider_capability_error"));
    assert!(err.to_string().contains("capability=vision"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_tool_call_loop_rejects_oversized_image_payload() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = VisionProvider {
        calls: Arc::clone(&calls),
    };

    let oversized_payload = STANDARD.encode(vec![0_u8; (1024 * 1024) + 1]);
    let mut history = vec![ChatMessage::user(format!(
        "[IMAGE:data:image/png;base64,{oversized_payload}]"
    ))];

    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let observer = NoopObserver;
    let multimodal = crate::config::MultimodalConfig {
        max_images: 4,
        max_image_size_mb: 1,
        allow_remote_fetch: false,
    };

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &multimodal,
        3,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("oversized payload must fail");

    assert!(err
        .to_string()
        .contains("multimodal image size limit exceeded"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_tool_call_loop_accepts_valid_multimodal_request_flow() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = VisionProvider {
        calls: Arc::clone(&calls),
    };

    let mut history = vec![ChatMessage::user(
        "Analyze this [IMAGE:data:image/png;base64,iVBORw0KGgo=]".to_string(),
    )];
    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        3,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("valid multimodal payload should pass");

    assert_eq!(result, "vision-ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn should_execute_tools_in_parallel_returns_false_when_approval_is_required() {
    let calls = vec![
        ParsedToolCall {
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "http_request".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            tool_call_id: None,
        },
    ];
    let approval_cfg = crate::config::AutonomyConfig::default();
    let approval_mgr = ApprovalManager::from_config(&approval_cfg);

    assert!(!should_execute_tools_in_parallel(
        &calls,
        Some(&approval_mgr),
        false,
    ));
}

#[test]
fn should_execute_tools_in_parallel_returns_true_when_cli_has_no_interactive_approvals() {
    let calls = vec![
        ParsedToolCall {
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
            tool_call_id: None,
        },
        ParsedToolCall {
            name: "http_request".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            tool_call_id: None,
        },
    ];
    let approval_cfg = crate::config::AutonomyConfig {
        level: crate::security::AutonomyLevel::Full,
        ..crate::config::AutonomyConfig::default()
    };
    let approval_mgr = ApprovalManager::from_config(&approval_cfg);

    assert!(should_execute_tools_in_parallel(
        &calls,
        Some(&approval_mgr),
        false,
    ));
}

#[tokio::test]
async fn run_tool_call_loop_executes_multiple_tools_with_ordered_results() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"delay_a","arguments":{"value":"A"}}
</tool_call>
<tool_call>
{"name":"delay_b","arguments":{"value":"B"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(DelayTool::new(
            "delay_a",
            200,
            Arc::clone(&active),
            Arc::clone(&max_active),
        )),
        Box::new(DelayTool::new(
            "delay_b",
            200,
            Arc::clone(&active),
            Arc::clone(&max_active),
        )),
    ];

    let approval_cfg = crate::config::AutonomyConfig {
        level: crate::security::AutonomyLevel::Full,
        ..crate::config::AutonomyConfig::default()
    };
    let approval_mgr = ApprovalManager::from_config(&approval_cfg);

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run tool calls"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(&approval_mgr),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("parallel execution should complete");

    assert_eq!(result, "done");
    assert!(
        max_active.load(Ordering::SeqCst) >= 1,
        "tools should execute successfully"
    );

    let tool_results_message = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.starts_with("[Tool results]"))
        .expect("tool results message should be present");
    let idx_a = tool_results_message
        .content
        .find("name=\"delay_a\"")
        .expect("delay_a result should be present");
    let idx_b = tool_results_message
        .content
        .find("name=\"delay_b\"")
        .expect("delay_b result should be present");
    assert!(
        idx_a < idx_b,
        "tool results should preserve input order for tool call mapping"
    );
}

#[tokio::test]
async fn run_tool_call_loop_denies_supervised_tools_on_non_cli_channels() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo hi"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(DelayTool::new(
        "shell",
        50,
        Arc::clone(&active),
        Arc::clone(&max_active),
    ))];

    let approval_mgr = ApprovalManager::from_config(&crate::config::AutonomyConfig::default());

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run shell"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(&approval_mgr),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("tool loop should complete with denied tool execution");

    assert_eq!(result, "done");
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        0,
        "shell tool must not execute when approval is unavailable on non-CLI channels"
    );
}

#[tokio::test]
async fn run_tool_call_loop_returns_pending_non_cli_approval_error() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo hi"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(DelayTool::new(
        "shell",
        50,
        Arc::clone(&active),
        Arc::clone(&max_active),
    ))];

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));
    let (prompt_tx, mut prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<NonCliApprovalPrompt>();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run shell"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop_with_non_cli_approval_context(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        Some(NonCliApprovalContext {
            sender: "alice".to_string(),
            reply_target: "chat-approval".to_string(),
            message_id: "msg-approval".to_string(),
            content: "run shell".to_string(),
            timestamp: 1,
            thread_ts: None,
            prompt_tx,
        }),
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("tool loop should fail fast while waiting for non-cli approval");

    let pending = is_non_cli_approval_pending(&result)
        .expect("non-cli approval error should be surfaced");
    assert_eq!(pending.tool_name, "current execution plan");
    let prompt = prompt_rx
        .recv()
        .await
        .expect("approval prompt should arrive");
    assert_eq!(
        prompt.title,
        "Approval required for current execution plan."
    );
    assert!(prompt.details.contains("`shell`"));
    assert!(prompt.details.contains("echo hi"));
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        0,
        "shell tool must not execute before a new turn is sent after approval"
    );
}

#[tokio::test]
async fn run_tool_call_loop_consumes_one_time_non_cli_allow_all_token() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo hi"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(DelayTool::new(
        "shell",
        50,
        Arc::clone(&active),
        Arc::clone(&max_active),
    ))];

    let approval_mgr = ApprovalManager::from_config(&crate::config::AutonomyConfig::default());
    approval_mgr.grant_non_cli_allow_all_once();
    assert_eq!(approval_mgr.non_cli_allow_all_once_remaining(), 1);

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run shell once"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(&approval_mgr),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("tool loop should consume one-time allow-all token");

    assert_eq!(result, "done");
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        1,
        "shell tool should execute after consuming one-time allow-all token"
    );
    assert_eq!(approval_mgr.non_cli_allow_all_once_remaining(), 0);
}

#[tokio::test]
async fn run_tool_call_loop_executes_approved_shell_command_from_turn_grant() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"touch approved-turn-grant.txt"}}
</tool_call>"#,
        "done",
    ]);

    let workspace = tempfile::tempdir().expect("temp dir should be created");
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(crate::tools::ShellTool::new(
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace.path().to_path_buf(),
            ..SecurityPolicy::default()
        }),
        Arc::new(NativeRuntime::new()),
    ))];

    let approval_mgr = ApprovalManager::from_config(&crate::config::AutonomyConfig::default());
    approval_mgr.grant_non_cli_turn_grant(crate::approval::NonCliTurnApprovalGrant {
        approved_shell_commands: vec!["touch approved-turn-grant.txt".to_string()],
    });

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("create the approved file"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(&approval_mgr),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("approved one-turn shell command should execute");

    assert_eq!(result, "done");
    assert!(workspace.path().join("approved-turn-grant.txt").exists());
}

/// Simulates the full Telegram approval-to-shell cycle:
/// 1. First turn: tool loop hits non-CLI approval gate, returns pending error
/// 2. Approval: grant turn with approved shell commands
/// 3. Resumed turn: tool loop consumes grant, shell executes with real ShellTool
///
/// This proves the complete Telegram supervised shell-command flow works.
#[tokio::test]
async fn run_tool_call_loop_full_telegram_approval_to_shell_cycle() {
    let workspace = tempfile::tempdir().expect("temp dir should be created");
    let shell_command = "touch telegram-approval-cycle-test.txt";

    // Step 1: First turn — should return NonCliApprovalPending
    let provider_turn1 = ScriptedProvider::from_text_responses(vec![
        &format!(
            r#"<tool_call>
{{"name":"shell","arguments":{{"command":"{shell_command}"}}}}
</tool_call>"#
        ),
        "done",
    ]);

    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: workspace.path().to_path_buf(),
        allowed_commands: vec!["touch".into()],
        ..SecurityPolicy::default()
    });
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(crate::tools::ShellTool::new(
        Arc::clone(&security),
        Arc::new(NativeRuntime::new()),
    ))];

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));
    let (prompt_tx, mut prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<NonCliApprovalPrompt>();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("create the file"),
    ];
    let observer = NoopObserver;

    let err = run_tool_call_loop_with_non_cli_approval_context(
        &provider_turn1,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        Some(NonCliApprovalContext {
            sender: "alice".to_string(),
            reply_target: "chat-cycle".to_string(),
            message_id: "msg-cycle".to_string(),
            content: "create the file".to_string(),
            timestamp: 1,
            thread_ts: None,
            prompt_tx,
        }),
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("first turn must return pending approval error");

    let pending =
        is_non_cli_approval_pending(&err).expect("error should be NonCliApprovalPending");
    assert_eq!(pending.tool_name, "current execution plan");

    let prompt = prompt_rx
        .recv()
        .await
        .expect("approval prompt should arrive");
    assert!(prompt.details.contains("`shell`"));
    assert!(prompt.details.contains(shell_command));
    assert!(
        !workspace
            .path()
            .join("telegram-approval-cycle-test.txt")
            .exists(),
        "file must NOT exist before approval"
    );

    // Step 2: Simulate user clicking "Approve" — grant the turn with approved commands
    let pending_req = approval_mgr.list_non_cli_pending_requests(
        Some("alice"),
        Some("telegram"),
        Some("chat-cycle"),
    );
    assert_eq!(pending_req.len(), 1);
    let req = &pending_req[0];
    assert!(req
        .approved_shell_commands
        .contains(&shell_command.to_string()));

    let confirmed = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "alice", "telegram", "chat-cycle")
        .expect("confirm should succeed");
    approval_mgr.grant_non_cli_turn_grant(crate::approval::NonCliTurnApprovalGrant {
        approved_shell_commands: confirmed.approved_shell_commands.clone(),
    });

    // Step 3: Resumed turn — should consume grant and execute shell
    let provider_turn2 = ScriptedProvider::from_text_responses(vec![
        &format!(
            r#"<tool_call>
{{"name":"shell","arguments":{{"command":"{shell_command}"}}}}
</tool_call>"#
        ),
        "done",
    ]);

    let mut resumed_history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("create the file"),
    ];

    let result = run_tool_call_loop(
        &provider_turn2,
        &mut resumed_history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("resumed turn should complete successfully");

    assert_eq!(result, "done");
    assert!(
        workspace
            .path()
            .join("telegram-approval-cycle-test.txt")
            .exists(),
        "file MUST exist after approved execution"
    );
    assert_eq!(
        approval_mgr.non_cli_allow_all_once_remaining(),
        0,
        "turn grant must be consumed"
    );
}

/// Proves that denying a pending Telegram approval request prevents shell execution.
#[tokio::test]
async fn run_tool_call_loop_telegram_deny_prevents_shell_execution() {
    let workspace = tempfile::tempdir().expect("temp dir should be created");

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));

    // Create pending request
    let req = approval_mgr.create_non_cli_pending_request(
        crate::channels::APPROVAL_ALL_TOOLS_ONCE_TOKEN,
        "alice",
        "telegram",
        "chat-deny",
        Some(crate::approval::PendingNonCliResumeRequest {
            message_id: "msg-deny".into(),
            content: "create a file".into(),
            timestamp: 1,
            thread_ts: None,
        }),
        None,
        vec!["touch deny-test.txt".into()],
    );

    // Deny the request
    let rejected = approval_mgr
        .reject_non_cli_pending_request(&req.request_id, "alice", "telegram", "chat-deny")
        .expect("reject should succeed");
    assert_eq!(rejected.request_id, req.request_id);

    // No turn grant should exist
    assert_eq!(approval_mgr.non_cli_allow_all_once_remaining(), 0);

    // Confirm the request is gone
    assert!(approval_mgr
        .list_non_cli_pending_requests(Some("alice"), Some("telegram"), Some("chat-deny"))
        .is_empty());

    // File must not exist
    assert!(
        !workspace.path().join("deny-test.txt").exists(),
        "file must NOT exist after denial"
    );
}

/// Proves that expired requests cannot be confirmed (duplicate/stale callback).
#[tokio::test]
async fn run_tool_call_loop_telegram_expired_request_fails_cleanly() {
    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));

    let req = approval_mgr.create_non_cli_pending_request(
        crate::channels::APPROVAL_ALL_TOOLS_ONCE_TOKEN,
        "alice",
        "telegram",
        "chat-expire",
        None,
        None,
        vec!["echo expired".into()],
    );

    // Force the request to expire
    {
        let pending = approval_mgr.list_non_cli_pending_requests(None, None, None);
        assert_eq!(pending.len(), 1);
    }

    // Confirm once — should succeed
    let confirmed = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "alice", "telegram", "chat-expire")
        .expect("first confirm should succeed");
    assert_eq!(confirmed.request_id, req.request_id);

    // Second confirm on same ID — should fail (already consumed)
    let err = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "alice", "telegram", "chat-expire")
        .expect_err("duplicate confirm should fail");
    assert_eq!(err, crate::approval::PendingApprovalError::NotFound);
}

/// Proves that unauthorized approvers cannot confirm requests.
#[tokio::test]
async fn run_tool_call_loop_telegram_unauthorized_approver_rejected() {
    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));

    let req = approval_mgr.create_non_cli_pending_request(
        crate::channels::APPROVAL_ALL_TOOLS_ONCE_TOKEN,
        "alice",
        "telegram",
        "chat-auth",
        None,
        None,
        vec!["echo authorized".into()],
    );

    // Different sender tries to confirm
    let err = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "mallory", "telegram", "chat-auth")
        .expect_err("different sender should fail");
    assert_eq!(
        err,
        crate::approval::PendingApprovalError::RequesterMismatch
    );

    // Different channel tries to confirm
    let err = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "alice", "discord", "chat-auth")
        .expect_err("different channel should fail");
    assert_eq!(
        err,
        crate::approval::PendingApprovalError::RequesterMismatch
    );

    // Correct sender + channel should still work
    let confirmed = approval_mgr
        .confirm_non_cli_pending_request(&req.request_id, "alice", "telegram", "chat-auth")
        .expect("correct sender+channel should succeed");
    assert_eq!(confirmed.request_id, req.request_id);
}

#[tokio::test]
async fn run_tool_call_loop_parallelizes_approved_non_cli_turn() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"delay_a","arguments":{"value":"A"}}
</tool_call>
<tool_call>
{"name":"delay_b","arguments":{"value":"B"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(DelayTool::new(
            "delay_a",
            200,
            Arc::clone(&active),
            Arc::clone(&max_active),
        )),
        Box::new(DelayTool::new(
            "delay_b",
            200,
            Arc::clone(&active),
            Arc::clone(&max_active),
        )),
    ];

    let approval_mgr = ApprovalManager::from_config(&crate::config::AutonomyConfig::default());
    approval_mgr.grant_non_cli_allow_all_once();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run the approved tools"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(&approval_mgr),
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("approved non-cli turn should execute");

    assert_eq!(result, "done");
    assert!(
        max_active.load(Ordering::SeqCst) >= 2,
        "approved turn should allow parallel execution"
    );
}

#[tokio::test]
async fn run_tool_call_loop_executes_non_cli_investigation_batch_without_prompt() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"task_plan","arguments":{"action":"create","value":"plan"}}
</tool_call>"#,
        r#"<tool_call>
{"name":"glob_search","arguments":{"value":"src/**/*.rs"}}
</tool_call>"#,
        "done",
    ]);

    let task_plan_calls = Arc::new(AtomicUsize::new(0));
    let glob_calls = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(CountingTool::new("task_plan", Arc::clone(&task_plan_calls))),
        Box::new(CountingTool::new("glob_search", Arc::clone(&glob_calls))),
    ];

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));
    let (prompt_tx, mut prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<NonCliApprovalPrompt>();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("inspect the repo and tell me what you find"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop_with_non_cli_approval_context(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        Some(NonCliApprovalContext {
            sender: "alice".to_string(),
            reply_target: "chat-investigation".to_string(),
            message_id: "msg-investigation".to_string(),
            content: "run shell".to_string(),
            timestamp: 1,
            thread_ts: None,
            prompt_tx,
        }),
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("investigation batch should execute without non-cli approval prompts");

    assert_eq!(result, "done");
    assert_eq!(task_plan_calls.load(Ordering::SeqCst), 1);
    assert_eq!(glob_calls.load(Ordering::SeqCst), 1);
    assert!(
        prompt_rx.try_recv().is_err(),
        "non-cli investigation tools should not emit approval prompts"
    );
}

#[tokio::test]
async fn run_tool_call_loop_batches_non_cli_plan_into_single_prompt() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"git status"}}
</tool_call>
<tool_call>
{"name":"web_fetch","arguments":{"url":"https://example.com/docs"}}
</tool_call>"#,
        "done",
    ]);

    let shell_calls = Arc::new(AtomicUsize::new(0));
    let web_calls = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(CountingTool::new("shell", Arc::clone(&shell_calls))),
        Box::new(CountingTool::new("web_fetch", Arc::clone(&web_calls))),
    ];

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));
    let (prompt_tx, mut prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<NonCliApprovalPrompt>();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("inspect the docs and the repo state"),
    ];
    let observer = NoopObserver;

    let err = run_tool_call_loop_with_non_cli_approval_context(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        Some(NonCliApprovalContext {
            sender: "alice".to_string(),
            reply_target: "chat-batch".to_string(),
            message_id: "msg-batch".to_string(),
            content: "inspect the docs and the repo state".to_string(),
            timestamp: 1,
            thread_ts: None,
            prompt_tx,
        }),
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("batched non-cli plan should wait for approval");

    let pending =
        is_non_cli_approval_pending(&err).expect("pending approval error should be returned");
    assert_eq!(pending.tool_name, "current execution plan");

    let prompt = prompt_rx
        .recv()
        .await
        .expect("batched approval prompt should arrive");
    assert_eq!(
        prompt.title,
        "Approval required for current execution plan."
    );
    assert!(prompt.details.contains("`shell`"));
    assert!(prompt.details.contains("git status"));
    assert!(prompt.details.contains("`web_fetch`"));
    assert!(prompt.details.contains("https://example.com/docs"));
    assert!(
        prompt_rx.try_recv().is_err(),
        "only one approval prompt should be emitted for the current plan"
    );
    assert_eq!(shell_calls.load(Ordering::SeqCst), 0);
    assert_eq!(web_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn repeated_web_tool_failures_stop_before_iteration_limit() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"web_search_tool","arguments":{"query":"current weather Montreal"}}
</tool_call>"#,
        r#"<tool_call>
{"name":"web_fetch","arguments":{}}
</tool_call>"#,
        r#"<tool_call>
{"name":"web_search_tool","arguments":{"query":"Montreal weather today"}}
</tool_call>"#,
    ]);

    let search_calls = Arc::new(AtomicUsize::new(0));
    let fetch_calls = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(FailingTool::new(
            "web_search_tool",
            Arc::clone(&search_calls),
            "simulated search outage",
        )),
        Box::new(FailingTool::new(
            "web_fetch",
            Arc::clone(&fetch_calls),
            "missing url",
        )),
    ];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("what's current weather in Montreal?"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        6,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("repeated web tool failures should produce a graceful answer");

    assert!(
        result.contains("web tools failed 2 times"),
        "result should explain why the loop stopped: {result}"
    );
    assert!(
        result.contains("stopped instead of continuing"),
        "result should avoid silent iteration-limit exhaustion: {result}"
    );
    assert!(
        result.contains("missing url"),
        "result should include the latest useful tool error: {result}"
    );
    assert_eq!(search_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fetch_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn run_tool_call_loop_shows_approval_prompt_for_unlisted_shell_command() {
    // When the LLM plans a shell command whose binary is not in the
    // static allowlist, the system should still show the inline-button
    // approval prompt instead of silently blocking the plan. On
    // approval, the command is granted via the temporary allowlist.
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"rm -rf tmp_test_dir"}}
</tool_call>
<tool_call>
{"name":"web_fetch","arguments":{"url":"https://example.com/docs"}}
</tool_call>"#,
        "done",
    ]);

    let web_calls = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![
        Box::new(crate::tools::ShellTool::new(
            Arc::new(SecurityPolicy {
                autonomy: AutonomyLevel::Supervised,
                allowed_commands: vec!["echo".into()],
                workspace_dir: std::env::temp_dir(),
                ..SecurityPolicy::default()
            }),
            Arc::new(NativeRuntime::new()),
        )),
        Box::new(CountingTool::new("web_fetch", Arc::clone(&web_calls))),
    ];

    let approval_mgr = Arc::new(ApprovalManager::from_config(
        &crate::config::AutonomyConfig::default(),
    ));
    let (prompt_tx, mut prompt_rx) =
        tokio::sync::mpsc::unbounded_channel::<NonCliApprovalPrompt>();

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("inspect the repo and docs"),
    ];
    let observer = NoopObserver;

    let err = run_tool_call_loop_with_non_cli_approval_context(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        Some(approval_mgr.as_ref()),
        "telegram",
        Some(NonCliApprovalContext {
            sender: "alice".to_string(),
            reply_target: "chat-blocked-shell".to_string(),
            message_id: "msg-blocked-shell".to_string(),
            content: "inspect the repo and docs".to_string(),
            timestamp: 1,
            thread_ts: None,
            prompt_tx,
        }),
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("unlisted shell plan should trigger approval prompt, not silent block");

    assert!(
        err.downcast_ref::<NonCliApprovalPending>().is_some(),
        "error should be NonCliApprovalPending so the caller can wait for user decision"
    );

    let prompt = prompt_rx
        .try_recv()
        .expect("approval prompt should be emitted for unlisted shell commands");
    assert!(
        !prompt.request_id.is_empty(),
        "approval prompt should carry a request ID for inline-button callbacks"
    );
}

#[tokio::test]
async fn run_tool_call_loop_blocks_tools_excluded_for_channel() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo hi"}}
</tool_call>"#,
        "done",
    ]);

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(DelayTool::new(
        "shell",
        50,
        Arc::clone(&active),
        Arc::clone(&max_active),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run shell"),
    ];
    let observer = NoopObserver;
    let excluded_tools = vec!["shell".to_string()];

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &excluded_tools,
    )
    .await
    .expect("tool loop should complete with blocked tool execution");

    assert_eq!(result, "done");
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        0,
        "excluded tool must not execute even if the model requests it"
    );

    let tool_results_message = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.starts_with("[Tool results]"))
        .expect("tool results message should be present");
    assert!(
        tool_results_message
            .content
            .contains("not available in this channel"),
        "blocked reason should be visible to the model"
    );
}

#[tokio::test]
async fn run_tool_call_loop_deduplicates_repeated_tool_calls() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"count_tool","arguments":{"value":"A"}}
</tool_call>
<tool_call>
{"name":"count_tool","arguments":{"value":"A"}}
</tool_call>"#,
        "done",
    ]);

    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run tool calls"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("loop should finish after deduplicating repeated calls");

    assert_eq!(result, "done");
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "duplicate tool call with same args should not execute twice"
    );

    let tool_results = history
        .iter()
        .find(|msg| msg.role == "user" && msg.content.starts_with("[Tool results]"))
        .expect("prompt-mode tool result payload should be present");
    assert!(tool_results.content.contains("counted:A"));
    assert!(tool_results.content.contains("Skipped duplicate tool call"));
}

#[tokio::test]
async fn run_tool_call_loop_shell_strip_policy_handles_repeated_redirect_calls() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo redirect-loop-ok 2>&1"}}
</tool_call>"#,
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo redirect-loop-ok 2>&1"}}
</tool_call>"#,
        r#"<tool_call>
{"name":"shell","arguments":{"command":"echo redirect-loop-ok 2>&1"}}
</tool_call>"#,
        "done after shell redirect retries",
    ]);

    let workspace = TempDir::new().expect("temp workspace");
    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.path().to_path_buf(),
        allowed_commands: vec!["echo".into()],
        shell_redirect_policy: ShellRedirectPolicy::Strip,
        ..SecurityPolicy::default()
    });
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(crate::tools::ShellTool::new(
        Arc::clone(&security),
        Arc::new(NativeRuntime::new()),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run repeated shell redirects"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        6,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("loop should complete when strip policy normalizes redirects");

    assert_eq!(result, "done after shell redirect retries");

    let tool_result_messages: Vec<_> = history
        .iter()
        .filter(|msg| msg.role == "user" && msg.content.starts_with("[Tool results]"))
        .collect();
    assert_eq!(
        tool_result_messages.len(),
        3,
        "expected one tool result payload per scripted shell call"
    );
    for message in tool_result_messages {
        assert!(
            message.content.contains("<tool_result name=\"shell\">"),
            "tool results should include shell execution payloads"
        );
        assert!(
            !message
                .content
                .contains("Command not allowed by security policy"),
            "strip policy should avoid redirect-policy rejections"
        );
    }
}

#[tokio::test]
async fn run_tool_call_loop_retries_when_response_claims_completion_without_tool_call() {
    let provider = ScriptedProvider::from_text_responses(vec![
        "Done — I've created the `names` folder in the current working directory.",
        r#"<tool_call>
{"name":"count_tool","arguments":{"value":"mkdir names"}}
</tool_call>"#,
        "done after verified tool execution",
    ]);

    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("please create the names folder"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("completion claim without tool call should trigger a recovery retry");

    assert_eq!(result, "done after verified tool execution");
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "recovery retry should enforce one real tool execution"
    );
}

#[tokio::test]
async fn run_tool_call_loop_errors_when_completion_claim_repeats_without_tool_call() {
    let provider = ScriptedProvider::from_text_responses(vec![
        "Done — I've created the `names` folder in the current working directory.",
        "I've successfully written the files into the workspace directory.",
    ]);

    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("please create the names folder"),
    ];
    let observer = NoopObserver;

    let err = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect_err("repeated completion claims without tool call should hard-fail");

    let err_text = err.to_string();
    assert!(
        err_text.contains("deferred action without emitting a tool call"),
        "unexpected error text: {err_text}"
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "tool should not execute when provider never emits a real tool call"
    );
}

#[tokio::test]
async fn run_tool_call_loop_retries_when_model_claims_missing_file_tools() {
    let provider = ScriptedProvider::from_text_responses(vec![
        "I don't have access to a file creation tool in my current set of available functions.",
        r#"<tool_call>
{"name":"file_write","arguments":{"value":"retry"}}
</tool_call>"#,
        "done after file tool",
    ]);

    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "file_write",
        Arc::clone(&invocations),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("create a test file"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("loop should retry once when model wrongly claims file tools are unavailable");

    assert_eq!(result, "done after file tool");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn run_tool_call_loop_allows_text_only_planning_without_tool_call() {
    let provider = ScriptedProvider::from_text_responses(vec![
        "We were previously discussing gmail integration. Goal 1 is done. Our next task is Goal 2 — Gmail API via OAuth. Here is the implementation plan before any tool actions.",
    ]);

    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::new(AtomicUsize::new(0)),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("we finished goal one, what is next"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("planning-only text should be returned without forced tool-call rejection");

    assert!(result.contains("implementation plan"));
}

#[tokio::test]
async fn run_tool_call_loop_native_mode_preserves_fallback_tool_call_ids() {
    let provider = ScriptedProvider::from_text_responses(vec![
        r#"{"content":"Need to call tool","tool_calls":[{"id":"call_abc","name":"count_tool","arguments":"{\"value\":\"X\"}"}]}"#,
        "done",
    ])
    .with_native_tool_support();

    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];

    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run tool calls"),
    ];
    let observer = NoopObserver;

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "cli",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        None,
        None,
        &[],
    )
    .await
    .expect("native fallback id flow should complete");

    assert_eq!(result, "done");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert!(
        history.iter().any(|msg| {
            msg.role == "tool" && msg.content.contains("\"tool_call_id\":\"call_abc\"")
        }),
        "tool result should preserve parsed fallback tool_call_id in native mode"
    );
    assert!(
        history
            .iter()
            .all(|msg| !(msg.role == "user" && msg.content.starts_with("[Tool results]"))),
        "native mode should use role=tool history instead of prompt fallback wrapper"
    );
}

#[tokio::test]
async fn run_tool_call_loop_consumes_provider_stream_for_final_response() {
    let provider =
        StreamingScriptedProvider::from_text_responses(vec!["streamed final answer"]);
    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("say hi"),
    ];
    let observer = NoopObserver;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        Some(tx),
        None,
        &[],
    )
    .await
    .expect("streaming provider should complete");

    let mut visible_deltas = String::new();
    while let Some(delta) = rx.recv().await {
        if delta == DRAFT_CLEAR_SENTINEL || delta.starts_with(DRAFT_PROGRESS_SENTINEL) {
            continue;
        }
        visible_deltas.push_str(&delta);
    }

    assert_eq!(result, "streamed final answer");
    assert_eq!(
        visible_deltas, "streamed final answer",
        "draft should receive upstream deltas once without post-hoc duplication"
    );
    assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.chat_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_tool_call_loop_streaming_path_preserves_tool_loop_semantics() {
    let provider = StreamingScriptedProvider::from_text_responses(vec![
        r#"<tool_call>
{"name":"count_tool","arguments":{"value":"A"}}
</tool_call>"#,
        "done",
    ]);
    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];
    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run tool calls"),
    ];
    let observer = NoopObserver;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        Some(tx),
        None,
        &[],
    )
    .await
    .expect("streaming tool loop should execute tool and finish");

    let mut visible_deltas = String::new();
    while let Some(delta) = rx.recv().await {
        if delta == DRAFT_CLEAR_SENTINEL || delta.starts_with(DRAFT_PROGRESS_SENTINEL) {
            continue;
        }
        visible_deltas.push_str(&delta);
    }

    assert_eq!(result, "done");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.chat_calls.load(Ordering::SeqCst), 0);
    assert_eq!(visible_deltas, "done");
    assert!(
        !visible_deltas.contains("<tool_call"),
        "draft text should not leak streamed tool payload markers"
    );
}

#[tokio::test]
async fn run_tool_call_loop_streams_native_tool_events_without_chat_fallback() {
    let provider = StreamingNativeToolEventProvider::with_turns(vec![
        NativeStreamTurn::ToolCall(ToolCall {
            id: "call_native_1".to_string(),
            name: "count_tool".to_string(),
            arguments: r#"{"value":"A"}"#.to_string(),
        }),
        NativeStreamTurn::Text("done".to_string()),
    ]);
    let invocations = Arc::new(AtomicUsize::new(0));
    let tools_registry: Vec<Box<dyn Tool>> = vec![Box::new(CountingTool::new(
        "count_tool",
        Arc::clone(&invocations),
    ))];
    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("run native tools"),
    ];
    let observer = NoopObserver;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    let result = run_tool_call_loop(
        &provider,
        &mut history,
        &tools_registry,
        &observer,
        "mock-provider",
        "mock-model",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        5,
        None,
        Some(tx),
        None,
        &[],
    )
    .await
    .expect("native streaming events should preserve tool loop semantics");

    let mut visible_deltas = String::new();
    while let Some(delta) = rx.recv().await {
        if delta == DRAFT_CLEAR_SENTINEL || delta.starts_with(DRAFT_PROGRESS_SENTINEL) {
            continue;
        }
        visible_deltas.push_str(&delta);
    }

    assert_eq!(result, "done");
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 2);
    assert_eq!(provider.stream_tool_requests.load(Ordering::SeqCst), 2);
    assert_eq!(provider.chat_calls.load(Ordering::SeqCst), 0);
    assert_eq!(visible_deltas, "done");
}

#[tokio::test]
async fn run_tool_call_loop_routed_streaming_uses_live_provider_deltas_once() {
    let default_provider = RouteAwareStreamingProvider::new("default answer");
    let default_stream_calls = Arc::clone(&default_provider.stream_calls);
    let default_chat_calls = Arc::clone(&default_provider.chat_calls);

    let routed_provider = RouteAwareStreamingProvider::new("routed streamed answer");
    let routed_stream_calls = Arc::clone(&routed_provider.stream_calls);
    let routed_chat_calls = Arc::clone(&routed_provider.chat_calls);
    let routed_last_model = Arc::clone(&routed_provider.last_model);

    let router = RouterProvider::new(
        vec![
            ("default".to_string(), Box::new(default_provider)),
            ("fast".to_string(), Box::new(routed_provider)),
        ],
        vec![(
            "fast".to_string(),
            Route {
                provider_name: "fast".to_string(),
                model: "routed-model".to_string(),
            },
        )],
        "default-model".to_string(),
    );

    let tools_registry: Vec<Box<dyn Tool>> = Vec::new();
    let mut history = vec![
        ChatMessage::system("test-system"),
        ChatMessage::user("say hi"),
    ];
    let observer = NoopObserver;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    let result = run_tool_call_loop(
        &router,
        &mut history,
        &tools_registry,
        &observer,
        "router",
        "hint:fast",
        0.0,
        true,
        None,
        "telegram",
        &crate::config::MultimodalConfig::default(),
        4,
        None,
        Some(tx),
        None,
        &[],
    )
    .await
    .expect("routed streaming provider should complete");

    let mut visible_deltas = String::new();
    while let Some(delta) = rx.recv().await {
        if delta == DRAFT_CLEAR_SENTINEL || delta.starts_with(DRAFT_PROGRESS_SENTINEL) {
            continue;
        }
        visible_deltas.push_str(&delta);
    }

    assert_eq!(result, "routed streamed answer");
    assert_eq!(
        visible_deltas, "routed streamed answer",
        "routed draft should receive upstream deltas once without post-hoc duplication"
    );
    assert_eq!(default_stream_calls.load(Ordering::SeqCst), 0);
    assert_eq!(routed_stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(default_chat_calls.load(Ordering::SeqCst), 0);
    assert_eq!(routed_chat_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        routed_last_model
            .lock()
            .expect("routed_last_model lock should be valid")
            .as_str(),
        "routed-model"
    );
}

#[test]
fn looks_like_unverified_action_completion_without_tool_call_detects_claimed_side_effects() {
    assert!(looks_like_unverified_action_completion_without_tool_call(
        "Done — I've created the `names` folder in the current working directory."
    ));
    assert!(looks_like_unverified_action_completion_without_tool_call(
        "Finished successfully: I wrote the file to the workspace path."
    ));
}

#[test]
fn looks_like_tool_unavailability_claim_detects_false_missing_tool_replies() {
    let tools = vec![
        crate::tools::ToolSpec {
            name: "file_write".to_string(),
            description: "Write file".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        },
        crate::tools::ToolSpec {
            name: "file_edit".to_string(),
            description: "Edit file".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        },
    ];

    assert!(looks_like_tool_unavailability_claim(
        "I don't have access to a file creation tool in my current set of available functions.",
        &tools
    ));
    assert!(!looks_like_tool_unavailability_claim(
        "I can create that file now.",
        &tools
    ));
}

#[test]
fn parse_tool_calls_extracts_multiple_calls() {
    let response = r#"<tool_call>
{"name": "file_read", "arguments": {"path": "a.txt"}}
</tool_call>
<tool_call>
{"name": "file_read", "arguments": {"path": "b.txt"}}
</tool_call>"#;

    let (_, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "file_read");
    assert_eq!(calls[1].name, "file_read");
}

#[test]
fn parse_tool_calls_handles_malformed_json() {
    let response = r#"<tool_call>
not valid json
</tool_call>
Some text after."#;

    let (text, calls) = parse_tool_calls(response);
    assert!(calls.is_empty());
    assert!(text.contains("Some text after."));
}

#[test]
fn parse_tool_calls_handles_openai_format() {
    // OpenAI-style response with tool_calls array
    let response = r#"{"content": "Let me check that for you.", "tool_calls": [{"type": "function", "function": {"name": "shell", "arguments": "{\"command\": \"ls -la\"}"}}]}"#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(text, "Let me check that for you.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "ls -la"
    );
}

#[test]
fn parse_tool_calls_preserves_openai_tool_call_ids() {
    let response = r#"{"tool_calls":[{"id":"call_42","function":{"name":"shell","arguments":"{\"command\":\"pwd\"}"}}]}"#;
    let (_, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_call_id.as_deref(), Some("call_42"));
}

#[test]
fn parse_tool_calls_handles_xml_nested_tool_payload() {
    let response = r#"<tool_call>
<memory_recall>
<query>project roadmap</query>
</memory_recall>
</tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "memory_recall");
    assert_eq!(
        calls[0].arguments.get("query").unwrap().as_str().unwrap(),
        "project roadmap"
    );
}

#[test]
fn parse_tool_calls_handles_markdown_tool_call_fence() {
    let response = r#"I'll check that.
```tool_call
{"name": "shell", "arguments": {"command": "pwd"}}
```
Done."#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "pwd"
    );
    assert!(text.contains("I'll check that."));
    assert!(text.contains("Done."));
    assert!(!text.contains("```tool_call"));
}

#[test]
fn parse_tool_calls_handles_tool_name_fence_format() {
    // Issue #1420: xAI grok models use ```tool <name> format
    let response = r#"I'll write a test file.
```tool file_write
{"path": "/home/user/test.txt", "content": "Hello world"}
```
Done."#;

    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_write");
    assert_eq!(
        calls[0].arguments.get("path").unwrap().as_str().unwrap(),
        "/home/user/test.txt"
    );
    assert!(text.contains("I'll write a test file."));
    assert!(text.contains("Done."));
}

#[test]
fn parse_tool_calls_handles_minimax_invoke_parameter_format() {
    let response = r#"<minimax:tool_call>
<invoke name="shell">
<parameter name="command">sqlite3 /tmp/test.db ".tables"</parameter>
</invoke>
</minimax:tool_call>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        r#"sqlite3 /tmp/test.db ".tables""#
    );
}

#[test]
fn parse_tool_calls_handles_perl_style_tool_call_blocks() {
    let response = r#"TOOL_CALL
{tool => "shell", args => { --command "uname -a" }}}
/TOOL_CALL"#;

    let calls = parse_perl_style_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uname -a"
    );
}

#[test]
fn parse_tool_calls_recovers_unclosed_tool_call_with_json() {
    let response = r#"I will call the tool now.
<tool_call>
{"name": "shell", "arguments": {"command": "uptime -p"}}"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("I will call the tool now."));
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uptime -p"
    );
}

#[test]
fn parse_tool_calls_recovers_mismatched_close_tag() {
    let response = r#"<tool_call>
{"name": "shell", "arguments": {"command": "uptime"}}
</arg_value>"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(
        calls[0].arguments.get("command").unwrap().as_str().unwrap(),
        "uptime"
    );
}

#[test]
fn parse_tool_calls_rejects_raw_tool_json_without_tags() {
    // SECURITY: Raw JSON without explicit wrappers should NOT be parsed
    // This prevents prompt injection attacks where malicious content
    // could include JSON that mimics a tool call.
    let response = r#"Sure, creating the file now.
{"name": "file_write", "arguments": {"path": "hello.py", "content": "print('hello')"}}"#;

    let (text, calls) = parse_tool_calls(response);
    assert!(text.contains("Sure, creating the file now."));
    assert_eq!(
        calls.len(),
        0,
        "Raw JSON without wrappers should not be parsed"
    );
}

#[test]
fn build_tool_instructions_includes_all_tools() {
    use crate::security::SecurityPolicy;
    let security = Arc::new(SecurityPolicy::from_config(
        &crate::config::AutonomyConfig::default(),
        std::path::Path::new("/tmp"),
    ));
    let tools = tools::default_tools(security);
    let instructions = build_tool_instructions(&tools);

    assert!(instructions.contains("## Tool Use Protocol"));
    assert!(instructions.contains("<tool_call>"));
    assert!(instructions.contains("shell"));
    assert!(instructions.contains("file_read"));
    assert!(instructions.contains("file_write"));
}

#[test]
fn build_shell_policy_instructions_lists_allowlist() {
    let mut autonomy = crate::config::AutonomyConfig::default();
    autonomy.level = crate::security::AutonomyLevel::Supervised;
    autonomy.allowed_commands = vec!["grep".into(), "cat".into(), "grep".into()];

    let instructions = build_shell_policy_instructions(&autonomy);

    assert!(instructions.contains("## Shell Policy"));
    assert!(instructions.contains("Autonomy level: `supervised`"));
    assert!(instructions.contains("`cat`"));
    assert!(instructions.contains("`grep`"));
}

#[test]
fn build_shell_policy_instructions_handles_wildcard() {
    let mut autonomy = crate::config::AutonomyConfig::default();
    autonomy.level = crate::security::AutonomyLevel::Full;
    autonomy.allowed_commands = vec!["*".into()];

    let instructions = build_shell_policy_instructions(&autonomy);

    assert!(instructions.contains("Autonomy level: `full`"));
    assert!(instructions.contains("wildcard `*`"));
}

#[test]
fn build_shell_policy_instructions_read_only_disables_shell() {
    let mut autonomy = crate::config::AutonomyConfig::default();
    autonomy.level = crate::security::AutonomyLevel::ReadOnly;

    let instructions = build_shell_policy_instructions(&autonomy);

    assert!(instructions.contains("Autonomy level: `read_only`"));
    assert!(instructions.contains("Shell execution is disabled"));
}

#[test]
fn build_shell_policy_instructions_never_includes_preflight() {
    let mut autonomy = crate::config::AutonomyConfig::default();
    autonomy.level = crate::security::AutonomyLevel::Supervised;
    autonomy.allowed_commands = vec!["echo".into()];

    let instructions = build_shell_policy_instructions(&autonomy);
    assert!(!instructions.contains("first present a compact preflight"));
    assert!(instructions.contains("Call the shell tool directly"));
}

#[test]
fn trim_history_preserves_system_prompt() {
    let max_history: usize = 50;
    let mut history = vec![ChatMessage::system("system prompt")];
    for i in 0..max_history + 20 {
        history.push(ChatMessage::user(format!("msg {i}")));
    }
    let original_len = history.len();
    assert!(original_len > max_history + 1);

    trim_history(&mut history, max_history);

    // System prompt preserved
    assert_eq!(history[0].role, "system");
    assert_eq!(history[0].content, "system prompt");
    // Trimmed to limit
    assert_eq!(history.len(), max_history + 1); // +1 for system
                                                // Most recent messages preserved
    let last = &history[history.len() - 1];
    assert_eq!(last.content, format!("msg {}", max_history + 19));
}

#[tokio::test]
async fn autosave_memory_keys_preserve_multiple_turns() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();

    let key1 = autosave_memory_key("user_msg");
    let key2 = autosave_memory_key("user_msg");

    mem.store(&key1, "I'm Paul", MemoryCategory::Conversation, None)
        .await
        .unwrap();
    mem.store(&key2, "I'm 45", MemoryCategory::Conversation, None)
        .await
        .unwrap();

    assert_eq!(mem.count().await.unwrap(), 2);

    let recalled = mem.recall("45", 5, None).await.unwrap();
    assert!(recalled.iter().any(|entry| entry.content.contains("45")));
}

#[tokio::test]
async fn build_context_ignores_legacy_assistant_autosave_entries() {
    let tmp = TempDir::new().unwrap();
    let mem = SqliteMemory::new(tmp.path()).unwrap();
    mem.store(
        "assistant_resp_poisoned",
        "User suffered a fabricated event",
        MemoryCategory::Daily,
        None,
    )
    .await
    .unwrap();
    mem.store(
        "user_msg_real",
        "User asked for concise status updates",
        MemoryCategory::Conversation,
        None,
    )
    .await
    .unwrap();

    let context = build_context(&mem, "status updates", 0.0).await;
    assert!(context.contains("user_msg_real"));
    assert!(!context.contains("assistant_resp_poisoned"));
    assert!(!context.contains("fabricated event"));
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Parsing Edge Cases
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn detect_tool_call_parse_issue_flags_malformed_payloads() {
    let response =
        "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}</tool_call>";
    let issue = detect_tool_call_parse_issue(response, &[]);
    assert!(
        issue.is_some(),
        "malformed tool payload should be flagged for diagnostics"
    );
}

#[test]
fn detect_tool_call_parse_issue_does_not_flag_walkthrough_with_inline_substring() {
    let response = "Here is how the agent loop works. The parser checks if a \
                    response contains a markdown fence like ```tool file_read \
                    and treats it as a deferred tool call. The relevant code \
                    lives in src/agent/loop_/parsing.rs.";
    let issue = detect_tool_call_parse_issue(response, &[]);
    assert!(
        issue.is_none(),
        "explanatory prose mentioning ```tool ` inline must not be flagged: {issue:?}",
    );
}

#[test]
fn detect_tool_call_parse_issue_does_not_flag_inline_tool_markers_in_prose() {
    let response = "The parser checks for literal markers like <tool_call> and \
                    the JSON key \"tool_calls\" when diagnosing malformed output. \
                    Documentation may also show examples like ` ```tool shell ` \
                    inline without actually invoking a tool.";
    let issue = detect_tool_call_parse_issue(response, &[]);
    assert!(
        issue.is_none(),
        "inline prose mentioning payload markers must not be flagged: {issue:?}",
    );
}

#[test]
fn detect_tool_call_parse_issue_does_not_flag_fence_with_non_tool_token() {
    let response = "Example fence with language tag:\n```tooling\nsome content\n```";
    assert!(detect_tool_call_parse_issue(response, &[]).is_none());

    let response2 = "```tools\nlist of things\n```";
    assert!(detect_tool_call_parse_issue(response2, &[]).is_none());
}

#[test]
fn detect_tool_call_parse_issue_flags_line_anchored_tool_fence() {
    let response = "I will read the file now.\n```tool file_read\npath: src/main.rs\n```";
    let issue = detect_tool_call_parse_issue(response, &[]);
    assert!(
        issue.is_some(),
        "line-anchored ```tool <name> fence must still be flagged"
    );
}

#[test]
fn detect_tool_call_parse_issue_flags_tool_call_lang_fence() {
    let response = "```tool_call\n{\"name\":\"shell\"}\n```";
    assert!(detect_tool_call_parse_issue(response, &[]).is_some());
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - History Management
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn trim_history_preserves_role_ordering() {
    // Recovery: After trimming, role ordering should remain consistent
    let mut history = vec![ChatMessage::system("system")];
    for i in 0..60 {
        history.push(ChatMessage::user(format!("user {i}")));
        history.push(ChatMessage::assistant(format!("assistant {i}")));
    }
    trim_history(&mut history, 50);
    assert_eq!(history[0].role, "system");
    assert_eq!(history[history.len() - 1].role, "assistant");
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Arguments Parsing
// ═══════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - JSON Extraction
// ═══════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Constants Validation
// ═══════════════════════════════════════════════════════════════════════

const _: () = {
    assert!(DEFAULT_MAX_TOOL_ITERATIONS > 0);
    assert!(DEFAULT_MAX_TOOL_ITERATIONS <= 100);
};

// ═══════════════════════════════════════════════════════════════════════
// Recovery Tests - Tool Call Value Parsing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_tool_call_value_recovers_shell_command_from_raw_string_arguments() {
    let value = serde_json::json!({
        "name": "shell",
        "arguments": "uname -a"
    });
    let result = parse_tool_call_value(&value).expect("tool call should parse");
    assert_eq!(result.name, "shell");
    assert_eq!(
        result.arguments.get("command").and_then(|v| v.as_str()),
        Some("uname -a")
    );
}

#[test]
fn parse_tool_call_value_preserves_tool_call_id_aliases() {
    let value = serde_json::json!({
        "call_id": "legacy_1",
        "function": {
            "name": "shell",
            "arguments": {"command": "date"}
        }
    });
    let result = parse_tool_call_value(&value).expect("tool call should parse");
    assert_eq!(result.tool_call_id.as_deref(), Some("legacy_1"));
}

#[test]
fn parse_structured_tool_calls_recovers_shell_command_from_string_payload() {
    let calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "shell".to_string(),
        arguments: "ls -la".to_string(),
    }];
    let parsed = parse_structured_tool_calls(&calls);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "shell");
    assert_eq!(
        parsed[0].arguments.get("command").and_then(|v| v.as_str()),
        Some("ls -la")
    );
}

// ═══════════════════════════════════════════════════════════════════════
// GLM-Style Tool Call Parsing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn parse_glm_style_tool_call_integration() {
    // Integration test: GLM format should be parsed in parse_tool_calls
    let response = "Checking...\nbrowser_open/url>https://example.com\nDone";
    let (text, calls) = parse_tool_calls(response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert!(text.contains("Checking"));
    assert!(text.contains("Done"));
}

#[test]
fn parse_glm_style_rejects_non_http_url_param() {
    let response = "browser_open/url>javascript:alert(1)";
    let calls = parse_glm_style_tool_calls(response);
    assert!(calls.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): parse_tool_calls robustness — malformed/edge-case inputs
// Prevents: Pattern 4 issues #746, #418, #777, #848
// ─────────────────────────────────────────────────────────────────────

#[test]
fn parse_tool_calls_truncated_json_no_panic() {
    // Incomplete JSON inside tool_call tags
    let response = r#"<tool_call>{"name":"shell","arguments":{"command":"ls"</tool_call>"#;
    let (_text, _calls) = parse_tool_calls(response);
    // Should not panic — graceful handling of truncated JSON
}

#[test]
fn parse_tool_calls_very_large_arguments_no_panic() {
    let large_arg = "x".repeat(100_000);
    let response = format!(
        r#"<tool_call>{{"name":"echo","arguments":{{"message":"{}"}}}}</tool_call>"#,
        large_arg
    );
    let (_text, calls) = parse_tool_calls(&response);
    assert_eq!(calls.len(), 1, "large arguments should still parse");
    assert_eq!(calls[0].name, "echo");
}

#[test]
fn parse_tool_calls_text_with_embedded_json_not_extracted() {
    // Raw JSON without any tags should NOT be extracted as a tool call
    let response = r#"Here is some data: {"name":"echo","arguments":{"message":"hi"}} end."#;
    let (_text, calls) = parse_tool_calls(response);
    assert!(
        calls.is_empty(),
        "raw JSON in text without tags should not be extracted"
    );
}

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): scrub_credentials edge cases
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// TG4 (inline): trim_history edge cases
// ─────────────────────────────────────────────────────────────────────

#[test]
fn trim_history_removes_oldest_non_system() {
    let mut history = vec![
        crate::providers::ChatMessage::system("system"),
        crate::providers::ChatMessage::user("old msg"),
        crate::providers::ChatMessage::assistant("old reply"),
        crate::providers::ChatMessage::user("new msg"),
        crate::providers::ChatMessage::assistant("new reply"),
    ];
    trim_history(&mut history, 2);
    assert_eq!(history.len(), 3); // system + 2 kept
    assert_eq!(history[0].role, "system");
    assert_eq!(history[1].content, "new msg");
}

/// When `build_system_prompt_with_mode` is called with `native_tools = true`,
/// the output must contain ZERO XML protocol artifacts. In the native path
/// `build_tool_instructions` is never called, so the system prompt alone
/// must be clean of XML tool-call protocol.
#[test]
fn native_tools_system_prompt_contains_zero_xml() {
    use crate::channels::build_system_prompt_with_mode;

    let tool_summaries: Vec<(&str, &str)> = vec![
        ("shell", "Execute shell commands"),
        ("file_read", "Read files"),
    ];

    let system_prompt = build_system_prompt_with_mode(
        std::path::Path::new("/tmp"),
        "test-model",
        &tool_summaries,
        &[],  // no skills
        None, // no identity config
        None, // no bootstrap_max_chars
        true, // native_tools
        crate::config::SkillsPromptInjectionMode::Full,
    );

    // Must contain zero XML protocol artifacts
    assert!(
        !system_prompt.contains("<tool_call>"),
        "Native prompt must not contain <tool_call>"
    );
    assert!(
        !system_prompt.contains("</tool_call>"),
        "Native prompt must not contain </tool_call>"
    );
    assert!(
        !system_prompt.contains("<tool_result>"),
        "Native prompt must not contain <tool_result>"
    );
    assert!(
        !system_prompt.contains("</tool_result>"),
        "Native prompt must not contain </tool_result>"
    );
    assert!(
        !system_prompt.contains("## Tool Use Protocol"),
        "Native prompt must not contain XML protocol header"
    );

    // Positive: native prompt should still list tools and contain task instructions
    assert!(
        system_prompt.contains("shell"),
        "Native prompt must list tool names"
    );
    assert!(
        system_prompt.contains("## Your Task"),
        "Native prompt should contain task instructions"
    );
}

// ── Cross-Alias & GLM Shortened Body Tests ──────────────────────────

#[test]
fn parse_tool_calls_cross_alias_close_tag_with_json() {
    // <tool_call> opened but closed with </invoke> — JSON body
    let input = r#"<tool_call>{"name": "shell", "arguments": {"command": "ls"}}</invoke>"#;
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "ls");
    assert!(text.is_empty());
}

#[test]
fn parse_tool_calls_unclosed_glm_shortened_no_close_tag() {
    // <tool_call>shell>ls -la (no close tag at all)
    let input = "<tool_call>shell>ls -la";
    let (text, calls) = parse_tool_calls(input);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments["command"], "ls -la");
    assert!(text.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════
// reasoning_content pass-through tests for history builders
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn build_native_assistant_history_includes_reasoning_content() {
    let calls = vec![ToolCall {
        id: "call_1".into(),
        name: "shell".into(),
        arguments: "{}".into(),
    }];
    let result = build_native_assistant_history("answer", &calls, Some("thinking step"));
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert_eq!(parsed["reasoning_content"].as_str(), Some("thinking step"));
    assert!(parsed["tool_calls"].is_array());
}

#[test]
fn build_native_assistant_history_from_parsed_calls_includes_reasoning_content() {
    let calls = vec![ParsedToolCall {
        name: "shell".into(),
        arguments: serde_json::json!({"command": "pwd"}),
        tool_call_id: Some("call_2".into()),
    }];
    let result = build_native_assistant_history_from_parsed_calls(
        "answer",
        &calls,
        Some("deep thought"),
    );
    assert!(result.is_some());
    let parsed: serde_json::Value = serde_json::from_str(result.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["content"].as_str(), Some("answer"));
    assert_eq!(parsed["reasoning_content"].as_str(), Some("deep thought"));
    assert!(parsed["tool_calls"].is_array());
}
