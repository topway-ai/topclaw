//! Linux desktop helper bootstrap for computer_use.
//!
//! Handles detection, probing, and installation of xdotool, wmctrl, scrot.
//! This logic is separated from the main computer_use tool to keep concerns clear:
//! bootstrap is a Linux-specific system provisioning task, while computer_use
//! is the actual desktop automation tool.

use crate::tools::traits::{tool_fail, ToolResult};
use std::io::IsTerminal;
use std::process::Stdio;
use tracing::info;

/// Linux desktop helpers the sidecar shells out to.
pub const LINUX_HELPERS: &[&str] = &[
    "xdotool", // window/workspace control, mouse/keyboard simulation
    "wmctrl",  // window list, focus, close
    "scrot",   // screenshot capture
               // NOTE: xdg-open is not probed here because:
               // 1. The sidecar (linux.rs) uses xdg-open via std::process::Command::new("xdg-open")
               // 2. xdg-open ships in xdg-utils package, but xdg-open binary is usually available
               //    on most desktop Linux systems by default
               // 3. app_launch with URLs doesn't require xdg-open to be pre-probed - the tool
               //    will return an error at runtime if xdg-open fails, which is informative
               //    rather than silently succeeding
];

/// Probe result for Linux desktop helper readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopHelperProbe {
    /// All helpers that were checked.
    pub checked_helpers: Vec<&'static str>,
    /// Helpers that are missing from PATH.
    pub missing_helpers: Vec<&'static str>,
    /// Detected package manager (e.g. apt-get, dnf, pacman, zypper, apk).
    pub package_manager: Option<&'static str>,
    /// Packages needed to install missing helpers.
    pub packages_to_install: Vec<&'static str>,
    /// Full sudo install command for display to user.
    pub install_command: Option<String>,
}

/// Detect which Linux desktop helpers are missing from PATH.
pub fn missing_helpers() -> Vec<&'static str> {
    #[cfg(target_os = "linux")]
    {
        LINUX_HELPERS
            .iter()
            .copied()
            .filter(|bin| which::which(bin).is_err())
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Probe desktop helper readiness: check which helpers are missing and
/// what install command would be needed.
pub fn probe_desktop_helpers() -> DesktopHelperProbe {
    #[cfg(target_os = "linux")]
    {
        let missing = missing_helpers();
        let package_manager = detect_package_manager().map(|(m, _)| m);
        let packages = package_manager
            .map(|m| packages_for_missing(&missing, m))
            .unwrap_or_default();
        let install_cmd =
            package_manager.map(|m| format!("sudo {}", install_command_string(m, &packages)));

        DesktopHelperProbe {
            checked_helpers: LINUX_HELPERS.to_vec(),
            missing_helpers: missing,
            package_manager,
            packages_to_install: packages,
            install_command: install_cmd,
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        DesktopHelperProbe {
            checked_helpers: Vec::new(),
            missing_helpers: Vec::new(),
            package_manager: None,
            packages_to_install: Vec::new(),
            install_command: None,
        }
    }
}

/// Install missing Linux desktop helpers using a daemon-safe, non-interactive
/// path. Non-Linux platforms report "no-op".
pub async fn install_desktop_helpers() -> String {
    summarize_bootstrap_result(run_bootstrap_impl_with_mode(false).await)
}

/// Install missing Linux desktop helpers for an explicit user-driven setup
/// flow. When a real terminal is available, this may prompt for the user's
/// sudo password instead of requiring `sudo -n`.
pub async fn install_desktop_helpers_for_user_request() -> String {
    summarize_bootstrap_result(run_bootstrap_impl_with_mode(true).await)
}

/// Internal bootstrap implementation returning a ToolResult.
pub async fn run_bootstrap() -> ToolResult {
    run_bootstrap_impl_with_mode(false).await
}

fn summarize_bootstrap_result(result: ToolResult) -> String {
    if result.success {
        result.output
    } else {
        format!("Bootstrap failed: {}", result.error.unwrap_or_default())
    }
}

#[cfg(target_os = "linux")]
async fn run_bootstrap_impl_with_mode(allow_interactive_sudo: bool) -> ToolResult {
    let probe = probe_desktop_helpers();
    if probe.missing_helpers.is_empty() {
        return ToolResult {
            success: true,
            output: "All Linux desktop helpers (xdotool, wmctrl, scrot) are already installed."
                .into(),
            error: None,
        };
    }

    let manager = match probe.package_manager {
        Some(m) => m,
        None => {
            return fail(&format!(
                "missing helpers ({}) but no supported package manager (apt-get, dnf, pacman, zypper, apk) was found. Install manually: xdotool wmctrl scrot",
                probe.missing_helpers.join(", ")
            ));
        }
    };

    let pkgs = probe.packages_to_install;

    // Sanity-check sudo is available.
    if which::which("sudo").is_err() {
        return fail("'sudo' not found. Install manually as root: xdotool wmctrl scrot");
    }

    // Check if we can run sudo non-interactively.
    let interactive_sudo = allow_interactive_sudo
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();

    if !interactive_sudo && !sudo_noninteractive_ok().await {
        return fail(&format!(
            "sudo requires a password (sudo -n failed). Either configure passwordless sudo, or run manually: sudo {}",
            install_command_string(manager, &pkgs)
        ));
    }

    let mut argv = Vec::new();
    if !interactive_sudo {
        argv.push("-n".to_string());
    }
    argv.extend(install_argv(manager, &pkgs));

    if interactive_sudo {
        match tokio::process::Command::new("sudo")
            .args(&argv)
            .status()
            .await
        {
            Ok(status) if status.success() => bootstrap_install_success(manager, &pkgs),
            Ok(status) => fail(&format!(
                "{manager} install failed (exit {:?}).",
                status.code()
            )),
            Err(e) => fail(&format!("failed to run sudo {manager}: {e}")),
        }
    } else {
        let out = tokio::process::Command::new("sudo")
            .args(&argv)
            .stdin(Stdio::null())
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => bootstrap_install_success(manager, &pkgs),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                fail(&format!(
                    "{manager} install failed (exit {:?}): {}",
                    o.status.code(),
                    stderr.trim()
                ))
            }
            Err(e) => fail(&format!("failed to run sudo {manager}: {e}")),
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn run_bootstrap_impl_with_mode(_allow_interactive_sudo: bool) -> ToolResult {
    ToolResult {
        success: true,
        output: format!(
            "bootstrap is a no-op on {}: desktop helpers are not required.",
            std::env::consts::OS
        ),
        error: None,
    }
}

#[cfg(target_os = "linux")]
fn bootstrap_install_success(manager: &str, pkgs: &[&str]) -> ToolResult {
    let still_missing = missing_helpers();
    if still_missing.is_empty() {
        info!(
            target: "topclaw::audit",
            event = "computer_use_bootstrap",
            manager = manager,
            packages = ?pkgs,
            "installed Linux desktop helpers"
        );
        ToolResult {
            success: true,
            output: format!(
                "Installed {} via {manager}. Desktop helpers ready.",
                pkgs.join(" ")
            ),
            error: None,
        }
    } else {
        fail(&format!(
            "{manager} reported success but {} still missing. Try reinstalling manually.",
            still_missing.join(", ")
        ))
    }
}

/// Detect which package manager is available.
/// Returns (manager-binary, full-package-list).
#[cfg(target_os = "linux")]
fn detect_package_manager() -> Option<(&'static str, Vec<&'static str>)> {
    let candidates = [
        ("apt-get", vec!["xdotool", "wmctrl", "scrot"]),
        ("dnf", vec!["xdotool", "wmctrl", "scrot"]),
        ("pacman", vec!["xdotool", "wmctrl", "scrot"]),
        ("zypper", vec!["xdotool", "wmctrl", "scrot"]),
        ("apk", vec!["xdotool", "wmctrl", "scrot"]),
    ];
    for (bin, pkgs) in candidates {
        if which::which(bin).is_ok() {
            return Some((bin, pkgs));
        }
    }
    None
}

/// Map helper binary names to distro package names.
#[cfg(target_os = "linux")]
fn packages_for_missing(missing: &[&str], _manager: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for m in missing {
        let pkg = match *m {
            "xdotool" => "xdotool",
            "wmctrl" => "wmctrl",
            "scrot" => "scrot",
            _ => continue,
        };
        if !out.contains(&pkg) {
            out.push(pkg);
        }
    }
    out
}

/// Build install argv for a package manager.
#[cfg(target_os = "linux")]
pub fn install_argv(manager: &str, pkgs: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = match manager {
        "apt-get" => vec!["apt-get".into(), "install".into(), "-y".into()],
        "dnf" => vec!["dnf".into(), "install".into(), "-y".into()],
        "pacman" => vec!["pacman".into(), "-S".into(), "--noconfirm".into()],
        "zypper" => vec!["zypper".into(), "install".into(), "-y".into()],
        "apk" => vec!["apk".into(), "add".into()],
        _ => vec![manager.into(), "install".into()],
    };
    for p in pkgs {
        argv.push((*p).into());
    }
    argv
}

/// Human-readable install command string.
#[cfg(target_os = "linux")]
fn install_command_string(manager: &str, pkgs: &[&str]) -> String {
    install_argv(manager, pkgs).join(" ")
}

fn fail(msg: &str) -> ToolResult {
    tool_fail(msg)
}

/// Check if sudo can run non-interactively (NOPASSWD or cached credential).
#[cfg(target_os = "linux")]
async fn sudo_noninteractive_ok() -> bool {
    match tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_helper_probe_is_structured() {
        let probe = probe_desktop_helpers();
        #[cfg(target_os = "linux")]
        {
            assert_eq!(probe.checked_helpers, LINUX_HELPERS.to_vec());
            assert!(probe
                .missing_helpers
                .iter()
                .all(|h| probe.checked_helpers.contains(h)));
            if probe.package_manager.is_some() && !probe.missing_helpers.is_empty() {
                assert!(!probe.packages_to_install.is_empty());
                assert!(probe.install_command.is_some());
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(probe.checked_helpers.is_empty());
            assert!(probe.missing_helpers.is_empty());
            assert!(probe.package_manager.is_none());
            assert!(probe.packages_to_install.is_empty());
            assert!(probe.install_command.is_none());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn install_argv_shapes_by_manager() {
        let s = |v: Vec<&str>| -> Vec<String> { v.into_iter().map(String::from).collect() };
        assert_eq!(
            install_argv("apt-get", &["xdotool"]),
            s(vec!["apt-get", "install", "-y", "xdotool"])
        );
        assert_eq!(
            install_argv("pacman", &["xdotool", "wmctrl"]),
            s(vec!["pacman", "-S", "--noconfirm", "xdotool", "wmctrl"])
        );
        assert_eq!(
            install_argv("dnf", &["scrot"]),
            s(vec!["dnf", "install", "-y", "scrot"])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn packages_for_missing_maps_correctly() {
        let pkgs = packages_for_missing(&["xdotool", "wmctrl"], "apt-get");
        assert_eq!(pkgs, vec!["xdotool", "wmctrl"]);
    }

    #[tokio::test]
    async fn bootstrap_reports_already_ready_when_nothing_missing() {
        #[cfg(target_os = "linux")]
        {
            if !missing_helpers().is_empty() {
                return; // Skip on hosts missing helpers
            }
            let r = run_bootstrap().await;
            assert!(r.success);
            assert!(r.output.contains("already installed"));
        }
        #[cfg(not(target_os = "linux"))]
        {
            let r = run_bootstrap().await;
            assert!(r.success);
            assert!(r.output.contains("no-op"));
        }
    }

    #[tokio::test]
    async fn install_desktop_helpers_returns_string() {
        #[cfg(target_os = "linux")]
        {
            if !missing_helpers().is_empty() {
                return; // Skip on hosts missing helpers
            }
            let result = install_desktop_helpers().await;
            assert!(!result.is_empty());
        }
        #[cfg(not(target_os = "linux"))]
        {
            let result = install_desktop_helpers().await;
            assert!(!result.is_empty());
            assert!(result.contains("no-op"));
        }
    }

    #[test]
    fn missing_helpers_returns_vec() {
        let missing = missing_helpers();
        for bin in &missing {
            assert!(!bin.is_empty());
        }
    }
}
