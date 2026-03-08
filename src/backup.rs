use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

const BACKUP_MANIFEST_FILE: &str = "manifest.toml";
const BACKUP_PAYLOAD_DIR: &str = "config-root";
const BACKUP_RESTORE_GUIDE_FILE: &str = "RESTORE.md";
const BACKUP_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    format_version: u32,
    created_at: String,
    source_config_dir: String,
    includes_logs: bool,
    file_count: usize,
    total_bytes: u64,
    files: Vec<BackupFileEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupFileEntry {
    relative_path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug)]
struct BackupBuildStats {
    file_count: usize,
    total_bytes: u64,
    files: Vec<BackupFileEntry>,
}

pub(crate) async fn handle_command(backup_command: topclaw::BackupCommands) -> Result<()> {
    match backup_command {
        topclaw::BackupCommands::Create {
            destination,
            include_logs,
        } => {
            let (config_dir, _) = crate::config::schema::resolve_runtime_dirs_for_onboarding()
                .await
                .context("Failed to resolve TopClaw runtime directories for backup")?;
            let backup_root =
                create_backup_bundle_from_source(&config_dir, &destination, include_logs)?;

            println!("Backup created.");
            println!("  Source: {}", config_dir.display());
            println!("  Bundle: {}", backup_root.display());
            println!(
                "  Includes logs: {}",
                if include_logs { "yes" } else { "no" }
            );
            let manifest = load_manifest(&backup_root.join(BACKUP_MANIFEST_FILE))?;
            println!("  Files: {}", manifest.file_count);
            println!("  Bytes: {}", manifest.total_bytes);
            println!(
                "Restore with: topclaw backup restore {}",
                backup_root.display()
            );
            Ok(())
        }
        topclaw::BackupCommands::Inspect { source } => {
            let summary = inspect_backup_bundle(&source)?;
            println!("Backup bundle verified.");
            println!("  Bundle: {}", source.display());
            println!("  Created: {}", summary.created_at);
            println!("  Source: {}", summary.source_config_dir);
            println!("  Files: {}", summary.file_count);
            println!("  Bytes: {}", summary.total_bytes);
            println!(
                "  Includes logs: {}",
                if summary.includes_logs { "yes" } else { "no" }
            );
            Ok(())
        }
        topclaw::BackupCommands::Restore { source, force } => {
            let (target_config_dir, _) =
                crate::config::schema::resolve_runtime_dirs_for_onboarding()
                    .await
                    .context("Failed to resolve TopClaw runtime directories for restore")?;
            let rollback_dir = restore_backup_bundle_to_target(&source, &target_config_dir, force)?;
            crate::config::schema::persist_active_workspace_config_dir(&target_config_dir).await?;

            println!("Backup restored.");
            println!("  Source: {}", source.display());
            println!("  Target: {}", target_config_dir.display());
            if let Some(rollback_dir) = rollback_dir {
                println!("  Previous target preserved at: {}", rollback_dir.display());
            }
            println!("If TopClaw is managed by a background service, restart it now.");
            Ok(())
        }
    }
}

fn create_backup_bundle_from_source(
    source_config_dir: &Path,
    destination: &Path,
    include_logs: bool,
) -> Result<PathBuf> {
    let source_config_dir = source_config_dir.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize source config dir {}",
            source_config_dir.display()
        )
    })?;
    if !source_config_dir.is_dir() {
        bail!(
            "TopClaw config directory does not exist or is not a directory: {}",
            source_config_dir.display()
        );
    }

    let backup_root = normalize_destination_path(destination)?;
    if backup_root.starts_with(&source_config_dir) {
        bail!(
            "Backup destination cannot be inside the source config directory: {}",
            backup_root.display()
        );
    }
    if backup_root.exists() && !is_directory_empty(&backup_root)? {
        bail!(
            "Backup destination already exists and is not empty: {}",
            backup_root.display()
        );
    }

    std::fs::create_dir_all(&backup_root).with_context(|| {
        format!(
            "Failed to create backup directory {}",
            backup_root.display()
        )
    })?;

    let payload_dir = backup_root.join(BACKUP_PAYLOAD_DIR);
    let mut stats = BackupBuildStats {
        file_count: 0,
        total_bytes: 0,
        files: Vec::new(),
    };
    copy_dir_recursive_secure(
        &source_config_dir,
        &payload_dir,
        Path::new(""),
        include_logs,
        &mut stats,
    )?;

    let manifest = BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        created_at: Utc::now().to_rfc3339(),
        source_config_dir: source_config_dir.display().to_string(),
        includes_logs: include_logs,
        file_count: stats.file_count,
        total_bytes: stats.total_bytes,
        files: stats.files,
    };
    write_manifest(&backup_root.join(BACKUP_MANIFEST_FILE), &manifest)?;
    write_restore_guide(&backup_root, &manifest)?;

    Ok(backup_root)
}

fn inspect_backup_bundle(source: &Path) -> Result<BackupManifest> {
    let source_root = source
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize backup source {}", source.display()))?;
    let manifest_path = source_root.join(BACKUP_MANIFEST_FILE);
    let payload_dir = source_root.join(BACKUP_PAYLOAD_DIR);
    let manifest = load_manifest(&manifest_path)?;
    if !payload_dir.is_dir() {
        bail!(
            "Backup payload directory is missing: {}",
            payload_dir.display()
        );
    }
    verify_backup_payload(&payload_dir, &manifest)?;
    Ok(manifest)
}

fn restore_backup_bundle_to_target(
    source: &Path,
    target_config_dir: &Path,
    force: bool,
) -> Result<Option<PathBuf>> {
    let source_root = source
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize backup source {}", source.display()))?;
    let manifest_path = source_root.join(BACKUP_MANIFEST_FILE);
    let payload_dir = source_root.join(BACKUP_PAYLOAD_DIR);

    let manifest = load_manifest(&manifest_path)?;
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        bail!(
            "Unsupported backup format version {} in {}",
            manifest.format_version,
            manifest_path.display()
        );
    }
    if !payload_dir.is_dir() {
        bail!(
            "Backup payload directory is missing: {}",
            payload_dir.display()
        );
    }
    verify_backup_payload(&payload_dir, &manifest)?;

    let target_parent = target_config_dir.parent().with_context(|| {
        format!(
            "Target config directory must have a parent: {}",
            target_config_dir.display()
        )
    })?;
    std::fs::create_dir_all(target_parent).with_context(|| {
        format!(
            "Failed to create target parent directory {}",
            target_parent.display()
        )
    })?;

    let staging_dir =
        target_parent.join(format!(".topclaw-restore-staging-{}", uuid::Uuid::new_v4()));
    let mut stats = BackupBuildStats {
        file_count: 0,
        total_bytes: 0,
        files: Vec::new(),
    };
    copy_dir_recursive_secure(&payload_dir, &staging_dir, Path::new(""), true, &mut stats)?;

    let rollback_dir = if target_config_dir.exists() && !is_directory_empty(target_config_dir)? {
        if !force {
            let _ = std::fs::remove_dir_all(&staging_dir);
            bail!(
                "Target config directory already exists and is not empty: {}. Re-run with --force to replace it.",
                target_config_dir.display()
            );
        }
        let rollback_dir = target_parent.join(format!(
            ".topclaw-restore-backup-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ));
        std::fs::rename(target_config_dir, &rollback_dir).with_context(|| {
            format!(
                "Failed to move existing target config directory {} to rollback location {}",
                target_config_dir.display(),
                rollback_dir.display()
            )
        })?;
        Some(rollback_dir)
    } else {
        if target_config_dir.exists() {
            std::fs::remove_dir_all(target_config_dir).with_context(|| {
                format!(
                    "Failed to remove empty target config directory {}",
                    target_config_dir.display()
                )
            })?;
        }
        None
    };

    if let Err(error) = std::fs::rename(&staging_dir, target_config_dir) {
        let _ = std::fs::remove_dir_all(&staging_dir);
        if let Some(rollback_dir) = rollback_dir.as_ref() {
            let _ = std::fs::rename(rollback_dir, target_config_dir);
        }
        return Err(error).with_context(|| {
            format!(
                "Failed to move restored backup into target location {}",
                target_config_dir.display()
            )
        });
    }

    Ok(rollback_dir)
}

fn copy_dir_recursive_secure(
    source: &Path,
    target: &Path,
    relative_root: &Path,
    include_logs: bool,
    stats: &mut BackupBuildStats,
) -> Result<()> {
    let source_meta = std::fs::symlink_metadata(source)
        .with_context(|| format!("Failed to read metadata for {}", source.display()))?;
    if source_meta.file_type().is_symlink() {
        bail!("Refusing to copy symlinked path: {}", source.display());
    }
    if !source_meta.is_dir() {
        bail!("Expected directory for backup copy: {}", source.display());
    }

    std::fs::create_dir_all(target)
        .with_context(|| format!("Failed to create directory {}", target.display()))?;
    copy_permissions(source, target)?;

    for entry in std::fs::read_dir(source)
        .with_context(|| format!("Failed to read directory {}", source.display()))?
    {
        let entry = entry?;
        let file_name = entry.file_name();
        if !include_logs && file_name.as_os_str() == OsStr::new("logs") {
            continue;
        }

        let source_path = entry.path();
        let target_path = target.join(&file_name);
        let metadata = std::fs::symlink_metadata(&source_path)
            .with_context(|| format!("Failed to read metadata for {}", source_path.display()))?;

        if metadata.file_type().is_symlink() {
            bail!(
                "Refusing to copy symlink within backup source: {}",
                source_path.display()
            );
        }

        let next_relative = relative_root.join(&file_name);
        if metadata.is_dir() {
            copy_dir_recursive_secure(&source_path, &target_path, &next_relative, true, stats)?;
        } else if metadata.is_file() {
            std::fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "Failed to copy file from {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
            copy_permissions(&source_path, &target_path)?;
            stats.file_count += 1;
            stats.total_bytes += metadata.len();
            stats.files.push(BackupFileEntry {
                relative_path: relative_path_to_string(&next_relative)?,
                size_bytes: metadata.len(),
                sha256: sha256_file(&target_path)?,
            });
        }
    }

    Ok(())
}

fn copy_permissions(source: &Path, target: &Path) -> Result<()> {
    let permissions = std::fs::metadata(source)
        .with_context(|| format!("Failed to read permissions for {}", source.display()))?
        .permissions();
    std::fs::set_permissions(target, permissions)
        .with_context(|| format!("Failed to apply permissions to {}", target.display()))?;
    Ok(())
}

fn normalize_destination_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Failed to resolve current working directory")?
            .join(path)
    };
    let file_name = absolute.file_name().with_context(|| {
        format!(
            "Backup destination must include a directory name: {}",
            absolute.display()
        )
    })?;
    let parent = absolute.parent().with_context(|| {
        format!(
            "Backup destination must have a writable parent directory: {}",
            absolute.display()
        )
    })?;
    let parent = parent.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize backup destination parent {}",
            parent.display()
        )
    })?;
    Ok(parent.join(file_name))
}

fn is_directory_empty(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if !path.is_dir() {
        bail!(
            "Expected directory path, found non-directory: {}",
            path.display()
        );
    }
    Ok(std::fs::read_dir(path)
        .with_context(|| format!("Failed to inspect directory {}", path.display()))?
        .next()
        .is_none())
}

fn write_manifest(path: &Path, manifest: &BackupManifest) -> Result<()> {
    let manifest_raw =
        toml::to_string_pretty(manifest).context("Failed to serialize backup manifest")?;
    std::fs::write(path, manifest_raw)
        .with_context(|| format!("Failed to write backup manifest {}", path.display()))?;
    Ok(())
}

fn load_manifest(path: &Path) -> Result<BackupManifest> {
    let manifest_raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read backup manifest {}", path.display()))?;
    toml::from_str(&manifest_raw).context("Failed to parse backup manifest")
}

fn write_restore_guide(backup_root: &Path, manifest: &BackupManifest) -> Result<()> {
    let restore_guide = format!(
        "# TopClaw Backup Restore Guide

Created: {}
Source config dir: `{}`
Files: {}
Bytes: {}
Includes logs: {}

## Restore on this machine

```bash
topclaw backup restore {}
```

## Restore and replace an existing install

```bash
topclaw backup restore {} --force
```

## Move to another machine

1. Install the same or newer TopClaw version on the target machine.
2. Copy this entire backup bundle directory to the target machine.
3. Run `topclaw backup restore <bundle_dir> --force` on the target machine if TopClaw was already initialized there.
4. Restart any TopClaw background service after restore.

## Safety notes

- This backup contains secrets, auth state, memories, preferences, and installed skills.
- Keep the bundle in a private location.
- During `--force` restore, TopClaw preserves the previous target config as a sibling rollback directory instead of deleting it immediately.
",
        manifest.created_at,
        manifest.source_config_dir,
        manifest.file_count,
        manifest.total_bytes,
        if manifest.includes_logs { "yes" } else { "no" },
        backup_root.display(),
        backup_root.display()
    );
    std::fs::write(backup_root.join(BACKUP_RESTORE_GUIDE_FILE), restore_guide).with_context(
        || {
            format!(
                "Failed to write restore guide {}",
                backup_root.join(BACKUP_RESTORE_GUIDE_FILE).display()
            )
        },
    )?;
    Ok(())
}

fn verify_backup_payload(payload_dir: &Path, manifest: &BackupManifest) -> Result<()> {
    let mut verified_files = 0_usize;
    let mut verified_bytes = 0_u64;

    for file in &manifest.files {
        let relative_path = PathBuf::from(&file.relative_path);
        let full_path = payload_dir.join(&relative_path);
        let metadata = std::fs::metadata(&full_path)
            .with_context(|| format!("Backup payload file is missing: {}", full_path.display()))?;
        if !metadata.is_file() {
            bail!("Backup payload path is not a file: {}", full_path.display());
        }
        if metadata.len() != file.size_bytes {
            bail!(
                "Backup payload file size mismatch for {}: expected {}, got {}",
                full_path.display(),
                file.size_bytes,
                metadata.len()
            );
        }
        let actual_sha256 = sha256_file(&full_path)?;
        if actual_sha256 != file.sha256 {
            bail!(
                "Backup payload checksum mismatch for {}",
                full_path.display()
            );
        }
        verified_files += 1;
        verified_bytes += metadata.len();
    }

    if verified_files != manifest.file_count {
        bail!(
            "Backup manifest file count mismatch: expected {}, verified {}",
            manifest.file_count,
            verified_files
        );
    }
    if verified_bytes != manifest.total_bytes {
        bail!(
            "Backup manifest byte count mismatch: expected {}, verified {}",
            manifest.total_bytes,
            verified_bytes
        );
    }
    Ok(())
}

fn relative_path_to_string(path: &Path) -> Result<String> {
    let value = path.to_str().with_context(|| {
        format!(
            "Backup contains a non-UTF-8 path that cannot be represented in the manifest: {}",
            path.display()
        )
    })?;
    Ok(value.replace('\\', "/"))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open file for checksum {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read file for checksum {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::{
        create_backup_bundle_from_source, load_manifest, restore_backup_bundle_to_target,
        BACKUP_MANIFEST_FILE, BACKUP_PAYLOAD_DIR, BACKUP_RESTORE_GUIDE_FILE,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn create_backup_omits_logs_by_default() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("bundle");

        fs::create_dir_all(source.join("workspace/skills")).unwrap();
        fs::create_dir_all(source.join("logs")).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();
        fs::write(source.join("workspace/skills/test.txt"), "skill").unwrap();
        fs::write(source.join("logs/runtime.log"), "trace").unwrap();

        let backup_root = create_backup_bundle_from_source(&source, &destination, false).unwrap();

        assert!(backup_root.join(BACKUP_MANIFEST_FILE).is_file());
        assert!(backup_root.join(BACKUP_RESTORE_GUIDE_FILE).is_file());
        assert!(backup_root
            .join(BACKUP_PAYLOAD_DIR)
            .join("config.toml")
            .is_file());
        assert!(backup_root
            .join(BACKUP_PAYLOAD_DIR)
            .join("workspace/skills/test.txt")
            .is_file());
        assert!(!backup_root.join(BACKUP_PAYLOAD_DIR).join("logs").exists());
    }

    #[test]
    fn create_backup_rejects_destination_inside_source() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = source.join("bundle");

        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();

        let error = create_backup_bundle_from_source(&source, &destination, false).unwrap_err();
        assert!(error
            .to_string()
            .contains("Backup destination cannot be inside the source config directory"));
    }

    #[test]
    fn restore_requires_force_for_non_empty_target() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("bundle");
        let target = temp.path().join("target");

        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();
        create_backup_bundle_from_source(&source, &destination, false).unwrap();

        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("config.toml"),
            "default_provider = \"anthropic\"\n",
        )
        .unwrap();

        let error = restore_backup_bundle_to_target(&destination, &target, false).unwrap_err();
        assert!(error
            .to_string()
            .contains("Re-run with --force to replace it."));
    }

    #[test]
    fn restore_replaces_target_with_force() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("bundle");
        let target = temp.path().join("target");

        fs::create_dir_all(source.join("workspace")).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();
        fs::write(source.join("workspace/MEMORY.md"), "remember").unwrap();
        create_backup_bundle_from_source(&source, &destination, false).unwrap();

        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("stale.txt"), "old").unwrap();

        restore_backup_bundle_to_target(&destination, &target, true).unwrap();

        assert!(target.join("config.toml").is_file());
        assert!(target.join("workspace/MEMORY.md").is_file());
        assert!(!target.join("stale.txt").exists());
        assert!(temp.path().read_dir().unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".topclaw-restore-backup-")));
    }

    #[test]
    fn create_backup_records_checksums_and_counts() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("bundle");

        fs::create_dir_all(source.join("workspace")).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();
        fs::write(source.join("workspace/MEMORY.md"), "remember").unwrap();

        create_backup_bundle_from_source(&source, &destination, false).unwrap();
        let manifest = load_manifest(&destination.join(BACKUP_MANIFEST_FILE)).unwrap();

        assert_eq!(manifest.file_count, 2);
        assert_eq!(manifest.files.len(), 2);
        assert!(manifest.total_bytes > 0);
        assert!(manifest.files.iter().all(|entry| entry.sha256.len() == 64));
    }

    #[test]
    fn restore_rejects_checksum_mismatch() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("bundle");
        let target = temp.path().join("target");

        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("config.toml"),
            "default_provider = \"openrouter\"\n",
        )
        .unwrap();
        create_backup_bundle_from_source(&source, &destination, false).unwrap();
        fs::write(
            destination.join(BACKUP_PAYLOAD_DIR).join("config.toml"),
            "tampered\n",
        )
        .unwrap();

        let error = restore_backup_bundle_to_target(&destination, &target, false).unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
    }
}
