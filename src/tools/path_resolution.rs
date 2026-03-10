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
    let mut parent = candidate
        .parent()
        .ok_or_else(|| "Invalid path: missing parent directory".to_string())?;
    let resolved_parent = loop {
        match tokio::fs::canonicalize(parent).await {
            Ok(resolved) => break resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = parent.parent().ok_or_else(|| {
                    format!("Failed to resolve file path: no existing parent for {raw_path}")
                })?;
            }
            Err(error) => return Err(format_path_resolution_error(raw_path, error)),
        }
    };

    if !security.is_resolved_path_allowed(&resolved_parent) {
        return Err(security.resolved_path_violation_message(&resolved_parent));
    }

    Ok(candidate)
}

pub(super) async fn verify_write_target_still_allowed(
    security: &SecurityPolicy,
    target: &Path,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "Invalid path: missing parent directory".to_string())?;
    let resolved_parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(|e| format!("Failed to resolve file path: {e}"))?;

    if !security.is_resolved_path_allowed(&resolved_parent) {
        return Err(security.resolved_path_violation_message(&resolved_parent));
    }

    if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "Refusing to write through symlink: {}",
                target.display()
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AutonomyLevel, SecurityPolicy};

    #[tokio::test]
    async fn verify_write_target_rejects_symlinked_parent_outside_allowed_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let link_parent = workspace.path().join("nested");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link_parent).expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), &link_parent).expect("symlink");

        let security = SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace.path().to_path_buf(),
            ..SecurityPolicy::default()
        };

        let result =
            verify_write_target_still_allowed(&security, &link_parent.join("file.txt")).await;
        assert!(result.is_err());
    }
}
