//! Shared sidecar client utilities for the `computer_use`,
//! `computer_use_sidecar_start`, and `browser` tools.
//!
//! This module eliminates the previously duplicated health-probe, sidecar-spawn,
//! URL-derivation, and action-POST logic that was copied across tool files.

use serde_json::{json, Value};
use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;
use tracing::{debug, warn};

/// Default bind address for the computer-use sidecar.
pub const DEFAULT_BIND: &str = "127.0.0.1:8787";

/// Maximum time to wait for the sidecar `/health` endpoint after spawn.
pub const HEALTH_POLL_TIMEOUT: Duration = Duration::from_secs(15);

/// Interval between `/health` probes while waiting for the sidecar.
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Probe the sidecar's `/health` endpoint. Returns `true` if it responds with
/// a success status within a short timeout.
pub async fn probe_health(url: &str) -> bool {
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

/// Spawn the built-in sidecar as a detached child process, returning only
/// the PID. The child is reaped by an async task so the caller never needs
/// to worry about zombies.
///
/// This is the convenience wrapper for callers that only need "fire and
/// forget" semantics (e.g. `computer_use` tool auto-start).
pub fn spawn_sidecar_process(bind: &str, api_key: Option<&str>) -> Result<u32, String> {
    let mut child = spawn_sidecar_child(bind, api_key)?;
    let pid = child.id().expect("child process should have a PID");
    debug!(?pid, %bind, "spawned computer-use sidecar");
    // Detach a reaper so the Child handle can be dropped without
    // leaving a zombie. The sidecar continues running until killed.
    tokio::spawn(async move {
        if let Err(e) = child.wait().await {
            warn!(error=%e, "computer-use sidecar wait failed");
        }
    });
    Ok(pid)
}

/// Spawn the built-in sidecar as a child process and return the `Child`
/// handle so the caller can control its lifecycle (kill on failure,
/// detach on success, etc.).
///
/// This is the low-level primitive used by `computer_use_sidecar_start`
/// which needs to kill the child if `/health` never becomes ready.
pub fn spawn_sidecar_child(
    bind: &str,
    api_key: Option<&str>,
) -> Result<tokio::process::Child, String> {
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

    cmd.spawn()
        .map_err(|e| format!("failed to spawn sidecar: {e}"))
}

/// Derive the `/health` URL from a sidecar action endpoint.
///
/// For example, `http://127.0.0.1:8787/v1/actions` becomes
/// `http://127.0.0.1:8787/health`.
pub fn derive_health_url(endpoint: &str) -> String {
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

/// Poll the sidecar's `/health` endpoint until it responds or the timeout
/// elapses. Returns `true` if the sidecar became healthy within the window.
///
/// This consolidates the previously duplicated health-poll loops in
/// `computer_use.rs` and `computer_use_sidecar_start.rs`.
pub async fn wait_for_healthy(health_url: &str) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < HEALTH_POLL_TIMEOUT {
        if probe_health(health_url).await {
            return true;
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    false
}

/// Extract a `host:port` bind address from a sidecar endpoint URL.
///
/// Returns `None` if the URL cannot be parsed or the resulting address
/// is not a valid `SocketAddr`.
pub fn bind_addr_from_endpoint(endpoint: &str) -> Option<String> {
    let u = reqwest::Url::parse(endpoint).ok()?;
    let host = u.host_str()?;
    let port = u.port_or_known_default()?;
    let addr = format!("{host}:{port}");
    addr.parse::<SocketAddr>().ok().map(|s| s.to_string())
}

/// POST an action payload to the sidecar endpoint and return the parsed
/// JSON response body. Uses the runtime proxy-aware client so proxy
/// environment variables are respected.
///
/// This consolidates the duplicate HTTP POST logic that was previously in
/// both `computer_use.rs` and `browser.rs`.
/// Validate a computer-use action's params before sending to the sidecar.
///
/// This consolidates the duplicate validation logic that was previously in
/// both `computer_use.rs` and `browser.rs`. The browser.rs limits (256-char
/// app names, 32-char key_press, 4096-char text, coordinate bounds) are
/// used as canonical since they protect against abuse.
pub fn validate_computer_use_action(
    action: &str,
    params: &serde_json::Map<String, Value>,
    max_coordinate_x: Option<i64>,
    max_coordinate_y: Option<i64>,
) -> Result<(), String> {
    match action {
        "open" => {
            params
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'url' for open action".to_string())?;
        }
        "mouse_move" | "mouse_click" => {
            let x = read_required_i64(params, "x")?;
            let y = read_required_i64(params, "y")?;
            validate_coordinate("x", x, max_coordinate_x)?;
            validate_coordinate("y", y, max_coordinate_y)?;
        }
        "mouse_drag" => {
            let from_x = read_required_i64(params, "from_x")?;
            let from_y = read_required_i64(params, "from_y")?;
            let to_x = read_required_i64(params, "to_x")?;
            let to_y = read_required_i64(params, "to_y")?;
            validate_coordinate("from_x", from_x, max_coordinate_x)?;
            validate_coordinate("to_x", to_x, max_coordinate_x)?;
            validate_coordinate("from_y", from_y, max_coordinate_y)?;
            validate_coordinate("to_y", to_y, max_coordinate_y)?;
        }
        "key_type" => {
            let text = params
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'text' for key_type action".to_string())?;
            if text.trim().is_empty() {
                return Err("'text' for key_type must not be empty".to_string());
            }
            if text.len() > 4096 {
                return Err("'text' for key_type exceeds maximum length (4096 chars)".to_string());
            }
        }
        "key_press" => {
            let key = params
                .get("key")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'key' for key_press action".to_string())?;
            let valid = !key.trim().is_empty()
                && key.len() <= 32
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+'));
            if !valid {
                return Err("'key' for key_press must be 1-32 chars of [A-Za-z0-9_+]".to_string());
            }
        }
        "window_list" => {
            if let Some(query) = params.get("query").and_then(Value::as_str) {
                if query.len() > 256 {
                    return Err(
                        "'query' for window_list exceeds maximum length (256 chars)".to_string()
                    );
                }
            }
        }
        "window_focus" | "window_close" => {
            let has_window_id = params
                .get("window_id")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.trim().is_empty() && v.len() <= 128);
            let has_window_title = params
                .get("window_title")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.trim().is_empty() && v.len() <= 256);
            let has_app = params
                .get("app")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.trim().is_empty() && v.len() <= 256);
            if !(has_window_id || has_window_title || has_app) {
                return Err(format!(
                    "'{action}' requires one of: window_id, window_title, or app"
                ));
            }
        }
        "app_launch" => {
            let app = params
                .get("app")
                .and_then(Value::as_str)
                .ok_or_else(|| "Missing 'app' for app_launch action".to_string())?;
            if app.trim().is_empty() || app.len() > 256 {
                return Err("'app' for app_launch must be 1-256 chars".to_string());
            }
            if let Some(arguments) = params.get("args") {
                let args = arguments
                    .as_array()
                    .ok_or_else(|| "'args' for app_launch must be an array".to_string())?;
                if args.len() > 32 {
                    return Err(
                        "'args' for app_launch exceeds maximum length (32 items)".to_string()
                    );
                }
                for arg in args {
                    let value = arg.as_str().ok_or_else(|| {
                        "all 'args' entries for app_launch must be strings".to_string()
                    })?;
                    if value.len() > 256 {
                        return Err(
                            "an app_launch argument exceeds maximum length (256 chars)".to_string()
                        );
                    }
                }
            }
        }
        "app_terminate" => {
            let has_app = params
                .get("app")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.trim().is_empty() && v.len() <= 256);
            let has_pid = params.get("pid").and_then(Value::as_i64).is_some();
            if !(has_app || has_pid) {
                return Err("'app_terminate' requires either 'app' or 'pid'".to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

fn read_required_i64(params: &serde_json::Map<String, Value>, key: &str) -> Result<i64, String> {
    params
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("Missing or invalid '{key}' parameter"))
}

fn validate_coordinate(key: &str, value: i64, max: Option<i64>) -> Result<(), String> {
    if value < 0 {
        return Err(format!("'{key}' must be >= 0"));
    }
    if let Some(limit) = max {
        if limit < 0 {
            return Err(format!(
                "Configured coordinate limit for '{key}' must be >= 0"
            ));
        }
        if value > limit {
            return Err(format!("'{key}'={value} exceeds configured limit {limit}"));
        }
    }
    Ok(())
}

/// Parse the sidecar response into a success/error verdict.
///
/// Returns `Ok((success, data, error))` where `success` defaults to `false`
/// when the field is absent from the response (fail-closed). This fixes the
/// previously divergent defaults: `computer_use.rs` used `unwrap_or(false)`
/// while `browser.rs` used `unwrap_or(true)`.
pub fn parse_sidecar_response(response: &Value) -> (bool, Value, Option<String>) {
    let success = response
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if success {
        let data = response.get("data").cloned().unwrap_or_else(|| json!({}));
        (true, data, None)
    } else {
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| "sidecar reported failure without message".to_string());
        (false, Value::Null, Some(error))
    }
}

/// Build the full sidecar action payload with policy and metadata envelopes.
///
/// This consolidates the duplicate envelope construction that was previously
/// inlined in both `computer_use.rs` and `browser.rs`. The `allowed_domains`
/// parameter is now required (was hardcoded to empty in `computer_use.rs`).
pub fn build_sidecar_payload(
    action: &str,
    params: Value,
    allowed_domains: &[String],
    window_allowlist: &[String],
    max_coordinate_x: Option<i64>,
    max_coordinate_y: Option<i64>,
    session_name: Option<&str>,
    source: &str,
) -> Value {
    json!({
        "action": action,
        "params": params,
        "policy": {
            "allowed_domains": allowed_domains,
            "window_allowlist": window_allowlist,
            "max_coordinate_x": max_coordinate_x,
            "max_coordinate_y": max_coordinate_y,
        },
        "metadata": {
            "session_name": session_name,
            "source": source,
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
        }
    })
}

/// Check whether a browser backend string represents the computer_use backend.
///
/// This consolidates the string-matching logic that was duplicated in
/// `doctor/mod.rs` (which compared against both `"computer_use"` and
/// `"computer-use"`). The canonical backend name is `"computer_use"` but
/// the sidecar also accepts `"computer-use"`.
pub fn is_computer_use_backend(backend: &str) -> bool {
    let key = backend.trim().to_ascii_lowercase().replace('-', "_");
    key == "computer_use"
}

pub async fn post_sidecar_action(
    endpoint: &str,
    api_key: Option<&str>,
    timeout_ms: u64,
    payload: &Value,
) -> Result<Value, String> {
    let client = crate::config::build_runtime_proxy_client_with_timeouts(
        "tool.computer_use_sidecar",
        timeout_ms / 1000 + 1,
        5,
    );
    let mut req = client.post(endpoint).json(payload);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn derive_health_url_falls_back_to_default_on_invalid_url() {
        assert_eq!(
            derive_health_url("not-a-url"),
            "http://127.0.0.1:8787/health"
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
    fn bind_addr_from_endpoint_rejects_invalid() {
        assert!(bind_addr_from_endpoint("not-a-url").is_none());
    }

    #[test]
    fn bind_addr_from_endpoint_rejects_hostname_only() {
        assert!(bind_addr_from_endpoint("http://localhost/v1/actions").is_none());
    }

    #[test]
    fn default_bind_constant_is_valid() {
        assert!(DEFAULT_BIND.parse::<SocketAddr>().is_ok());
    }

    #[test]
    fn health_poll_constants_are_reasonable() {
        assert!(HEALTH_POLL_TIMEOUT.as_secs() >= 1);
        assert!(HEALTH_POLL_INTERVAL < HEALTH_POLL_TIMEOUT);
    }

    #[tokio::test]
    async fn probe_health_returns_false_for_unreachable_url() {
        let result = probe_health("http://192.0.2.1:8787/health").await;
        assert!(!result, "unreachable URL should return false");
    }

    #[tokio::test]
    async fn probe_health_returns_false_for_invalid_url() {
        let result = probe_health("not-a-url").await;
        assert!(!result, "invalid URL should return false");
    }

    #[tokio::test]
    async fn post_sidecar_action_returns_error_for_unreachable() {
        let payload = json!({"action": "window_list", "params": {}});
        let result =
            post_sidecar_action("http://192.0.2.1:8787/v1/actions", None, 1000, &payload).await;
        assert!(result.is_err(), "unreachable sidecar should return error");
        let err = result.unwrap_err();
        assert!(
            err.contains("sidecar request failed"),
            "error should mention request failure: {err}"
        );
    }

    #[tokio::test]
    async fn post_sidecar_action_sends_bearer_auth_when_key_provided() {
        use wiremock::matchers::{bearer_token, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/actions"))
            .and(bearer_token("test-api-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "data": {"windows": []}
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/actions", server.uri());
        let payload = json!({"action": "window_list", "params": {}});
        let result = post_sidecar_action(&endpoint, Some("test-api-key"), 5000, &payload).await;
        assert!(
            result.is_ok(),
            "should succeed with valid mock: {:?}",
            result
        );
        let body = result.unwrap();
        assert_eq!(body["success"], true);
    }

    #[tokio::test]
    async fn post_sidecar_action_returns_error_on_non_200() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/actions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/actions", server.uri());
        let payload = json!({"action": "window_list", "params": {}});
        let result = post_sidecar_action(&endpoint, None, 5000, &payload).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("HTTP 500"),
            "error should mention HTTP status: {err}"
        );
    }

    #[tokio::test]
    async fn post_sidecar_action_returns_error_on_invalid_json_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/actions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1/actions", server.uri());
        let payload = json!({"action": "window_list", "params": {}});
        let result = post_sidecar_action(&endpoint, None, 5000, &payload).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("invalid JSON"),
            "error should mention invalid JSON: {err}"
        );
    }

    #[tokio::test]
    async fn wait_for_healthy_returns_false_on_timeout() {
        let result = wait_for_healthy("http://192.0.2.1:8787/health").await;
        assert!(!result, "unreachable sidecar should time out");
    }

    #[test]
    fn validate_rejects_missing_key_press_key() {
        let params = serde_json::Map::new();
        assert!(validate_computer_use_action("key_press", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_long_key_press() {
        let mut params = serde_json::Map::new();
        params.insert("key".to_string(), json!("a".repeat(33)));
        assert!(validate_computer_use_action("key_press", &params, None, None).is_err());
    }

    #[test]
    fn validate_accepts_valid_key_press() {
        let mut params = serde_json::Map::new();
        params.insert("key".to_string(), json!("ctrl+l"));
        assert!(validate_computer_use_action("key_press", &params, None, None).is_ok());
    }

    #[test]
    fn validate_rejects_invalid_key_press_chars() {
        let mut params = serde_json::Map::new();
        params.insert("key".to_string(), json!("Ctrl-Alt"));
        assert!(validate_computer_use_action("key_press", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_long_key_type_text() {
        let mut params = serde_json::Map::new();
        params.insert("text".to_string(), json!("x".repeat(4097)));
        assert!(validate_computer_use_action("key_type", &params, None, None).is_err());
    }

    #[test]
    fn validate_accepts_valid_key_type() {
        let mut params = serde_json::Map::new();
        params.insert("text".to_string(), json!("hello world"));
        assert!(validate_computer_use_action("key_type", &params, None, None).is_ok());
    }

    #[test]
    fn validate_rejects_missing_mouse_click_coords() {
        let params = serde_json::Map::new();
        assert!(validate_computer_use_action("mouse_click", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_negative_coordinate() {
        let mut params = serde_json::Map::new();
        params.insert("x".to_string(), json!(-1));
        params.insert("y".to_string(), json!(10));
        assert!(validate_computer_use_action("mouse_click", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_coordinate_exceeding_max() {
        let mut params = serde_json::Map::new();
        params.insert("x".to_string(), json!(2000));
        params.insert("y".to_string(), json!(10));
        assert!(
            validate_computer_use_action("mouse_click", &params, Some(1920), Some(1080)).is_err()
        );
    }

    #[test]
    fn validate_accepts_coordinate_within_max() {
        let mut params = serde_json::Map::new();
        params.insert("x".to_string(), json!(100));
        params.insert("y".to_string(), json!(200));
        assert!(
            validate_computer_use_action("mouse_click", &params, Some(1920), Some(1080)).is_ok()
        );
    }

    #[test]
    fn validate_rejects_long_app_name() {
        let mut params = serde_json::Map::new();
        params.insert("app".to_string(), json!("a".repeat(257)));
        assert!(validate_computer_use_action("app_launch", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_too_many_app_launch_args() {
        let mut params = serde_json::Map::new();
        params.insert("app".to_string(), json!("chrome"));
        let args: Vec<String> = (0..33).map(|i| format!("arg{i}")).collect();
        params.insert("args".to_string(), json!(args));
        assert!(validate_computer_use_action("app_launch", &params, None, None).is_err());
    }

    #[test]
    fn validate_rejects_app_terminate_without_app_or_pid() {
        let params = serde_json::Map::new();
        assert!(validate_computer_use_action("app_terminate", &params, None, None).is_err());
    }

    #[test]
    fn validate_accepts_app_terminate_with_pid() {
        let mut params = serde_json::Map::new();
        params.insert("pid".to_string(), json!(1234));
        assert!(validate_computer_use_action("app_terminate", &params, None, None).is_ok());
    }

    #[test]
    fn validate_rejects_window_focus_without_selector() {
        let params = serde_json::Map::new();
        assert!(validate_computer_use_action("window_focus", &params, None, None).is_err());
    }

    #[test]
    fn validate_accepts_window_focus_with_window_id() {
        let mut params = serde_json::Map::new();
        params.insert("window_id".to_string(), json!("0x123456"));
        assert!(validate_computer_use_action("window_focus", &params, None, None).is_ok());
    }

    #[test]
    fn validate_rejects_long_window_query() {
        let mut params = serde_json::Map::new();
        params.insert("query".to_string(), json!("x".repeat(257)));
        assert!(validate_computer_use_action("window_list", &params, None, None).is_err());
    }

    #[test]
    fn validate_allows_unknown_action() {
        let params = serde_json::Map::new();
        assert!(validate_computer_use_action("screen_capture", &params, None, None).is_ok());
    }

    #[test]
    fn parse_sidecar_response_success() {
        let response = json!({"success": true, "data": {"windows": []}});
        let (success, data, error) = parse_sidecar_response(&response);
        assert!(success);
        assert!(data.is_object());
        assert!(error.is_none());
    }

    #[test]
    fn parse_sidecar_response_failure() {
        let response = json!({"success": false, "error": "something went wrong"});
        let (success, data, error) = parse_sidecar_response(&response);
        assert!(!success);
        assert!(data.is_null());
        assert_eq!(error.as_deref(), Some("something went wrong"));
    }

    #[test]
    fn parse_sidecar_response_defaults_to_failure_when_missing() {
        let response = json!({"data": {"windows": []}});
        let (success, _, _) = parse_sidecar_response(&response);
        assert!(
            !success,
            "missing 'success' field must default to false (fail-closed)"
        );
    }

    #[test]
    fn parse_sidecar_response_failure_without_error_message() {
        let response = json!({"success": false});
        let (success, _, error) = parse_sidecar_response(&response);
        assert!(!success);
        assert!(error.is_some());
    }

    #[test]
    fn build_sidecar_payload_includes_all_fields() {
        let payload = build_sidecar_payload(
            "mouse_click",
            json!({"x": 100, "y": 200}),
            &["example.com".to_string()],
            &["firefox".to_string()],
            Some(1920),
            Some(1080),
            Some("test-session"),
            "topclaw.test",
        );
        assert_eq!(payload["action"], "mouse_click");
        assert_eq!(payload["params"]["x"], 100);
        assert_eq!(payload["policy"]["allowed_domains"][0], "example.com");
        assert_eq!(payload["policy"]["window_allowlist"][0], "firefox");
        assert_eq!(payload["policy"]["max_coordinate_x"], 1920);
        assert_eq!(payload["policy"]["max_coordinate_y"], 1080);
        assert_eq!(payload["metadata"]["session_name"], "test-session");
        assert_eq!(payload["metadata"]["source"], "topclaw.test");
        assert!(payload["metadata"]["version"].is_string());
        assert!(payload["metadata"]["platform"].is_string());
    }

    #[test]
    fn is_computer_use_backend_matches_canonical_and_dash_variant() {
        assert!(is_computer_use_backend("computer_use"));
        assert!(is_computer_use_backend("computer-use"));
        assert!(is_computer_use_backend("COMPUTER_USE"));
        assert!(is_computer_use_backend(" Computer-Use "));
        assert!(!is_computer_use_backend("agent_browser"));
        assert!(!is_computer_use_backend("rust_native"));
        assert!(!is_computer_use_backend("auto"));
        assert!(!is_computer_use_backend("playwright"));
    }
}
