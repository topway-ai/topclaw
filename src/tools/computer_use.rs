//! Provider-agnostic `computer_use` tool.
//!
//! Single tool exposing the full sidecar action surface (launch apps, focus
//! windows, screenshot, click, type) through one flat JSON schema. Works with
//! any LLM that supports function calling — screenshots are returned as a file
//! path plus text summary so non-vision providers still get useful output.
//!
//! On first use, if the sidecar is not answering `/health`, the tool will
//! spawn the built-in sidecar (same binary, `computer-use-sidecar` subcommand)
//! and poll until ready. This keeps the happy path one tool call, not two.

use super::bootstrap;
use super::sidecar_client;
use super::traits::{Tool, ToolResult};
use crate::config::BrowserComputerUseConfig;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

const ACTIONS: &[&str] = &[
    "screen_capture",
    "window_list",
    "window_focus",
    "window_close",
    "app_launch",
    "app_terminate",
    "mouse_move",
    "mouse_click",
    "mouse_drag",
    "key_type",
    "key_press",
    "bootstrap",
];

/// Which helpers each action requires. Actions not listed (bootstrap, app_launch,
/// app_terminate, screen_capture) don't need pre-flight helpers. screen_capture
/// falls back inside the sidecar, so blocking on scrot would false-reject when
/// only gnome-screenshot is installed.
#[cfg(target_os = "linux")]
fn required_helpers(action: &str) -> &'static [&'static str] {
    match action {
        "window_list" | "window_focus" | "window_close" => &["wmctrl"],
        "mouse_move" | "mouse_click" | "mouse_drag" | "key_type" | "key_press" => &["xdotool"],
        _ => &[],
    }
}

/// Provider-agnostic desktop automation tool.
pub struct ComputerUseTool {
    security: Arc<SecurityPolicy>,
    config: BrowserComputerUseConfig,
    topclaw_dir: PathBuf,
    session_name: Option<String>,
}

impl ComputerUseTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        config: BrowserComputerUseConfig,
        topclaw_dir: PathBuf,
        session_name: Option<String>,
    ) -> Self {
        Self {
            security,
            config,
            topclaw_dir,
            session_name,
        }
    }

    fn health_url(&self) -> String {
        sidecar_client::derive_health_url(&self.config.endpoint)
    }

    fn policy_envelope(&self) -> Value {
        json!({
            "window_allowlist": self.config.window_allowlist,
            "max_coordinate_x": self.config.max_coordinate_x,
            "max_coordinate_y": self.config.max_coordinate_y,
        })
    }

    fn metadata_envelope(&self) -> Value {
        json!({
            "source": "topclaw.computer_use",
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "session_name": self.session_name,
        })
    }

    fn http_client(&self) -> anyhow::Result<reqwest::Client> {
        Ok(reqwest::Client::builder()
            .timeout(Duration::from_millis(self.config.timeout_ms))
            .build()?)
    }

    /// Ensure the sidecar is healthy. If not and `auto_start` is on, spawn
    /// the built-in sidecar and wait for `/health`.
    async fn ensure_sidecar(&self) -> Result<(), String> {
        let health = self.health_url();
        if sidecar_client::probe_health(&health).await {
            return Ok(());
        }
        if !self.config.auto_start {
            return Err(format!(
                "computer-use sidecar not reachable at {health}. Either start it manually or set browser.computer_use.auto_start = true."
            ));
        }
        if !self.config.endpoint_is_local() {
            return Err(format!(
                "auto_start only spawns a local sidecar; endpoint {} is remote. Start the remote sidecar manually.",
                self.config.endpoint
            ));
        }
        let bind = sidecar_client::bind_addr_from_endpoint(&self.config.endpoint).ok_or_else(|| {
            format!(
                "cannot derive bind address from endpoint: {}",
                self.config.endpoint
            )
        })?;
        sidecar_client::spawn_sidecar_process(&bind, self.config.api_key.as_deref())?;

        let start = std::time::Instant::now();
        while start.elapsed() < sidecar_client::HEALTH_POLL_TIMEOUT {
            if sidecar_client::probe_health(&health).await {
                return Ok(());
            }
            tokio::time::sleep(sidecar_client::HEALTH_POLL_INTERVAL).await;
        }
        Err(format!(
            "spawned computer-use sidecar but /health at {health} did not become ready within {}s; ensure xdotool/wmctrl/scrot are installed (Linux) or the platform equivalents",
            sidecar_client::HEALTH_POLL_TIMEOUT.as_secs()
        ))
    }

    async fn post_action(&self, action: &str, params: Value) -> Result<Value, String> {
        let body = json!({
            "action": action,
            "params": params,
            "policy": self.policy_envelope(),
            "metadata": self.metadata_envelope(),
        });
        let client = self
            .http_client()
            .map_err(|e| format!("http client: {e}"))?;
        let mut req = client.post(&self.config.endpoint).json(&body);
        if let Some(key) = self.config.api_key.as_ref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("sidecar request failed: {e}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("sidecar read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("sidecar returned HTTP {status}: {text}"));
        }
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("sidecar invalid JSON: {e} -- body: {text}"))
    }

    fn capture_path(&self) -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        self.topclaw_dir
            .join("captures")
            .join(format!("capture_{ts}.png"))
    }

    fn enforce_app_allowlist(&self, app: &str) -> Result<(), String> {
        if self.config.app_allowlist.is_empty() {
            return Ok(());
        }
        let needle = Path::new(app)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(app)
            .to_ascii_lowercase();
        let hit = self.config.app_allowlist.iter().any(|allowed| {
            let a = allowed.to_ascii_lowercase();
            a == needle || a == app.to_ascii_lowercase()
        });
        if hit {
            Ok(())
        } else {
            Err(format!(
                "app '{app}' is not in browser.computer_use.app_allowlist ({:?}). Add it to config.toml to permit launching.",
                self.config.app_allowlist
            ))
        }
    }
}

#[async_trait]
impl Tool for ComputerUseTool {
    fn name(&self) -> &str {
        "computer_use"
    }

    fn description(&self) -> &str {
        "Desktop automation: launch applications, open URLs in a visible browser window, list/focus/close windows, take screenshots, click, drag, type, or press keys. \
         Standalone tool: use it directly for OS-level desktop tasks; it does NOT require `browser.enabled=true` or `browser.backend='computer_use'`. \
         IMPORTANT: When the user says 'open Chrome', 'open <app>', 'open this link in Chrome', or 'navigate to <URL> on the computer', use action=app_launch with the app name and args=[\"<URL>\"]. \
         Do NOT use web_fetch for these — web_fetch only downloads HTML text, it does NOT open a visible window or interact with the desktop. \
         Do NOT use browser_open for launching apps — browser_open only opens URLs and cannot launch arbitrary applications. \
         Example: to open https://example.com in Chrome, call computer_use with action=app_launch, app=\"google-chrome\", args=[\"https://example.com\"]. \
         If a call fails because Linux desktop helpers are missing, call it once with action=bootstrap to install them via the system package manager, then retry."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ACTIONS,
                    "description": "Desktop action to perform."
                },
                "app": { "type": "string", "description": "Application binary or name. For app_launch: executable like 'google-chrome' or 'code'. For window_focus/close/app_terminate: app name match." },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Command-line args for app_launch (e.g. a URL)." },
                "window_title": { "type": "string", "description": "Window title substring for window_focus/close." },
                "window_id": { "type": "string", "description": "Window id for window_focus/close." },
                "query": { "type": "string", "description": "Substring filter for window_list." },
                "pid": { "type": "integer", "description": "Process id for app_terminate." },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "from_x": { "type": "integer" },
                "from_y": { "type": "integer" },
                "to_x": { "type": "integer" },
                "to_y": { "type": "integer" },
                "text": { "type": "string", "description": "Text for key_type." },
                "key": { "type": "string", "description": "Key name for key_press (e.g. 'Enter', 'Escape', 'ctrl+l')." }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(fail("Action blocked: autonomy is read-only"));
        }
        if !self.security.record_action() {
            return Ok(fail("Action blocked: rate limit exceeded"));
        }

        let action = match args.get("action").and_then(Value::as_str) {
            Some(a) if ACTIONS.contains(&a) => a.to_string(),
            Some(other) => {
                return Ok(fail(&format!(
                    "unknown action '{other}'. Allowed: {}",
                    ACTIONS.join(", ")
                )))
            }
            None => return Ok(fail("missing 'action'")),
        };

        // Bootstrap runs entirely locally (no sidecar needed) so handle it up-front.
        if action == "bootstrap" {
            return Ok(bootstrap::run_bootstrap().await);
        }

        // Build action-specific params and any pre-flight validation.
        let (params, post): (Value, Option<PostProcess>) = match action.as_str() {
            "app_launch" => {
                let app = match args.get("app").and_then(Value::as_str) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => return Ok(fail("app_launch requires 'app'")),
                };
                if let Err(e) = self.enforce_app_allowlist(&app) {
                    return Ok(fail(&e));
                }
                let mut p = json!({ "app": app });
                if let Some(a) = args.get("args") {
                    p["args"] = a.clone();
                }
                (p, None)
            }
            "app_terminate" => {
                let mut p = json!({});
                if let Some(a) = args.get("app").and_then(Value::as_str) {
                    p["app"] = Value::String(a.to_string());
                }
                if let Some(pid) = args.get("pid").and_then(Value::as_i64) {
                    p["pid"] = Value::from(pid);
                }
                if p.as_object().map_or(true, |o| o.is_empty()) {
                    return Ok(fail("app_terminate requires 'app' or 'pid'"));
                }
                (p, None)
            }
            "window_list" => {
                let mut p = json!({});
                if let Some(q) = args.get("query").and_then(Value::as_str) {
                    p["query"] = Value::String(q.to_string());
                }
                (p, None)
            }
            "window_focus" | "window_close" => {
                let mut p = json!({});
                let mut have = false;
                for (k, jk) in [
                    ("window_id", "window_id"),
                    ("window_title", "window_title"),
                    ("app", "app"),
                ] {
                    if let Some(v) = args.get(k).and_then(Value::as_str) {
                        p[jk] = Value::String(v.to_string());
                        have = true;
                    }
                }
                if !have {
                    return Ok(fail(&format!(
                        "{action} requires one of 'window_id', 'window_title', or 'app'"
                    )));
                }
                (p, None)
            }
            "screen_capture" => {
                let path = self.capture_path();
                if let Some(parent) = path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return Ok(fail(&format!(
                            "cannot create capture dir {}: {e}",
                            parent.display()
                        )));
                    }
                }
                (
                    json!({ "path": path.to_string_lossy() }),
                    Some(PostProcess::Screenshot(path)),
                )
            }
            "mouse_move" | "mouse_click" => {
                let x = match args.get("x").and_then(Value::as_i64) {
                    Some(v) => v,
                    None => return Ok(fail(&format!("{action} requires 'x'"))),
                };
                let y = match args.get("y").and_then(Value::as_i64) {
                    Some(v) => v,
                    None => return Ok(fail(&format!("{action} requires 'y'"))),
                };
                (json!({ "x": x, "y": y }), None)
            }
            "mouse_drag" => {
                let required = ["from_x", "from_y", "to_x", "to_y"];
                let mut p = json!({});
                for k in required {
                    match args.get(k).and_then(Value::as_i64) {
                        Some(v) => {
                            p[k] = Value::from(v);
                        }
                        None => return Ok(fail(&format!("mouse_drag requires '{k}'"))),
                    }
                }
                (p, None)
            }
            "key_type" => {
                let t = match args.get("text").and_then(Value::as_str) {
                    Some(s) => s.to_string(),
                    None => return Ok(fail("key_type requires 'text'")),
                };
                (json!({ "text": t }), None)
            }
            "key_press" => {
                let k = match args.get("key").and_then(Value::as_str) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => return Ok(fail("key_press requires 'key'")),
                };
                (json!({ "key": k }), None)
            }
            _ => unreachable!(),
        };

        // Pre-flight: on Linux, check only the helpers the specific action needs.
        // This avoids blocking app_launch (no helpers needed) or screen_capture
        // (only needs scrot) just because wmctrl is missing.
        //
        // If helpers are missing, auto-bootstrap first (if auto_start is on),
        // then retry the pre-flight. This makes the first-use experience smooth:
        // the user doesn't need to manually call action=bootstrap.
        #[cfg(target_os = "linux")]
        {
            let needed = required_helpers(&action);
            let missing: Vec<&str> = needed
                .iter()
                .copied()
                .filter(|bin| which::which(bin).is_err())
                .collect();
            if !missing.is_empty() {
                // Auto-bootstrap: attempt to install missing helpers before
                // failing. This is independent of auto_start (which controls
                // sidecar spawning). The pre-flight already identified the
                // specific missing helpers, so the intent to use this action
                // is clear — installing the helpers is the natural next step.
                info!(
                    target: "topclaw::audit",
                    event = "computer_use_auto_bootstrap",
                    missing = %missing.join(", "),
                    "auto-bootstrapping missing desktop helpers"
                );
                let _ = bootstrap::run_bootstrap().await;
                // Re-check after bootstrap attempt.
                let still_missing: Vec<&str> = needed
                    .iter()
                    .copied()
                    .filter(|bin| which::which(bin).is_err())
                    .collect();
                if still_missing.is_empty() {
                    // Bootstrap succeeded — continue to the action.
                } else {
                    return Ok(fail(&format!(
                        "action '{}' requires {} but {} still not installed after auto-bootstrap. Install manually: sudo apt-get install xdotool wmctrl scrot",
                        action,
                        still_missing.join(", "),
                        if still_missing.len() == 1 { "it is" } else { "they are" }
                    )));
                }
            }
        }

        if let Err(e) = self.ensure_sidecar().await {
            return Ok(fail(&e));
        }

        let response = match self.post_action(&action, params).await {
            Ok(v) => v,
            Err(e) => return Ok(fail(&e)),
        };

        let success = response
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !success {
            let err = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("sidecar reported failure without message");
            return Ok(fail(err));
        }

        info!(
            target: "topclaw::audit",
            event = "computer_use_action",
            action = %action,
            "computer-use action succeeded"
        );

        let output = match post {
            Some(PostProcess::Screenshot(path)) => format_screenshot_output(&path, &response),
            None => format_generic_output(&action, &response),
        };

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

enum PostProcess {
    Screenshot(PathBuf),
}

fn fail(msg: &str) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(msg.to_string()),
    }
}

fn format_screenshot_output(path: &Path, response: &Value) -> String {
    let data = response.get("data").cloned().unwrap_or_else(|| json!({}));
    let dims = image::image_dimensions(path).ok();
    let size = dims.map(|(w, h)| format!(" ({w}x{h})")).unwrap_or_default();
    format!(
        "Screenshot saved: {}{size}. Sidecar data: {}",
        path.display(),
        compact_json(&data)
    )
}

fn format_generic_output(action: &str, response: &Value) -> String {
    let data = response.get("data").cloned().unwrap_or_else(|| json!({}));
    format!("{action} ok. {}", compact_json(&data))
}

fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".into())
}

/// Re-export bootstrap types for external callers (doctor, etc.).
pub use bootstrap::{DesktopHelperProbe, LINUX_HELPERS};

/// Detect which Linux desktop helpers are missing.
pub fn missing_linux_helpers() -> Vec<&'static str> {
    bootstrap::missing_helpers()
}

/// Probe desktop helper readiness.
pub fn probe_desktop_helpers() -> DesktopHelperProbe {
    bootstrap::probe_desktop_helpers()
}

/// Install missing Linux desktop helpers (daemon-safe).
pub async fn install_desktop_helpers() -> String {
    bootstrap::install_desktop_helpers().await
}

/// Install missing Linux desktop helpers (user-driven, may prompt).
pub async fn install_desktop_helpers_for_user_request() -> String {
    bootstrap::install_desktop_helpers_for_user_request().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy::default())
    }

    fn cfg_default() -> BrowserComputerUseConfig {
        let mut c = BrowserComputerUseConfig::default();
        c.enabled = true;
        c
    }

    fn tool(config: BrowserComputerUseConfig) -> ComputerUseTool {
        let tmp = tempfile::tempdir().unwrap();
        ComputerUseTool::new(security(), config, tmp.path().to_path_buf(), None)
    }

    #[test]
    fn schema_lists_all_actions() {
        let t = tool(cfg_default());
        let schema = t.parameters_schema();
        let enum_vals = schema["properties"]["action"]["enum"]
            .as_array()
            .expect("enum array");
        let names: Vec<&str> = enum_vals.iter().filter_map(Value::as_str).collect();
        for a in ACTIONS {
            assert!(names.contains(a), "missing action {a}");
        }
    }

    #[test]
    fn endpoint_is_local_detects_loopback() {
        let mut c = cfg_default();
        c.endpoint = "http://127.0.0.1:8787/v1/actions".into();
        assert!(c.endpoint_is_local());
        c.endpoint = "http://example.com:8787/v1/actions".into();
        assert!(!c.endpoint_is_local());
    }

    #[tokio::test]
    async fn rejects_unknown_action() {
        let t = tool(cfg_default());
        let r = t.execute(json!({"action": "fly_to_moon"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("unknown action"));
    }

    #[tokio::test]
    async fn rejects_missing_action() {
        let t = tool(cfg_default());
        let r = t.execute(json!({})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("missing 'action'"));
    }

    #[tokio::test]
    async fn rejects_app_launch_missing_app() {
        let t = tool(cfg_default());
        let r = t.execute(json!({"action": "app_launch"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("requires 'app'"));
    }

    #[tokio::test]
    async fn rejects_app_launch_not_in_allowlist() {
        let mut c = cfg_default();
        c.app_allowlist = vec!["google-chrome".into()];
        c.auto_start = false;
        // Use a remote endpoint so we never try to spawn a sidecar during the test.
        c.endpoint = "http://192.0.2.1:8787/v1/actions".into();
        let t = tool(c);
        let r = t
            .execute(json!({"action": "app_launch", "app": "rm"}))
            .await
            .unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("app_allowlist"));
    }

    #[tokio::test]
    async fn app_launch_basename_match_in_allowlist() {
        let mut c = cfg_default();
        c.app_allowlist = vec!["google-chrome".into()];
        c.auto_start = false;
        c.endpoint = "http://192.0.2.1:8787/v1/actions".into();
        let t = tool(c);
        // Allowlist should match by basename even when full path given.
        // The request will still fail at the HTTP layer (no sidecar), but the
        // allowlist gate must accept it first.
        let r = t
            .execute(json!({"action": "app_launch", "app": "/usr/bin/google-chrome"}))
            .await
            .unwrap();
        assert!(!r.success);
        let err = r.error.unwrap();
        assert!(
            !err.contains("app_allowlist"),
            "allowlist rejected a valid basename: {err}"
        );
    }

    #[tokio::test]
    async fn window_focus_requires_selector() {
        let t = tool(cfg_default());
        let r = t.execute(json!({"action": "window_focus"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("requires one of"));
    }

    #[tokio::test]
    async fn mouse_click_requires_coords() {
        let t = tool(cfg_default());
        let r = t
            .execute(json!({"action": "mouse_click", "x": 10}))
            .await
            .unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("requires 'y'"));
    }

    #[tokio::test]
    async fn key_type_requires_text() {
        let t = tool(cfg_default());
        let r = t.execute(json!({"action": "key_type"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("'text'"));
    }

    #[tokio::test]
    async fn key_press_requires_key() {
        let t = tool(cfg_default());
        let r = t.execute(json!({"action": "key_press"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().contains("'key'"));
    }

    #[test]
    fn schema_includes_bootstrap_action() {
        let t = tool(cfg_default());
        let schema = t.parameters_schema();
        let enum_vals = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert!(enum_vals.iter().any(|v| v.as_str() == Some("bootstrap")));
    }

    // Bootstrap module tests are in src/tools/bootstrap.rs

    #[test]
    fn desktop_helper_probe_is_structured() {
        let probe = probe_desktop_helpers();
        #[cfg(target_os = "linux")]
        {
            assert_eq!(probe.checked_helpers, LINUX_HELPERS.to_vec());
            assert!(probe
                .missing_helpers
                .iter()
                .all(|helper| probe.checked_helpers.contains(helper)));
            if probe.package_manager.is_some() && !probe.missing_helpers.is_empty() {
                assert!(!probe.packages_to_install.is_empty());
                assert!(probe.install_command.is_some());
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            assert!(probe.checked_helpers.is_empty());
            assert!(probe.missing_helpers.is_empty());
            assert!(probe.package_manager.is_none());
            assert!(probe.packages_to_install.is_empty());
            assert!(probe.install_command.is_none());
        }
    }

    // Bootstrap tests moved to src/tools/bootstrap.rs for proper separation

    #[cfg(target_os = "linux")]
    #[test]
    fn required_helpers_maps_actions_correctly() {
        // Actions that need no helpers
        assert!(required_helpers("app_launch").is_empty());
        assert!(required_helpers("app_terminate").is_empty());
        assert!(required_helpers("bootstrap").is_empty());
        // screen_capture skips pre-flight (sidecar has scrot/gnome-screenshot fallback)
        assert!(required_helpers("screen_capture").is_empty());
        // Actions that need specific helpers
        assert!(required_helpers("window_list").contains(&"wmctrl"));
        assert!(required_helpers("window_focus").contains(&"wmctrl"));
        assert!(required_helpers("window_close").contains(&"wmctrl"));
        assert!(required_helpers("mouse_move").contains(&"xdotool"));
        assert!(required_helpers("mouse_click").contains(&"xdotool"));
        assert!(required_helpers("mouse_drag").contains(&"xdotool"));
        assert!(required_helpers("key_type").contains(&"xdotool"));
        assert!(required_helpers("key_press").contains(&"xdotool"));
    }

    // Re-exports from bootstrap module - tests are in bootstrap.rs

    #[tokio::test]
    async fn rejects_when_autonomy_readonly() {
        use crate::security::AutonomyLevel;
        let sec = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tmp = tempfile::tempdir().unwrap();
        let t = ComputerUseTool::new(sec, cfg_default(), tmp.path().to_path_buf(), None);
        let r = t.execute(json!({"action": "window_list"})).await.unwrap();
        assert!(!r.success);
        assert!(r.error.unwrap().to_lowercase().contains("read-only"));
    }

    // ── computer_use failure path tests (PART 5: protect mainline) ─────────────

    #[tokio::test]
    async fn sidecar_unreachable_without_auto_start_returns_clear_error() {
        let mut c = cfg_default();
        c.auto_start = false;
        // Use a non-routable IP so the request fails immediately
        c.endpoint = "http://192.0.2.1:8787/v1/actions".into();
        let t = tool(c);
        let r = t.execute(json!({"action": "window_list"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.unwrap();
        assert!(err.contains("not reachable"), "error should mention sidecar not reachable: {err}");
        assert!(err.contains("auto_start"), "error should mention auto_start option: {err}");
    }

    #[tokio::test]
    async fn remote_endpoint_blocks_auto_start_with_clear_message() {
        let mut c = cfg_default();
        c.auto_start = true;
        c.endpoint = "http://remote.example.com:8787/v1/actions".into();
        let t = tool(c);
        let r = t.execute(json!({"action": "window_list"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.unwrap();
        assert!(err.contains("auto_start only spawns a local sidecar"),
            "error should explain auto_start limitation: {err}");
        assert!(err.contains("Start the remote sidecar manually"),
            "error should suggest manual start: {err}");
    }

    #[tokio::test]
    async fn invalid_endpoint_gives_parseable_error() {
        let mut c = cfg_default();
        c.auto_start = true;
        // Empty endpoint is treated as non-local, triggering auto_start block message
        // This is a descriptive error, not a panic
        c.endpoint = "".into();
        let t = tool(c);
        let r = t.execute(json!({"action": "window_list"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.as_ref().expect("error must be present");
        // Error should explain that auto_start can't handle this endpoint
        assert!(err.contains("auto_start") || err.contains("local"),
            "error should be descriptive: {err}");
    }

    #[tokio::test]
    async fn app_terminate_requires_app_or_pid() {
        let mut c = cfg_default();
        c.auto_start = false;
        c.endpoint = "http://192.0.2.1:8787/v1/actions".into();
        let t = tool(c);
        // app_terminate without app or pid should be rejected
        let r = t.execute(json!({"action": "app_terminate"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.as_ref().expect("error must be present");
        assert!(err.contains("app") || err.contains("pid"), "error should mention missing app/pid: {err}");
    }

    #[tokio::test]
    async fn window_focus_blocked_when_sidecar_unreachable() {
        let mut c = cfg_default();
        c.auto_start = false;
        c.endpoint = "http://192.0.2.1:8787/v1/actions".into();
        let t = tool(c);
        // window_focus without any selector (window_id, window_title, app) should fail
        let r = t.execute(json!({"action": "window_focus"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.as_ref().expect("error must be present");
        assert!(err.contains("requires"), "error should mention missing selector: {err}");
    }

    #[tokio::test]
    async fn mouse_drag_requires_all_coords() {
        let t = tool(cfg_default());
        let r = t.execute(json!({
            "action": "mouse_drag",
            "from_x": 0,
            "from_y": 0
            // missing to_x, to_y
        })).await.unwrap();
        assert!(!r.success);
        let err = r.error.as_ref().expect("error must be present");
        assert!(err.contains("to_x") || err.contains("to_y"), "error should mention missing coords: {err}");
    }

    #[tokio::test]
    async fn security_policy_rate_limit_blocks_execution() {
        use crate::security::AutonomyLevel;
        // Rate limit of 0 means no actions allowed
        let sec = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        });
        let tmp = tempfile::tempdir().unwrap();
        let t = ComputerUseTool::new(sec, cfg_default(), tmp.path().to_path_buf(), None);
        // Even bootstrap (which doesn't need sidecar) should be blocked by rate limit
        let r = t.execute(json!({"action": "bootstrap"})).await.unwrap();
        assert!(!r.success);
        let err = r.error.as_ref().expect("error must be present");
        assert!(err.contains("rate limit") || err.contains("exceeded"), "error should mention rate limit: {err}");
    }
}
