use anyhow::{Context, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

pub(super) fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

pub(super) const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ActiveWorkspaceState {
    pub(super) config_dir: String,
}

/// Return the default config directory (`~/.topclaw`).
///
/// This is the single source of truth for the default config root.
/// All callers that need the default path should use this function
/// instead of hardcoding `home.join(".topclaw")`.
pub fn default_config_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(".topclaw"))
}

fn active_workspace_state_path(marker_root: &Path) -> PathBuf {
    marker_root.join(ACTIVE_WORKSPACE_STATE_FILE)
}

/// Returns `true` if `path` lives under the OS temp directory.
#[cfg(not(test))]
fn is_temp_directory(path: &Path) -> bool {
    let temp = std::env::temp_dir();
    // Canonicalize when possible to handle symlinks (macOS /var -> /private/var)
    let canon_temp = temp.canonicalize().unwrap_or_else(|_| temp.clone());
    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canon_path.starts_with(&canon_temp)
}

async fn load_persisted_workspace_dirs(
    default_config_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let state_path = active_workspace_state_path(default_config_dir);
    if !state_path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&state_path).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "Failed to read active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let state: ActiveWorkspaceState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "Failed to parse active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let raw_config_dir = state.config_dir.trim();
    if raw_config_dir.is_empty() {
        tracing::warn!(
            "Ignoring active workspace marker {} because config_dir is empty",
            state_path.display()
        );
        return Ok(None);
    }

    let parsed_dir = PathBuf::from(raw_config_dir);
    let config_dir = if parsed_dir.is_absolute() {
        parsed_dir
    } else {
        default_config_dir.join(parsed_dir)
    };
    Ok(Some((config_dir.clone(), config_dir.join("workspace"))))
}

async fn remove_active_workspace_marker(marker_root: &Path) -> Result<()> {
    let state_path = active_workspace_state_path(marker_root);
    if !state_path.exists() {
        return Ok(());
    }

    fs::remove_file(&state_path).await.with_context(|| {
        format!(
            "Failed to clear active workspace marker: {}",
            state_path.display()
        )
    })?;

    if marker_root.exists() {
        super::sync_directory(marker_root).await?;
    }
    Ok(())
}

async fn write_active_workspace_marker(marker_root: &Path, config_dir: &Path) -> Result<()> {
    fs::create_dir_all(marker_root).await.with_context(|| {
        format!(
            "Failed to create active workspace marker root: {}",
            marker_root.display()
        )
    })?;

    let state = ActiveWorkspaceState {
        config_dir: config_dir.to_string_lossy().into_owned(),
    };
    let serialized =
        toml::to_string_pretty(&state).context("Failed to serialize active workspace marker")?;

    let temp_path = marker_root.join(format!(
        ".{ACTIVE_WORKSPACE_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, serialized).await.with_context(|| {
        format!(
            "Failed to write temporary active workspace marker: {}",
            temp_path.display()
        )
    })?;

    let state_path = active_workspace_state_path(marker_root);
    if let Err(error) = fs::rename(&temp_path, &state_path).await {
        let _ = fs::remove_file(&temp_path).await;
        anyhow::bail!(
            "Failed to atomically persist active workspace marker {}: {error}",
            state_path.display()
        );
    }

    super::sync_directory(marker_root).await?;
    Ok(())
}

pub(crate) async fn persist_active_workspace_config_dir(config_dir: &Path) -> Result<()> {
    let default_config_dir = default_config_dir()?;

    // Guard: never persist a temp-directory path as the active workspace.
    // This prevents transient test runs or one-off invocations from hijacking
    // the daemon's config resolution.
    #[cfg(not(test))]
    if is_temp_directory(config_dir) {
        tracing::warn!(
            path = %config_dir.display(),
            "Refusing to persist temp directory as active workspace marker"
        );
        return Ok(());
    }

    if config_dir == default_config_dir {
        remove_active_workspace_marker(&default_config_dir).await?;
        return Ok(());
    }

    // Primary marker lives with the selected config root to keep custom-home
    // layouts self-contained and writable in restricted environments.
    write_active_workspace_marker(config_dir, config_dir).await?;

    // Mirror into the default HOME-scoped root as a best-effort pointer for
    // later auto-discovery. Failure here must not break onboarding/update flows.
    if let Err(error) = write_active_workspace_marker(&default_config_dir, config_dir).await {
        tracing::warn!(
            selected_config_dir = %config_dir.display(),
            default_config_dir = %default_config_dir.display(),
            "Failed to mirror active workspace marker to default HOME config root; continuing with selected-root marker only: {error}"
        );
    }

    Ok(())
}

/// Resolve the config directory for a given workspace directory.
///
/// This function handles three cases:
/// 1. The workspace directory itself contains `config.toml` — use it directly.
/// 2. **Legacy layout fallback**: The workspace directory is a child of a directory
///    that contains `.topclaw/config.toml` (e.g. `~/projects/myproject` where
///    `~/.topclaw/config.toml` exists). This supports users who set
///    `TOPCLAW_WORKSPACE=~/projects/myproject/workspace` before per-workspace
///    config dirs were introduced.
/// 3. Neither exists — treat the workspace directory as the config root.
///
/// The legacy fallback (case 2) should be removed once the migration window
/// closes. Track usage via the `config_dir_legacy_fallback_used` metric if
/// possible.
pub(crate) fn resolve_config_dir_for_workspace(workspace_dir: &Path) -> (PathBuf, PathBuf) {
    let workspace_config_dir = workspace_dir.to_path_buf();
    if workspace_config_dir.join("config.toml").exists() {
        return (
            workspace_config_dir.clone(),
            workspace_config_dir.join("workspace"),
        );
    }

    // Legacy fallback: check parent for `.topclaw/` config directory.
    // This handles the pre-per-workspace-config layout where
    // `TOPCLAW_WORKSPACE=/some/path/workspace` would find config at
    // `/some/path/.topclaw/config.toml`.
    let legacy_config_dir = workspace_dir.parent().map(|parent| parent.join(".topclaw"));
    if let Some(legacy_dir) = legacy_config_dir {
        if legacy_dir.join("config.toml").exists() {
            return (legacy_dir, workspace_config_dir);
        }

        // Heuristic: if the workspace directory is literally named "workspace",
        // assume the parent's `.topclaw/` is the config root even without
        // a config.toml present (the config may be created later during onboarding).
        if workspace_dir
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return (legacy_dir, workspace_config_dir);
        }
    }

    (
        workspace_config_dir.clone(),
        workspace_config_dir.join("workspace"),
    )
}

/// Resolve the current runtime config/workspace directories for onboarding flows.
///
/// This mirrors the same precedence used by `Config::load_or_init()`:
/// `TOPCLAW_CONFIG_DIR` > `TOPCLAW_WORKSPACE` > active workspace marker > defaults.
pub(crate) async fn resolve_runtime_dirs_for_onboarding() -> Result<(PathBuf, PathBuf)> {
    let (default_topclaw_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
    let (config_dir, workspace_dir, _) =
        resolve_runtime_config_dirs(&default_topclaw_dir, &default_workspace_dir).await?;
    Ok((config_dir, workspace_dir))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConfigResolutionSource {
    EnvConfigDir,
    EnvWorkspace,
    ActiveWorkspaceMarker,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::EnvConfigDir => "TOPCLAW_CONFIG_DIR",
            Self::EnvWorkspace => "TOPCLAW_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

pub(super) async fn resolve_runtime_config_dirs(
    default_topclaw_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Ok(custom_config_dir) = std::env::var("TOPCLAW_CONFIG_DIR") {
        let custom_config_dir = custom_config_dir.trim();
        if !custom_config_dir.is_empty() {
            let topclaw_dir = PathBuf::from(custom_config_dir);
            return Ok((
                topclaw_dir.clone(),
                topclaw_dir.join("workspace"),
                ConfigResolutionSource::EnvConfigDir,
            ));
        }
    }

    if let Ok(custom_workspace) = std::env::var("TOPCLAW_WORKSPACE") {
        if !custom_workspace.is_empty() {
            let (topclaw_dir, workspace_dir) =
                resolve_config_dir_for_workspace(&PathBuf::from(custom_workspace));
            return Ok((
                topclaw_dir,
                workspace_dir,
                ConfigResolutionSource::EnvWorkspace,
            ));
        }
    }

    if let Some((topclaw_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_topclaw_dir).await?
    {
        return Ok((
            topclaw_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    Ok((
        default_topclaw_dir.to_path_buf(),
        default_workspace_dir.to_path_buf(),
        ConfigResolutionSource::DefaultConfigDir,
    ))
}
