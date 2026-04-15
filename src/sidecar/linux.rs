//! Linux action dispatcher for the computer-use sidecar.
//!
//! Shells out to `xdotool`, `wmctrl`, `scrot` (or `gnome-screenshot`),
//! `xdg-open`, and `kill`/`pkill`. Every handler enforces the policy envelope
//! before invoking a system binary.

use super::server::{ActionRequest, Policy};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::ffi::OsStr;
use tokio::process::Command;

/// Dispatch a validated action request. Returns the `data` payload on success.
pub async fn dispatch(req: &ActionRequest) -> Result<Value> {
    if !cfg!(target_os = "linux") {
        bail!("computer-use sidecar only supports Linux at runtime");
    }
    match req.action.as_str() {
        "window_list" => window_list(&req.params, &req.policy).await,
        "window_focus" => window_focus(&req.params, &req.policy).await,
        "window_close" => window_close(&req.params, &req.policy).await,
        "app_launch" => app_launch(&req.params).await,
        "app_terminate" => app_terminate(&req.params).await,
        "mouse_move" => mouse_move(&req.params, &req.policy).await,
        "mouse_click" => mouse_click(&req.params, &req.policy).await,
        "mouse_drag" => mouse_drag(&req.params, &req.policy).await,
        "key_type" => key_type(&req.params).await,
        "key_press" => key_press(&req.params).await,
        "screen_capture" => screen_capture(&req.params).await,
        "open" => open_url(&req.params, &req.policy).await,
        other => Err(anyhow!("unsupported action: {other}")),
    }
}

// ── policy helpers ─────────────────────────────────────────────────────────

fn require_str<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing or non-string param '{key}'"))
}

fn require_i64(params: &Value, key: &str) -> Result<i64> {
    params
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("missing or non-integer param '{key}'"))
}

fn clamp_coord(v: i64, max: Option<i64>, axis: &str) -> Result<i64> {
    if v < 0 {
        bail!("{axis}={v} is negative");
    }
    if let Some(m) = max {
        if v > m {
            bail!("{axis}={v} exceeds policy max ({m})");
        }
    }
    Ok(v)
}

fn window_title_permitted(title: &str, policy: &Policy) -> bool {
    if policy.window_allowlist.is_empty() {
        return true;
    }
    policy
        .window_allowlist
        .iter()
        .any(|allowed| title.contains(allowed))
}

/// Reject key sequences whose names are not in the safe character set.
/// xdotool accepts e.g. `Return`, `ctrl+c`, `shift+Tab`. We permit only
/// alphanumerics, underscore, and `+` — enough for documented key syntax,
/// not enough to sneak in a shell metacharacter.
fn key_name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+')
}

// ── command helpers ────────────────────────────────────────────────────────

async fn run_capture<S, I>(program: &str, args: I) -> Result<String>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to spawn '{program}'"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "'{program}' exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn run_status<S, I>(program: &str, args: I) -> Result<()>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let status = Command::new(program)
        .args(args)
        .status()
        .await
        .with_context(|| format!("failed to spawn '{program}'"))?;
    if !status.success() {
        bail!("'{program}' exited with {status}");
    }
    Ok(())
}

// ── action handlers ────────────────────────────────────────────────────────

async fn window_list(params: &Value, policy: &Policy) -> Result<Value> {
    let query = params
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let raw = run_capture("wmctrl", ["-l"]).await?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(4, char::is_whitespace).collect();
        if parts.len() < 4 {
            continue;
        }
        let id = parts[0];
        let title = parts[3].trim();
        if !query.is_empty() && !title.to_ascii_lowercase().contains(&query) {
            continue;
        }
        if !window_title_permitted(title, policy) {
            continue;
        }
        out.push(json!({ "id": id, "title": title }));
    }
    Ok(json!({ "action": "window_list", "windows": out }))
}

async fn window_focus(params: &Value, policy: &Policy) -> Result<Value> {
    if let Some(id) = params.get("window_id").and_then(Value::as_str) {
        if !id.starts_with("0x") || !id[2..].chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("window_id must be a hex string like 0x04a00005");
        }
        run_status("wmctrl", ["-i", "-a", id]).await?;
        return Ok(json!({ "action": "window_focus", "window_id": id }));
    }
    let title = require_str(params, "window_title")?;
    if !window_title_permitted(title, policy) {
        bail!("window title '{title}' is not in window_allowlist");
    }
    run_status("wmctrl", ["-a", title]).await?;
    Ok(json!({ "action": "window_focus", "window_title": title }))
}

async fn window_close(params: &Value, policy: &Policy) -> Result<Value> {
    let title = require_str(params, "window_title")?;
    if !window_title_permitted(title, policy) {
        bail!("window title '{title}' is not in window_allowlist");
    }
    run_status("wmctrl", ["-c", title]).await?;
    Ok(json!({ "action": "window_close", "window_title": title }))
}

#[allow(clippy::unused_async)] // dispatch() expects async; spawn alone has no await
async fn app_launch(params: &Value) -> Result<Value> {
    let app = require_str(params, "app")?;
    let args: Vec<&str> = params
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let child = Command::new(app)
        .args(&args)
        .spawn()
        .with_context(|| format!("failed to spawn '{app}'"))?;
    let pid = child.id();
    Ok(json!({ "action": "app_launch", "app": app, "pid": pid }))
}

async fn app_terminate(params: &Value) -> Result<Value> {
    if let Some(pid) = params.get("pid").and_then(Value::as_i64) {
        if pid <= 1 {
            bail!("refusing to terminate pid {pid}");
        }
        run_status("kill", ["--", &pid.to_string()]).await?;
        return Ok(json!({ "action": "app_terminate", "pid": pid }));
    }
    let app = require_str(params, "app")?;
    // Strict whitelist of characters — no shell metacharacters reach pkill.
    if !app
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("app name contains unsafe characters");
    }
    run_status("pkill", ["-x", app]).await?;
    Ok(json!({ "action": "app_terminate", "app": app }))
}

async fn mouse_move(params: &Value, policy: &Policy) -> Result<Value> {
    let x = clamp_coord(require_i64(params, "x")?, policy.max_coordinate_x, "x")?;
    let y = clamp_coord(require_i64(params, "y")?, policy.max_coordinate_y, "y")?;
    run_status("xdotool", ["mousemove", &x.to_string(), &y.to_string()]).await?;
    Ok(json!({ "action": "mouse_move", "x": x, "y": y }))
}

async fn mouse_click(params: &Value, policy: &Policy) -> Result<Value> {
    let x = clamp_coord(require_i64(params, "x")?, policy.max_coordinate_x, "x")?;
    let y = clamp_coord(require_i64(params, "y")?, policy.max_coordinate_y, "y")?;
    run_status(
        "xdotool",
        ["mousemove", &x.to_string(), &y.to_string(), "click", "1"],
    )
    .await?;
    Ok(json!({ "action": "mouse_click", "x": x, "y": y }))
}

async fn mouse_drag(params: &Value, policy: &Policy) -> Result<Value> {
    let fx = clamp_coord(
        require_i64(params, "from_x")?,
        policy.max_coordinate_x,
        "from_x",
    )?;
    let fy = clamp_coord(
        require_i64(params, "from_y")?,
        policy.max_coordinate_y,
        "from_y",
    )?;
    let tx = clamp_coord(
        require_i64(params, "to_x")?,
        policy.max_coordinate_x,
        "to_x",
    )?;
    let ty = clamp_coord(
        require_i64(params, "to_y")?,
        policy.max_coordinate_y,
        "to_y",
    )?;
    run_status(
        "xdotool",
        [
            "mousemove",
            &fx.to_string(),
            &fy.to_string(),
            "mousedown",
            "1",
            "mousemove",
            &tx.to_string(),
            &ty.to_string(),
            "mouseup",
            "1",
        ],
    )
    .await?;
    Ok(json!({
        "action": "mouse_drag",
        "from": {"x": fx, "y": fy},
        "to": {"x": tx, "y": ty}
    }))
}

async fn key_type(params: &Value) -> Result<Value> {
    let text = require_str(params, "text")?;
    if text.chars().any(|c| c == '\0') {
        bail!("text contains NUL byte");
    }
    // `--` stops flag parsing so user text starting with '-' is safe.
    run_status("xdotool", ["type", "--delay", "20", "--", text]).await?;
    Ok(json!({ "action": "key_type", "length": text.chars().count() }))
}

async fn key_press(params: &Value) -> Result<Value> {
    let key = require_str(params, "key")?;
    if !key_name_ok(key) {
        bail!("key name '{key}' contains unsafe characters");
    }
    run_status("xdotool", ["key", "--", key]).await?;
    Ok(json!({ "action": "key_press", "key": key }))
}

async fn screen_capture(params: &Value) -> Result<Value> {
    let path = require_str(params, "path")?;
    if path.chars().any(|c| c == '\0') {
        bail!("path contains NUL byte");
    }
    // Try scrot first; fall back to gnome-screenshot.
    let scrot = Command::new("scrot")
        .args(["-o", "--", path])
        .status()
        .await;
    match scrot {
        Ok(s) if s.success() => return Ok(json!({ "action": "screen_capture", "path": path })),
        _ => {}
    }
    run_status("gnome-screenshot", ["-f", path]).await?;
    Ok(json!({ "action": "screen_capture", "path": path }))
}

async fn open_url(params: &Value, policy: &Policy) -> Result<Value> {
    let url = require_str(params, "url")?;
    let parsed = reqwest::Url::parse(url).with_context(|| format!("invalid url: {url}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("url has no host: {url}"))?
        .to_ascii_lowercase();
    if !policy.allowed_domains.is_empty() {
        let allowed = policy.allowed_domains.iter().any(|d| {
            let d = d.to_ascii_lowercase();
            host == d || host.ends_with(&format!(".{d}"))
        });
        if !allowed {
            bail!("host '{host}' not in allowed_domains");
        }
    }
    run_status("xdg-open", ["--", url]).await?;
    Ok(json!({ "action": "open", "url": url }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with(allowlist: &[&str]) -> Policy {
        Policy {
            window_allowlist: allowlist.iter().map(|s| (*s).to_string()).collect(),
            ..Policy::default()
        }
    }

    #[test]
    fn window_title_permitted_empty_allowlist_is_open() {
        assert!(window_title_permitted("Any Title", &Policy::default()));
    }

    #[test]
    fn window_title_permitted_substring_match() {
        let p = policy_with(&["Chrome"]);
        assert!(window_title_permitted("Google Chrome", &p));
        assert!(!window_title_permitted("Firefox", &p));
    }

    #[test]
    fn clamp_coord_rejects_negative() {
        assert!(clamp_coord(-1, None, "x").is_err());
    }

    #[test]
    fn clamp_coord_rejects_over_max() {
        assert!(clamp_coord(3841, Some(3840), "x").is_err());
    }

    #[test]
    fn clamp_coord_accepts_in_bounds() {
        assert_eq!(clamp_coord(100, Some(3840), "x").unwrap(), 100);
    }

    #[test]
    fn key_name_ok_accepts_safe_chords() {
        assert!(key_name_ok("Return"));
        assert!(key_name_ok("ctrl+c"));
        assert!(key_name_ok("shift+Tab"));
        assert!(key_name_ok("F5"));
    }

    #[test]
    fn key_name_ok_rejects_unsafe() {
        assert!(!key_name_ok(""));
        assert!(!key_name_ok("; rm -rf /"));
        assert!(!key_name_ok("Return$(whoami)"));
        assert!(!key_name_ok("a b"));
    }
}
