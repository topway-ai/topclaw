//! Shared sidecar client utilities for the `computer_use`,
//! `computer_use_sidecar_start`, and `browser` tools.
//!
//! This module eliminates the previously duplicated health-probe, sidecar-spawn,
//! URL-derivation, and action-POST logic that was copied across tool files.

use serde_json::Value;
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
}
