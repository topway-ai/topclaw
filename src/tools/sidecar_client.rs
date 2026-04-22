//! Shared sidecar client utilities for the `computer_use` and
//! `computer_use_sidecar_start` tools.
//!
//! This module eliminates the previously duplicated health-probe, sidecar-spawn,
//! and URL-derivation logic that was copied across both tool files.

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
pub fn spawn_sidecar_process(
    bind: &str,
    api_key: Option<&str>,
) -> Result<u32, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bind_addr_from_endpoint_rejects_invalid() {
        assert!(bind_addr_from_endpoint("not-a-url").is_none());
    }
}
