use crate::security::SecurityPolicy;
use std::path::{Path, PathBuf};

fn format_path_resolution_error(raw_path: &str, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("File not found: {raw_path}")
    } else {
        format!("Failed to resolve file path: {error}")
    }
}

pub(super) fn resolve_tool_path_candidate(security: &SecurityPolicy, raw_path: &str) -> PathBuf {
    let path = Path::new(raw_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        security.workspace_dir.join(path)
    }
}

pub(super) async fn resolve_allowed_existing_path(
    security: &SecurityPolicy,
    raw_path: &str,
) -> Result<PathBuf, String> {
    if !security.is_path_allowed(raw_path) {
        return Err(format!("Path not allowed by security policy: {raw_path}"));
    }

    let candidate = resolve_tool_path_candidate(security, raw_path);
    let resolved = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|e| format_path_resolution_error(raw_path, e))?;

    if !security.is_resolved_path_allowed(&resolved) {
        return Err(security.resolved_path_violation_message(&resolved));
    }

    Ok(resolved)
}

pub(super) async fn resolve_allowed_parent_and_target(
    security: &SecurityPolicy,
    raw_path: &str,
) -> Result<PathBuf, String> {
    if !security.is_path_allowed(raw_path) {
        return Err(format!("Path not allowed by security policy: {raw_path}"));
    }

    let candidate = resolve_tool_path_candidate(security, raw_path);
    let parent = candidate
        .parent()
        .ok_or_else(|| "Invalid path: missing parent directory".to_string())?;
    let resolved_parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|e| format_path_resolution_error(raw_path, e))?;

    if !security.is_resolved_path_allowed(&resolved_parent) {
        return Err(security.resolved_path_violation_message(&resolved_parent));
    }

    let file_name = candidate
        .file_name()
        .ok_or_else(|| "Invalid path: missing file name".to_string())?;
    Ok(resolved_parent.join(file_name))
}
