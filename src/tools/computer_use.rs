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

use super::traits::{Tool, ToolResult};
use crate::config::BrowserComputerUseConfig;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

const HEALTH_POLL_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// Linux desktop helpers the sidecar shells out to.
#[cfg(target_os = "linux")]
const LINUX_HELPERS: &[&str] = &["xdotool", "wmctrl", "scrot", "xdg-open"];

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
        derive_health_url(&self.config.endpoint)
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
        if probe_health(&health).await {
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
        let bind = bind_addr_from_endpoint(&self.config.endpoint).ok_or_else(|| {
            format!(
                "cannot derive bind address from endpoint: {}",
                self.config.endpoint
            )
        })?;
        spawn_sidecar(&bind, self.config.api_key.as_deref())?;

        let start = std::time::Instant::now();
        while start.elapsed() < HEALTH_POLL_TIMEOUT {
            if probe_health(&health).await {
                return Ok(());
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
        Err(format!(
            "spawned computer-use sidecar but /health at {health} did not become ready within {}s; ensure xdotool/wmctrl/scrot are installed (Linux) or the platform equivalents",
            HEALTH_POLL_TIMEOUT.as_secs()
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
            return Ok(run_bootstrap().await);
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

        // Pre-flight: on Linux, the sidecar shells out to xdotool/wmctrl/scrot/
        // xdg-open. If any are missing, surface a clear error pointing the
        // model at the bootstrap action instead of letting the sidecar emit a
        // cryptic "command not found".
        #[cfg(target_os = "linux")]
        {
            let missing = missing_linux_helpers();
            if !missing.is_empty() {
                return Ok(fail(&format!(
                    "missing desktop helpers on Linux: {}. Call computer_use with action=bootstrap to install them automatically (uses the system package manager via sudo -n), then retry.",
                    missing.join(", ")
                )));
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

async fn probe_health(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    else {
        return false;
    };
    matches!(
        client.get(url).send().await,
        Ok(r) if r.status().is_success()
    )
}

fn spawn_sidecar(bind: &str, api_key: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot resolve current executable to spawn sidecar: {e}"))?;
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("computer-use-sidecar").arg("--bind").arg(bind);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        cmd.env("TOPCLAW_SIDECAR_API_KEY", key);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn sidecar: {e}"))?;
    let pid = child.id();
    debug!(?pid, %bind, "spawned computer-use sidecar");
    // Detach a reaper so the Child handle can be dropped without
    // leaving a zombie. The sidecar continues running until killed.
    tokio::spawn(async move {
        if let Err(e) = child.wait().await {
            warn!(error=%e, "computer-use sidecar wait failed");
        }
    });
    Ok(())
}

/// Detect missing Linux desktop helpers by probing `PATH` with `which`.
#[cfg(target_os = "linux")]
fn missing_linux_helpers() -> Vec<&'static str> {
    LINUX_HELPERS
        .iter()
        .copied()
        .filter(|bin| which::which(bin).is_err())
        .collect()
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn missing_linux_helpers() -> Vec<&'static str> {
    Vec::new()
}

/// Install missing Linux desktop helpers using the detected package manager.
/// Non-Linux platforms report "no-op".
#[cfg(target_os = "linux")]
async fn run_bootstrap() -> ToolResult {
    let missing = missing_linux_helpers();
    if missing.is_empty() {
        return ToolResult {
            success: true,
            output: "All Linux desktop helpers (xdotool, wmctrl, scrot, xdg-open) are already installed.".into(),
            error: None,
        };
    }
    let (manager, _packages) = match detect_package_manager() {
        Some(pair) => pair,
        None => {
            return fail(&format!(
                "missing helpers ({}) but no supported package manager (apt-get, dnf, pacman, zypper, apk) was found. Install manually: xdotool wmctrl scrot xdg-utils",
                missing.join(", ")
            ));
        }
    };

    // Map helper binary names to distro packages.
    let pkgs = packages_for_missing(&missing, manager);

    // Sanity-check sudo is available and non-interactive.
    if which::which("sudo").is_err() {
        return fail("'sudo' not found. Install manually as root: xdotool wmctrl scrot xdg-utils");
    }
    if !sudo_noninteractive_ok().await {
        return fail(&format!(
            "sudo requires a password (sudo -n failed). Either configure passwordless sudo for desktop package installs, or run manually: sudo {manager_install}",
            manager_install = install_command_string(manager, &pkgs)
        ));
    }

    // Run the install. Capture combined output so the model can show it.
    let mut argv = vec!["-n".to_string()];
    argv.extend(install_argv(manager, &pkgs));
    let out = tokio::process::Command::new("sudo")
        .args(&argv)
        .stdin(Stdio::null())
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            let still_missing = missing_linux_helpers();
            if still_missing.is_empty() {
                info!(
                    target: "topclaw::audit",
                    event = "computer_use_bootstrap",
                    manager = manager,
                    packages = ?pkgs,
                    "installed Linux desktop helpers"
                );
                ToolResult {
                    success: true,
                    output: format!(
                        "Installed {} via {manager}. Desktop helpers ready.",
                        pkgs.join(" ")
                    ),
                    error: None,
                }
            } else {
                fail(&format!(
                    "{manager} reported success but {} still missing. Try reinstalling manually.",
                    still_missing.join(", ")
                ))
            }
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            fail(&format!(
                "{manager} install failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            ))
        }
        Err(e) => fail(&format!("failed to run sudo {manager}: {e}")),
    }
}

#[cfg(not(target_os = "linux"))]
async fn run_bootstrap() -> ToolResult {
    ToolResult {
        success: true,
        output: format!(
            "bootstrap is a no-op on {}: desktop helpers are not required.",
            std::env::consts::OS
        ),
        error: None,
    }
}

#[cfg(target_os = "linux")]
fn detect_package_manager() -> Option<(&'static str, Vec<&'static str>)> {
    // Return (manager-binary, base-package-list-for-full-install).
    // The packages list is a superset; we filter to only what's actually missing.
    let candidates = [
        ("apt-get", vec!["xdotool", "wmctrl", "scrot", "xdg-utils"]),
        ("dnf", vec!["xdotool", "wmctrl", "scrot", "xdg-utils"]),
        ("pacman", vec!["xdotool", "wmctrl", "scrot", "xdg-utils"]),
        ("zypper", vec!["xdotool", "wmctrl", "scrot", "xdg-utils"]),
        ("apk", vec!["xdotool", "wmctrl", "scrot", "xdg-utils"]),
    ];
    for (bin, pkgs) in candidates {
        if which::which(bin).is_ok() {
            return Some((bin, pkgs));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn packages_for_missing(missing: &[&str], _manager: &str) -> Vec<&'static str> {
    // Map helper binary names to distro package names. xdg-open ships in
    // xdg-utils across the major distros.
    let mut out: Vec<&'static str> = Vec::new();
    for m in missing {
        let pkg = match *m {
            "xdotool" => "xdotool",
            "wmctrl" => "wmctrl",
            "scrot" => "scrot",
            "xdg-open" => "xdg-utils",
            _ => continue,
        };
        if !out.contains(&pkg) {
            out.push(pkg);
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn install_argv(manager: &str, pkgs: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = match manager {
        "apt-get" => vec!["apt-get".into(), "install".into(), "-y".into()],
        "dnf" => vec!["dnf".into(), "install".into(), "-y".into()],
        "pacman" => vec!["pacman".into(), "-S".into(), "--noconfirm".into()],
        "zypper" => vec!["zypper".into(), "install".into(), "-y".into()],
        "apk" => vec!["apk".into(), "add".into()],
        _ => vec![manager.into(), "install".into()],
    };
    for p in pkgs {
        argv.push((*p).into());
    }
    argv
}

#[cfg(target_os = "linux")]
fn install_command_string(manager: &str, pkgs: &[&str]) -> String {
    install_argv(manager, pkgs).join(" ")
}

#[cfg(target_os = "linux")]
async fn sudo_noninteractive_ok() -> bool {
    // `sudo -n true` exits 0 only if a valid cached credential or NOPASSWD
    // entry makes sudo non-interactive.
    match tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

fn derive_health_url(endpoint: &str) -> String {
    match reqwest::Url::parse(endpoint) {
        Ok(u) => {
            let mut base = format!("{}://{}", u.scheme(), u.host_str().unwrap_or("127.0.0.1"));
            if let Some(port) = u.port() {
                base.push(':');
                base.push_str(&port.to_string());
            }
            base.push_str("/health");
            base
        }
        Err(_) => "http://127.0.0.1:8787/health".into(),
    }
}

fn bind_addr_from_endpoint(endpoint: &str) -> Option<String> {
    let u = reqwest::Url::parse(endpoint).ok()?;
    let host = u.host_str()?;
    let port = u.port_or_known_default()?;
    let addr = format!("{host}:{port}");
    addr.parse::<SocketAddr>().ok().map(|s| s.to_string())
}

impl BrowserComputerUseConfig {
    fn endpoint_is_local(&self) -> bool {
        match reqwest::Url::parse(&self.endpoint) {
            Ok(u) => matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
            Err(_) => false,
        }
    }
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
    fn derive_health_url_works() {
        assert_eq!(
            derive_health_url("http://127.0.0.1:8787/v1/actions"),
            "http://127.0.0.1:8787/health"
        );
        assert_eq!(
            derive_health_url("http://localhost:9000/v1/actions"),
            "http://localhost:9000/health"
        );
    }

    #[test]
    fn bind_addr_from_endpoint_works() {
        assert_eq!(
            bind_addr_from_endpoint("http://127.0.0.1:8787/v1/actions").as_deref(),
            Some("127.0.0.1:8787")
        );
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

    #[cfg(target_os = "linux")]
    #[test]
    fn install_argv_shapes_by_manager() {
        let s = |v: Vec<&str>| -> Vec<String> { v.into_iter().map(String::from).collect() };
        assert_eq!(
            install_argv("apt-get", &["xdotool"]),
            s(vec!["apt-get", "install", "-y", "xdotool"])
        );
        assert_eq!(
            install_argv("pacman", &["xdotool", "wmctrl"]),
            s(vec!["pacman", "-S", "--noconfirm", "xdotool", "wmctrl"])
        );
        assert_eq!(
            install_argv("dnf", &["scrot"]),
            s(vec!["dnf", "install", "-y", "scrot"])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn packages_for_missing_maps_xdg_open_to_xdg_utils() {
        let pkgs = packages_for_missing(&["xdg-open", "xdotool"], "apt-get");
        assert_eq!(pkgs, vec!["xdg-utils", "xdotool"]);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn bootstrap_reports_already_ready_when_nothing_missing() {
        // This test is meaningful only when the host already has the helpers.
        if !missing_linux_helpers().is_empty() {
            return;
        }
        let r = run_bootstrap().await;
        assert!(r.success);
        assert!(r.output.contains("already installed"));
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn bootstrap_is_noop_on_non_linux() {
        let r = run_bootstrap().await;
        assert!(r.success);
        assert!(r.output.contains("no-op"));
    }

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
}
