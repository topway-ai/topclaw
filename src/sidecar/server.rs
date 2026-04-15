//! Axum HTTP server for the computer-use sidecar.

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;

/// Request envelope as documented in `docs/computer-use-sidecar-protocol.md`.
#[derive(Debug, Deserialize)]
pub struct ActionRequest {
    pub action: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct Policy {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub window_allowlist: Vec<String>,
    #[serde(default)]
    pub max_coordinate_x: Option<i64>,
    #[serde(default)]
    pub max_coordinate_y: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ActionResponse {
    pub fn ok(data: Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

struct AppState {
    api_key: Option<String>,
}

/// Run the sidecar server on `addr` until Ctrl-C or a socket error.
pub async fn run_server(addr: SocketAddr, api_key: Option<String>) -> Result<()> {
    let state = Arc::new(AppState { api_key });

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/actions", post(handle_action))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind sidecar socket on {addr}"))?;

    tracing::info!(target: "topclaw::sidecar", bind = %addr, "computer-use sidecar listening");
    axum::serve(listener, app)
        .await
        .context("axum serve failed")
}

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({"status": "ok", "service": "topclaw-computer-use-sidecar"})),
    )
}

async fn handle_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ActionRequest>,
) -> impl IntoResponse {
    if let Some(expected) = state.api_key.as_deref() {
        let provided = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        if provided != Some(expected) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ActionResponse::err("unauthorized")),
            );
        }
    }

    match super::linux::dispatch(&req).await {
        Ok(data) => (StatusCode::OK, Json(ActionResponse::ok(data))),
        Err(e) => (StatusCode::OK, Json(ActionResponse::err(format!("{e:#}")))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_ok_serializes_without_error() {
        let r = ActionResponse::ok(json!({"x": 1}));
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"success\":true"));
        assert!(!s.contains("\"error\""));
    }

    #[test]
    fn response_err_serializes_without_data() {
        let r = ActionResponse::err("boom");
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"success\":false"));
        assert!(s.contains("boom"));
        assert!(!s.contains("\"data\""));
    }
}
