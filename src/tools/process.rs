use super::shell::collect_allowed_shell_env_vars;
use super::traits::{Tool, ToolResult};
use crate::runtime::RuntimeAdapter;
use crate::security::policy::ToolOperation;
use crate::security::SecurityPolicy;
use crate::security::SyscallAnomalyDetector;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

/// Maximum output bytes kept per stream (stdout/stderr): 512KB.
const MAX_OUTPUT_BYTES: usize = 524_288;

/// Maximum concurrent background processes.
const MAX_PROCESSES: usize = 8;

#[derive(Debug, Default, Clone)]
struct OutputBuffer {
    data: String,
    dropped_prefix_bytes: u64,
}

struct ProcessEntry {
    id: usize,
    command: String,
    pid: u32,
    started_at: Instant,
    child: Mutex<tokio::process::Child>,
    stdout_buf: Arc<Mutex<OutputBuffer>>,
    stderr_buf: Arc<Mutex<OutputBuffer>>,
    analyzed_offsets: Mutex<(u64, u64)>,
}

/// Background process management tool.
///
/// Allows the agent to spawn long-running commands, check their output,
/// and terminate them. Complements the synchronous `ShellTool` for commands
/// that need to run beyond the 60-second shell timeout.
pub struct ProcessTool {
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    syscall_detector: Option<Arc<SyscallAnomalyDetector>>,
    processes: Arc<RwLock<HashMap<usize, ProcessEntry>>>,
    next_id: Mutex<usize>,
}

impl ProcessTool {
    pub fn new(security: Arc<SecurityPolicy>, runtime: Arc<dyn RuntimeAdapter>) -> Self {
        Self::new_with_syscall_detector(security, runtime, None)
    }

    pub fn new_with_syscall_detector(
        security: Arc<SecurityPolicy>,
        runtime: Arc<dyn RuntimeAdapter>,
        syscall_detector: Option<Arc<SyscallAnomalyDetector>>,
    ) -> Self {
        Self {
            security,
            runtime,
            syscall_detector,
            processes: Arc::new(RwLock::new(HashMap::new())),
            next_id: Mutex::new(0),
        }
    }

    fn handle_spawn(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let program = args
            .get("program")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'program' parameter for spawn action"))?;
        let argv = parse_process_args(args)?;
        let approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if let Some(result) = self.prepare_spawn_failure_result() {
            return Ok(result);
        }

        if let Err(reason) = self.security.validate_command_execution(program, approved) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(reason),
            });
        }

        if let Some(path) = self.security.forbidden_path_argv(&argv) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Path blocked by security policy: {path}")),
            });
        }

        let cmd =
            match self
                .runtime
                .build_exec_command(program, &argv, &self.security.workspace_dir)
            {
                Ok(cmd) => cmd,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Failed to build runtime exec command: {e}")),
                    });
                }
            };

        let display = format_process_command(program, &argv);
        self.spawn_prepared_command(cmd, display.clone(), format!("Process started: {display}"))
    }

    fn handle_spawn_shell(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let command = args
            .get("shell_command")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("command").and_then(|v| v.as_str()))
            .ok_or_else(|| {
                anyhow::anyhow!("Missing 'shell_command' parameter for spawn_shell action")
            })?;
        let approved = args
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !approved {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "spawn_shell requires explicit approval (`approved=true`) because it executes through the runtime shell".into(),
                ),
            });
        }

        if let Some(result) = self.prepare_spawn_failure_result() {
            return Ok(result);
        }

        if let Err(reason) = self.security.validate_command_execution(command, approved) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(reason),
            });
        }

        let effective_command = self.security.apply_shell_redirect_policy(command);
        if let Some(path) = self.security.forbidden_path_argument(&effective_command) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Path blocked by security policy: {path}")),
            });
        }

        let cmd = match self
            .runtime
            .build_shell_command(&effective_command, &self.security.workspace_dir)
        {
            Ok(cmd) => cmd,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to build runtime shell command: {e}")),
                });
            }
        };

        self.spawn_prepared_command(
            cmd,
            command.to_string(),
            format!("Shell process started: {command}"),
        )
    }

    fn prepare_spawn_failure_result(&self) -> Option<ToolResult> {
        if !self.runtime.supports_long_running() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Runtime does not support long-running processes".into()),
            });
        }

        self.prune_exited_processes();

        let processes = self.processes.read().unwrap();
        let running = processes
            .values()
            .filter(|e| {
                e.child
                    .lock()
                    .map(|mut c| matches!(c.try_wait(), Ok(None)))
                    .unwrap_or(false)
            })
            .count();
        if running >= MAX_PROCESSES {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Maximum concurrent processes ({MAX_PROCESSES}) reached"
                )),
            });
        }

        if self.security.is_rate_limited() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: too many actions in the last hour".into()),
            });
        }

        if !self.security.record_action() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: action budget exhausted".into()),
            });
        }

        None
    }

    fn spawn_prepared_command(
        &self,
        mut cmd: tokio::process::Command,
        command_label: String,
        success_message: String,
    ) -> anyhow::Result<ToolResult> {
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.env_clear();

        for var in collect_allowed_shell_env_vars(&self.security) {
            if let Ok(val) = std::env::var(&var) {
                cmd.env(&var, val);
            }
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to spawn process: {e}")),
                });
            }
        };

        let Some(pid) = child.id() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "Failed to capture process PID for spawned child; process was not tracked"
                        .into(),
                ),
            });
        };

        let stdout_buf = Arc::new(Mutex::new(OutputBuffer::default()));
        let stderr_buf = Arc::new(Mutex::new(OutputBuffer::default()));

        if let Some(stdout) = child.stdout.take() {
            spawn_reader_task(stdout, stdout_buf.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_reader_task(stderr, stderr_buf.clone());
        }

        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };

        let entry = ProcessEntry {
            id,
            command: command_label,
            pid,
            started_at: Instant::now(),
            child: Mutex::new(child),
            stdout_buf,
            stderr_buf,
            analyzed_offsets: Mutex::new((0, 0)),
        };

        self.processes.write().unwrap().insert(id, entry);

        Ok(ToolResult {
            success: true,
            output: json!({
                "id": id,
                "pid": pid,
                "message": success_message
            })
            .to_string(),
            error: None,
        })
    }

    fn handle_list(&self) -> anyhow::Result<ToolResult> {
        if let Err(e) = self
            .security
            .enforce_tool_operation(ToolOperation::Read, "process")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e),
            });
        }

        self.prune_exited_processes();

        let processes = self.processes.read().unwrap();
        let mut entries = Vec::new();

        for entry in processes.values() {
            let status = match entry.child.lock() {
                Ok(mut child) => match child.try_wait() {
                    Ok(Some(status)) => {
                        format!("exited ({})", status.code().unwrap_or(-1))
                    }
                    Ok(None) => "running".to_string(),
                    Err(e) => format!("error: {e}"),
                },
                Err(_) => "unknown".to_string(),
            };

            entries.push(json!({
                "id": entry.id,
                "command": entry.command,
                "pid": entry.pid,
                "status": status,
                "uptime_secs": entry.started_at.elapsed().as_secs(),
            }));
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&entries).unwrap_or_default(),
            error: None,
        })
    }

    fn handle_output(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Err(e) = self
            .security
            .enforce_tool_operation(ToolOperation::Read, "process")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e),
            });
        }

        let id = parse_id(args, "output")?;

        let processes = self.processes.read().unwrap();
        let entry = match processes.get(&id) {
            Some(e) => e,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("No process with id {id}")),
                });
            }
        };

        let stdout_snapshot = snapshot_output_buffer(&entry.stdout_buf);
        let stderr_snapshot = snapshot_output_buffer(&entry.stderr_buf);
        let stdout = stdout_snapshot.data;
        let stderr = stderr_snapshot.data;

        if let Some(detector) = &self.syscall_detector {
            let mut offsets = entry.analyzed_offsets.lock().unwrap();
            let stdout_delta = slice_unseen_output(
                &stdout,
                stdout_snapshot.dropped_prefix_bytes,
                &mut offsets.0,
            );
            let stderr_delta = slice_unseen_output(
                &stderr,
                stderr_snapshot.dropped_prefix_bytes,
                &mut offsets.1,
            );

            if !stdout_delta.is_empty() || !stderr_delta.is_empty() {
                let _ = detector.inspect_command_output(
                    &entry.command,
                    stdout_delta,
                    stderr_delta,
                    None,
                );
            }
        }

        Ok(ToolResult {
            success: true,
            output: json!({
                "stdout": stdout,
                "stderr": stderr,
            })
            .to_string(),
            error: None,
        })
    }

    async fn handle_kill(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Err(e) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "process")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e),
            });
        }

        let id = parse_id(args, "kill")?;

        let entry = {
            let mut processes = self.processes.write().unwrap();
            processes.remove(&id)
        };

        let Some(entry) = entry else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("No process with id {id}")),
            });
        };

        let pid = entry.pid;
        let mut child = match entry.child.into_inner() {
            Ok(child) => child,
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Ok(Some(status)) = child.try_wait() {
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "Process {id} (pid {pid}) already exited with status {:?}",
                    status.code()
                ),
                error: None,
            });
        }

        if let Err(e) = child.start_kill() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Failed to initiate termination for process {id} (pid {pid}): {e}"
                )),
            });
        }

        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => Ok(ToolResult {
                success: true,
                output: format!(
                    "Terminated process {id} (pid {pid}) with exit status {:?}",
                    status.code()
                ),
                error: None,
            }),
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed waiting for process {id} (pid {pid}) to exit: {e}")),
            }),
            Err(_) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Timed out waiting for process {id} (pid {pid}) to exit after termination signal"
                )),
            }),
        }
    }

    fn prune_exited_processes(&self) {
        let mut to_remove = Vec::new();
        {
            let processes = self.processes.read().unwrap();
            for (id, entry) in processes.iter() {
                if let Ok(mut child) = entry.child.lock() {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        to_remove.push(*id);
                    }
                }
            }
        }

        if to_remove.is_empty() {
            return;
        }

        let mut processes = self.processes.write().unwrap();
        for id in to_remove {
            processes.remove(&id);
        }
    }
}

/// Parse the `id` field from action args, returning a usize.
fn parse_id(args: &serde_json::Value, action: &str) -> anyhow::Result<usize> {
    args.get("id")
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for {action} action"))
}

fn parse_process_args(args: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let Some(raw_args) = args.get("args") else {
        return Ok(Vec::new());
    };
    let Some(items) = raw_args.as_array() else {
        anyhow::bail!("'args' must be an array of strings");
    };

    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(std::string::ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("'args' entries must be strings"))
        })
        .collect()
}

fn format_process_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// Append data to a bounded buffer, draining oldest bytes when over limit.
fn append_bounded(buf: &Mutex<OutputBuffer>, new_data: &str) {
    let mut guard = buf.lock().unwrap();
    guard.data.push_str(new_data);
    if guard.data.len() > MAX_OUTPUT_BYTES {
        let excess = guard.data.len() - MAX_OUTPUT_BYTES;
        // Find the first valid char boundary at or after `excess`.
        let mut drain_to = excess;
        while drain_to < guard.data.len() && !guard.data.is_char_boundary(drain_to) {
            drain_to += 1;
        }
        guard.data.drain(..drain_to);
        guard.dropped_prefix_bytes = guard
            .dropped_prefix_bytes
            .saturating_add(u64::try_from(drain_to).unwrap_or(u64::MAX));
    }
}

/// Spawn a background task that reads from an async reader into a bounded buffer.
fn spawn_reader_task<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    buf: Arc<Mutex<OutputBuffer>>,
) {
    tokio::spawn(async move {
        let mut chunk = vec![0u8; 8192];
        loop {
            match reader.read(&mut chunk).await {
                Ok(n) if n > 0 => {
                    let text = String::from_utf8_lossy(&chunk[..n]);
                    append_bounded(&buf, &text);
                }
                _ => break,
            }
        }
    });
}

fn snapshot_output_buffer(buf: &Mutex<OutputBuffer>) -> OutputBuffer {
    buf.lock().unwrap().clone()
}

fn slice_unseen_output<'a>(
    current: &'a str,
    dropped_prefix_bytes: u64,
    analyzed: &mut u64,
) -> &'a str {
    let len_u64 = u64::try_from(current.len()).unwrap_or(u64::MAX);
    let available_end = dropped_prefix_bytes.saturating_add(len_u64);

    let start = if *analyzed <= dropped_prefix_bytes {
        0
    } else {
        usize::try_from(analyzed.saturating_sub(dropped_prefix_bytes))
            .unwrap_or(current.len())
            .min(current.len())
    };

    let mut boundary = start;
    while boundary < current.len() && !current.is_char_boundary(boundary) {
        boundary += 1;
    }

    *analyzed = available_end;
    &current[boundary..]
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage background processes: spawn long-running commands, check output, and terminate them"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["spawn", "spawn_shell", "list", "output", "kill"],
                    "description": "Action to perform: spawn argv-safe process, spawn unsafe shell process, list all, get output, or kill"
                },
                "program": {
                    "type": "string",
                    "description": "Executable to run directly without shell parsing (required for 'spawn')"
                },
                "args": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Argument vector for direct process execution (optional for 'spawn')"
                },
                "shell_command": {
                    "type": "string",
                    "description": "Unsafe raw shell command to run in background (required for 'spawn_shell'; requires approved=true)"
                },
                "command": {
                    "type": "string",
                    "description": "Deprecated alias for shell_command when using spawn_shell or legacy spawn"
                },
                "id": {
                    "type": "integer",
                    "description": "Process ID returned by spawn (required for 'output' and 'kill')"
                },
                "approved": {
                    "type": "boolean",
                    "description": "Approve medium/high-risk execution. Required for 'spawn_shell'",
                    "default": false
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "spawn" => {
                if args.get("program").is_some() {
                    self.handle_spawn(&args)
                } else if args.get("shell_command").is_some() || args.get("command").is_some() {
                    self.handle_spawn_shell(&args)
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(
                            "spawn requires 'program' (preferred) or a legacy shell alias; use spawn_shell for explicit raw shell execution".into(),
                        ),
                    })
                }
            }
            "spawn_shell" => self.handle_spawn_shell(&args),
            "list" => self.handle_list(),
            "output" => self.handle_output(&args),
            "kill" => self.handle_kill(&args).await,
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action '{other}'. Use: spawn, spawn_shell, list, output, kill"
                )),
            }),
        }
    }
}

impl Drop for ProcessTool {
    fn drop(&mut self) {
        if let Ok(processes) = self.processes.read() {
            for entry in processes.values() {
                if let Ok(mut child) = entry.child.lock() {
                    let _ = child.start_kill();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuditConfig, SyscallAnomalyConfig};
    use crate::runtime::NativeRuntime;
    use crate::security::{AutonomyLevel, SecurityPolicy, SyscallAnomalyDetector};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn test_allowed_commands() -> Vec<String> {
        ["sleep", "echo", "cat", "grep"]
            .into_iter()
            .map(std::string::ToString::to_string)
            .collect()
    }

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: test_allowed_commands(),
            ..SecurityPolicy::default()
        })
    }

    fn test_runtime() -> Arc<dyn RuntimeAdapter> {
        Arc::new(NativeRuntime::new())
    }

    fn test_syscall_detector(tmp: &TempDir) -> Arc<SyscallAnomalyDetector> {
        let log_path = tmp.path().join("process-syscall-anomalies.log");
        let cfg = SyscallAnomalyConfig {
            baseline_syscalls: vec!["read".into(), "write".into()],
            log_path: log_path.to_string_lossy().to_string(),
            alert_cooldown_secs: 1,
            max_alerts_per_minute: 50,
            ..SyscallAnomalyConfig::default()
        };
        let audit = AuditConfig {
            enabled: false,
            ..AuditConfig::default()
        };
        Arc::new(SyscallAnomalyDetector::new(cfg, tmp.path(), audit))
    }

    fn make_tool() -> ProcessTool {
        ProcessTool::new(test_security(), test_runtime())
    }

    #[test]
    fn process_tool_name() {
        assert_eq!(make_tool().name(), "process");
    }

    #[test]
    fn process_tool_description_not_empty() {
        assert!(!make_tool().description().is_empty());
    }

    #[test]
    fn process_tool_schema_has_action() {
        let schema = make_tool().parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("action")));
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(MAX_OUTPUT_BYTES, 524_288);
        assert_eq!(MAX_PROCESSES, 8);
    }

    #[tokio::test]
    async fn spawn_starts_background_process() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["hello_process_test"]
            }))
            .await
            .unwrap();
        assert!(result.success, "spawn should succeed: {:?}", result.error);
        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(output["id"].is_number());
        assert!(output["pid"].is_number());
    }

    #[tokio::test]
    async fn list_shows_spawned_process() {
        let tool = make_tool();
        tool.execute(json!({
            "action": "spawn",
            "program": "echo",
            "args": ["list_test"]
        }))
        .await
        .unwrap();

        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("list_test"));
    }

    #[tokio::test]
    async fn list_prunes_exited_processes() {
        let tool = make_tool();
        let spawn_result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["prune_test"]
            }))
            .await
            .unwrap();
        assert!(spawn_result.success);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let list_result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(list_result.success);
        let entries: Vec<serde_json::Value> = serde_json::from_str(&list_result.output).unwrap();
        assert!(
            !entries
                .iter()
                .any(|entry| entry["command"].as_str() == Some("echo prune_test")),
            "exited process entries should be pruned during list()"
        );
    }

    #[tokio::test]
    async fn output_returns_stdout() {
        let tool = make_tool();
        let spawn_result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["output_capture_test"]
            }))
            .await
            .unwrap();

        let spawn_output: serde_json::Value = serde_json::from_str(&spawn_result.output).unwrap();
        let id = spawn_output["id"].as_u64().unwrap();

        // Wait for the process to finish and output to be captured.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let result = tool
            .execute(json!({
                "action": "output",
                "id": id
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("output_capture_test"));
    }

    #[tokio::test]
    async fn kill_terminates_process() {
        let tool = make_tool();
        let spawn_result = tool
            .execute(json!({
                "action": "spawn",
                "program": "sleep",
                "args": ["60"]
            }))
            .await
            .unwrap();
        assert!(spawn_result.success);

        let spawn_output: serde_json::Value = serde_json::from_str(&spawn_result.output).unwrap();
        let id = spawn_output["id"].as_u64().unwrap();

        let kill_result = tool
            .execute(json!({
                "action": "kill",
                "id": id
            }))
            .await
            .unwrap();
        assert!(kill_result.success);
    }

    #[tokio::test]
    async fn kill_removes_process_entry() {
        let tool = make_tool();
        let spawn_result = tool
            .execute(json!({
                "action": "spawn",
                "program": "sleep",
                "args": ["60"]
            }))
            .await
            .unwrap();
        assert!(spawn_result.success);

        let spawn_output: serde_json::Value = serde_json::from_str(&spawn_result.output).unwrap();
        let id = spawn_output["id"].as_u64().unwrap();

        let kill_result = tool
            .execute(json!({
                "action": "kill",
                "id": id
            }))
            .await
            .unwrap();
        assert!(kill_result.success);

        let list_result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(list_result.success);
        let entries: Vec<serde_json::Value> = serde_json::from_str(&list_result.output).unwrap();
        assert!(
            !entries.iter().any(|entry| entry["id"].as_u64() == Some(id)),
            "killed process should no longer be listed"
        );
    }

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let tool = make_tool();
        let result = tool.execute(json!({"action": "restart"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Unknown action"));
    }

    #[tokio::test]
    async fn spawn_blocks_disallowed_command() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn_shell",
                "command": "rm -rf /",
                "approved": true
            }))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn spawn_shell_blocks_forbidden_path() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn_shell",
                "command": "cat /etc/passwd",
                "approved": true
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Path blocked"));
    }

    #[tokio::test]
    async fn spawn_blocks_forbidden_path_arg() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "cat",
                "args": ["/etc/passwd"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Path blocked"));
    }

    #[tokio::test]
    async fn spawn_blocks_forbidden_option_assignment_path_arg() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "grep",
                "args": ["--file=/etc/passwd", "root", "./src"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Path blocked"));
    }

    #[tokio::test]
    async fn spawn_blocks_forbidden_attached_short_option_path_arg() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "grep",
                "args": ["-f/etc/passwd", "root", "./src"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Path blocked"));
    }

    #[tokio::test]
    async fn kill_blocks_readonly() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ProcessTool::new(security, test_runtime());
        let result = tool
            .execute(json!({
                "action": "kill",
                "id": 0
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("read-only"));
    }

    #[tokio::test]
    async fn output_missing_id_returns_error() {
        let tool = make_tool();
        let result = tool.execute(json!({"action": "output"})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn output_nonexistent_id_returns_error() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "output",
                "id": 9999
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("No process"));
    }

    #[tokio::test]
    async fn spawn_blocks_rate_limited() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            max_actions_per_hour: 0,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ProcessTool::new(security, test_runtime());
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["test"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Rate limit"));
    }

    struct NoLongRunningRuntime;

    impl RuntimeAdapter for NoLongRunningRuntime {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn name(&self) -> &str {
            "test-restricted"
        }
        fn has_shell_access(&self) -> bool {
            true
        }
        fn has_filesystem_access(&self) -> bool {
            true
        }
        fn storage_path(&self) -> PathBuf {
            PathBuf::from("/tmp")
        }
        fn supports_long_running(&self) -> bool {
            false
        }
        fn build_shell_command(
            &self,
            command: &str,
            workspace_dir: &std::path::Path,
        ) -> anyhow::Result<tokio::process::Command> {
            let mut cmd = tokio::process::Command::new("sh");
            cmd.arg("-c").arg(command).current_dir(workspace_dir);
            Ok(cmd)
        }

        fn build_exec_command(
            &self,
            program: &str,
            args: &[String],
            workspace_dir: &std::path::Path,
        ) -> anyhow::Result<tokio::process::Command> {
            let mut cmd = tokio::process::Command::new(program);
            cmd.args(args).current_dir(workspace_dir);
            Ok(cmd)
        }
    }

    #[tokio::test]
    async fn spawn_rejects_when_runtime_unsupported() {
        let tool = ProcessTool::new(test_security(), Arc::new(NoLongRunningRuntime));
        let result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["test"]
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("long-running"));
    }

    #[test]
    fn append_bounded_truncates_old_data() {
        let buf = Mutex::new(OutputBuffer::default());
        let data = "x".repeat(MAX_OUTPUT_BYTES + 100);
        append_bounded(&buf, &data);
        let guard = buf.lock().unwrap();
        assert!(guard.data.len() <= MAX_OUTPUT_BYTES);
        assert!(guard.dropped_prefix_bytes >= 100);
    }

    #[test]
    fn slice_unseen_output_tracks_new_tail_after_rollover() {
        let mut analyzed = u64::try_from(MAX_OUTPUT_BYTES).expect("size should fit in u64");
        let current = format!("{}tail", "x".repeat(MAX_OUTPUT_BYTES.saturating_sub(4)));
        let dropped = 4_u64;

        let delta = slice_unseen_output(&current, dropped, &mut analyzed);

        assert_eq!(delta, "tail");
        assert_eq!(
            analyzed,
            dropped.saturating_add(u64::try_from(current.len()).expect("len should fit in u64"))
        );
    }

    #[tokio::test]
    async fn process_output_runs_syscall_detector_incrementally() {
        let tmp = tempfile::tempdir().expect("temp dir should be created");
        let log_path = tmp.path().join("process-syscall-anomalies.log");
        let tool = ProcessTool::new_with_syscall_detector(
            test_security(),
            test_runtime(),
            Some(test_syscall_detector(&tmp)),
        );

        let spawn_result = tool
            .execute(json!({
                "action": "spawn",
                "program": "echo",
                "args": ["seccomp denied syscall=openat"]
            }))
            .await
            .expect("spawn should return result");
        assert!(spawn_result.success);
        let spawn_output: serde_json::Value =
            serde_json::from_str(&spawn_result.output).expect("spawn output should be json");
        let id = spawn_output["id"]
            .as_u64()
            .expect("process id should exist");

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let first_output = tool
            .execute(json!({"action": "output", "id": id}))
            .await
            .expect("first output should return result");
        assert!(first_output.success);

        let first_log = tokio::fs::read_to_string(&log_path)
            .await
            .expect("first anomaly log should exist");
        let first_lines = first_log.lines().count();
        assert!(first_lines >= 1);
        assert!(first_log.contains("\"kind\":\"unknown_syscall\""));

        let second_output = tool
            .execute(json!({"action": "output", "id": id}))
            .await
            .expect("second output should return result");
        assert!(second_output.success);

        let second_log = tokio::fs::read_to_string(&log_path)
            .await
            .expect("second anomaly log should still exist");
        let second_lines = second_log.lines().count();
        assert_eq!(
            second_lines, first_lines,
            "incremental offsets should prevent duplicate detector emissions for unchanged output"
        );
    }

    #[tokio::test]
    async fn spawn_shell_requires_explicit_approval() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn_shell",
                "shell_command": "echo unsafe"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .contains("requires explicit approval"));
    }

    #[tokio::test]
    async fn legacy_spawn_command_alias_routes_to_shell_mode() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "spawn",
                "command": "echo legacy_shell_mode",
                "approved": true
            }))
            .await
            .unwrap();
        assert!(result.success);
        let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(output["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Shell process started"));
    }
}
