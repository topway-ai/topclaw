//! Self-update functionality for TopClaw.
//!
//! Downloads and installs the latest release from GitHub.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// GitHub repository for releases
const GITHUB_REPO: &str = "topway-ai/topclaw";
const GITHUB_API_RELEASES: &str = "https://api.github.com/repos/topway-ai/topclaw/releases/latest";
const RELEASE_CHECKSUMS_ASSET: &str = "SHA256SUMS";

/// Release information from GitHub API
#[derive(Debug, serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Get the current version of the binary
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Get the target triple for the current platform
fn get_target_triple() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let target = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "arm") => "armv7-unknown-linux-gnueabihf",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => bail!("Unsupported platform: {}-{}", os, arch),
    };

    Ok(target.to_string())
}

/// Get the binary name for the current platform
fn get_binary_name() -> String {
    if cfg!(windows) {
        "topclaw.exe".to_string()
    } else {
        "topclaw".to_string()
    }
}

/// Get the archive name for a given target
fn get_archive_name(target: &str) -> String {
    if target.contains("windows") {
        format!("topclaw-{}.zip", target)
    } else {
        format!("topclaw-{}.tar.gz", target)
    }
}

/// Fetch the latest release information from GitHub
async fn fetch_latest_release() -> Result<Release> {
    let client = reqwest::Client::builder()
        .user_agent(format!("topclaw/{}", current_version()))
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(GITHUB_API_RELEASES)
        .send()
        .await
        .context("Failed to fetch release information from GitHub")?;

    if !response.status().is_success() {
        bail!("GitHub API returned status: {}", response.status());
    }

    let release: Release = response
        .json()
        .await
        .context("Failed to parse release information")?;

    Ok(release)
}

/// Find the appropriate asset for the current platform
fn find_asset_for_platform(release: &Release) -> Result<&Asset> {
    let target = get_target_triple()?;
    let archive_name = get_archive_name(&target);

    release
        .assets
        .iter()
        .find(|a| a.name == archive_name)
        .with_context(|| {
            format!(
                "No release asset found for platform {} (looking for {})",
                target, archive_name
            )
        })
}

fn find_checksums_asset(release: &Release) -> Result<&Asset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == RELEASE_CHECKSUMS_ASSET)
        .with_context(|| {
            format!("Release is missing required checksum asset '{RELEASE_CHECKSUMS_ASSET}'")
        })
}

async fn download_text_asset(asset: &Asset) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("topclaw/{}", current_version()))
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("Failed to download {}", asset.name))?;

    if !response.status().is_success() {
        bail!(
            "Download failed for {} with status: {}",
            asset.name,
            response.status()
        );
    }

    response
        .text()
        .await
        .with_context(|| format!("Failed to read {}", asset.name))
}

fn parse_expected_sha256(checksums: &str, asset_name: &str) -> Result<String> {
    for line in checksums.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(digest) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        let normalized_name = name.trim_start_matches('*');
        if normalized_name == asset_name {
            anyhow::ensure!(
                digest.len() == 64 && digest.chars().all(|ch| ch.is_ascii_hexdigit()),
                "Invalid SHA256 digest for {asset_name} in {RELEASE_CHECKSUMS_ASSET}"
            );
            return Ok(digest.to_ascii_lowercase());
        }
    }

    bail!(
        "Checksum for {} not found in {}",
        asset_name,
        RELEASE_CHECKSUMS_ASSET
    )
}

fn verify_archive_sha256(
    archive_bytes: &[u8],
    expected_sha256: &str,
    asset_name: &str,
) -> Result<()> {
    let actual = hex::encode(Sha256::digest(archive_bytes));
    anyhow::ensure!(
        actual == expected_sha256,
        "Checksum verification failed for {asset_name}"
    );
    Ok(())
}

/// Download and extract the binary from the release archive
async fn download_binary(asset: &Asset, expected_sha256: &str, temp_dir: &Path) -> Result<PathBuf> {
    let client = reqwest::Client::builder()
        .user_agent(format!("topclaw/{}", current_version()))
        .build()
        .context("Failed to create HTTP client")?;

    tracing::info!("Downloading {}...", asset.name);

    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("Failed to download release archive")?;

    if !response.status().is_success() {
        bail!("Download failed with status: {}", response.status());
    }

    let archive_path = temp_dir.join(&asset.name);
    let archive_bytes = response
        .bytes()
        .await
        .context("Failed to read download content")?;

    verify_archive_sha256(&archive_bytes, expected_sha256, &asset.name)?;

    fs::write(&archive_path, &archive_bytes).context("Failed to write archive to temp file")?;

    tracing::info!("Extracting {}...", asset.name);

    // Extract based on archive type
    if asset.name.ends_with(".tar.gz") {
        extract_tar_gz(&archive_path, temp_dir)?;
    } else if asset.name.ends_with(".zip") {
        extract_zip(&archive_path, temp_dir)?;
    } else {
        bail!("Unsupported archive format: {}", asset.name);
    }

    let binary_name = get_binary_name();
    let binary_path = temp_dir.join(&binary_name);

    if !binary_path.exists() {
        bail!(
            "Binary not found in archive. Expected: {}",
            binary_path.display()
        );
    }

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o755))
            .context("Failed to set executable permissions")?;
    }

    Ok(binary_path)
}

/// Extract a tar.gz archive
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(dest_dir)
        .output()
        .context("Failed to execute tar command")?;

    if !output.status.success() {
        bail!(
            "tar extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Extract a zip archive
fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let output = Command::new("unzip")
        .arg("-o")
        .arg(archive_path)
        .arg("-d")
        .arg(dest_dir)
        .output()
        .context("Failed to execute unzip command")?;

    if !output.status.success() {
        bail!(
            "unzip extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

/// Get the path to the current executable
fn get_current_exe() -> Result<PathBuf> {
    env::current_exe().context("Failed to get current executable path")
}

/// Replace the current binary with the new one
fn replace_binary(new_binary: &Path, current_exe: &Path) -> Result<()> {
    // On Windows, we can't replace a running executable directly
    // We need to rename the old one and place the new one
    #[cfg(windows)]
    {
        let old_path = current_exe.with_extension("exe.old");
        fs::rename(current_exe, &old_path).context("Failed to rename old binary")?;
        fs::copy(new_binary, current_exe).context("Failed to copy new binary")?;
        // Try to remove the old binary (may fail if still locked)
        let _ = fs::remove_file(&old_path);
    }

    // On Unix, we can overwrite the running executable
    #[cfg(unix)]
    {
        // Use rename for atomic replacement on Unix
        fs::rename(new_binary, current_exe).context("Failed to replace binary")?;
    }

    Ok(())
}

fn is_permission_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io_err| io_err.kind() == io::ErrorKind::PermissionDenied)
    })
}

fn print_update_recovery_hint(current_exe: &Path) {
    println!();
    println!("TopClaw downloaded the new release, but it could not replace the current binary.");
    println!("This usually means the install location requires elevated permissions.");
    println!();
    println!("Current binary location:");
    println!("  {}", current_exe.display());
    println!();
    println!("Recommended recovery paths:");
    if cfg!(target_os = "linux") {
        println!("  1. Re-run the official release installer:");
        println!("     curl -fsSL https://raw.githubusercontent.com/{GITHUB_REPO}/main/scripts/install-release.sh | bash");
        println!("  2. Or run `topclaw upgrade` again from a user-writable install location.");
    } else if cfg!(target_os = "macos") {
        println!("  1. If installed from a repo checkout, run:");
        println!("     ./bootstrap.sh --prefer-prebuilt");
        println!("  2. If installed with source builds, run:");
        println!("     cargo install --path . --force --locked");
        println!(
            "  3. If installed by another package manager, update through that package manager."
        );
    } else if cfg!(windows) {
        println!("  1. Close any running TopClaw processes.");
        println!(
            "  2. Re-run the installer or replace the binary from an elevated shell if needed."
        );
    } else {
        println!("  Reinstall TopClaw using the method you originally used to install it.");
    }
}

/// Check if an update is available
pub async fn check_for_update() -> Result<Option<String>> {
    let release = fetch_latest_release().await?;
    let latest_version = release.tag_name.trim_start_matches('v');

    if latest_version == current_version() {
        Ok(None)
    } else {
        Ok(Some(format!(
            "{} (current: {})",
            release.tag_name,
            current_version()
        )))
    }
}

/// Perform the self-upgrade from the latest GitHub release.
///
/// Returns `true` when the binary was replaced successfully and `false` when
/// the command was a no-op (already current or check-only).
pub async fn self_update(force: bool, check_only: bool) -> Result<bool> {
    println!("🦀 TopClaw Upgrade");
    println!();

    let current_exe = get_current_exe()?;
    println!("Current binary: {}", current_exe.display());
    println!("Current version: v{}", current_version());
    println!();

    // Fetch latest release info
    let release = fetch_latest_release().await?;
    let latest_version = release.tag_name.trim_start_matches('v');

    println!("Latest version:  {}", release.tag_name);

    // Check if update is needed
    if latest_version == current_version() && !force {
        println!();
        println!("✅ Already up to date!");
        return Ok(false);
    }

    if check_only {
        println!();
        println!(
            "Update available: {} -> {}",
            current_version(),
            latest_version
        );
        println!("Run `topclaw upgrade` to install the update.");
        return Ok(false);
    }

    println!();
    println!(
        "Updating from v{} to {}...",
        current_version(),
        latest_version
    );

    // Find the appropriate asset
    let asset = find_asset_for_platform(&release)?;
    let checksums_asset = find_checksums_asset(&release)?;
    let checksums = download_text_asset(checksums_asset).await?;
    let expected_sha256 = parse_expected_sha256(&checksums, &asset.name)?;
    println!("Downloading: {}", asset.name);

    // Create temp directory
    let temp_dir = tempfile::tempdir().context("Failed to create temp directory")?;

    // Download and extract
    let new_binary = download_binary(asset, &expected_sha256, temp_dir.path()).await?;

    println!("Installing upgrade...");

    // Replace the binary
    if let Err(err) = replace_binary(&new_binary, &current_exe) {
        if is_permission_error(&err) {
            print_update_recovery_hint(&current_exe);
        }
        return Err(err).context("Failed to install the downloaded update");
    }

    println!();
    println!("✅ Successfully upgraded to {}!", release.tag_name);
    println!();
    println!("Restart TopClaw to use the new version.");

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expected_sha256_finds_asset_digest() {
        let checksums = "\
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  topclaw-x86_64-unknown-linux-gnu.tar.gz
fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210  SHA256SUMS
";
        let digest = parse_expected_sha256(checksums, "topclaw-x86_64-unknown-linux-gnu.tar.gz")
            .expect("digest should parse");
        assert_eq!(
            digest,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn verify_archive_sha256_rejects_mismatch() {
        let err = verify_archive_sha256(b"topclaw", &"0".repeat(64), "topclaw.tar.gz")
            .expect_err("mismatch should fail");
        assert!(err.to_string().contains("Checksum verification failed"));
    }

    #[test]
    fn find_checksums_asset_requires_release_checksums() {
        let release = Release {
            tag_name: "v0.1.2".to_string(),
            assets: vec![Asset {
                name: "topclaw-x86_64-unknown-linux-gnu.tar.gz".to_string(),
                browser_download_url: "https://example.com/archive".to_string(),
            }],
        };
        let err = find_checksums_asset(&release).expect_err("missing checksums should fail");
        assert!(err.to_string().contains(RELEASE_CHECKSUMS_ASSET));
    }
}
