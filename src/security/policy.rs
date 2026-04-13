use crate::security::{DomainMatcher, OtpValidator, SecretStore};
use anyhow::Context;
use parking_lot::Mutex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// How much autonomy the agent has
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// Read-only: can observe but not act
    ReadOnly,
    /// Supervised: acts but requires approval for risky operations
    #[default]
    Supervised,
    /// Full: autonomous execution within policy bounds
    Full,
}

impl std::str::FromStr for AutonomyLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "read_only" | "readonly" => Ok(Self::ReadOnly),
            "supervised" => Ok(Self::Supervised),
            "full" => Ok(Self::Full),
            _ => Err(format!(
                "invalid autonomy level '{s}': expected read_only, supervised, or full"
            )),
        }
    }
}

/// Policy for handling shell redirect operators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellRedirectPolicy {
    /// Block redirect operators (`<`, `>`, `>>`, etc.) in unquoted shell input.
    #[default]
    Block,
    /// Strip common LLM-generated stderr/null redirects before validation/execution.
    ///
    /// Supported normalization:
    /// - `2>&1`, `1>&2`, `>&1`
    /// - `2>/dev/null`, `2>>/dev/null`, `>/dev/null`, `>>/dev/null`
    /// - `&>/dev/null`, `>&/dev/null`
    /// - `|&` -> `|`
    ///
    /// Other redirect forms remain blocked by command policy.
    Strip,
    /// Allow redirect operators and normalize supported heredoc patterns that
    /// LLMs commonly emit for local scripting.
    ///
    /// Path policy and command allowlist checks still apply after normalization.
    Allow,
}

/// Risk score for shell command execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRiskLevel {
    Low,
    Medium,
    High,
}

/// Classifies whether a tool operation is read-only or side-effecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOperation {
    Read,
    Act,
}

/// Sliding-window action tracker for rate limiting.
#[derive(Debug)]
pub struct ActionTracker {
    /// Timestamps of recent actions (kept within the last hour).
    actions: Mutex<Vec<Instant>>,
}

impl ActionTracker {
    pub fn new() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
        }
    }

    fn sliding_window_cutoff() -> Instant {
        Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now)
    }

    /// Record an action (unconditionally). Use `record_if_allowed` to check
    /// the budget first.
    pub fn record(&self) -> usize {
        let mut actions = self.actions.lock();
        actions.retain(|t| *t > Self::sliding_window_cutoff());
        actions.push(Instant::now());
        actions.len()
    }

    /// Count of actions in the current window without recording.
    pub fn count(&self) -> usize {
        let mut actions = self.actions.lock();
        actions.retain(|t| *t > Self::sliding_window_cutoff());
        actions.len()
    }
}

impl Clone for ActionTracker {
    fn clone(&self) -> Self {
        let actions = self.actions.lock();
        Self {
            actions: Mutex::new(actions.clone()),
        }
    }
}

/// Security policy enforced on all tool executions
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub autonomy: AutonomyLevel,
    pub workspace_dir: PathBuf,
    pub workspace_only: bool,
    pub allowed_commands: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allowed_roots: Vec<PathBuf>,
    pub max_actions_per_hour: u32,
    pub max_cost_per_day_cents: u32,
    pub require_approval_for_medium_risk: bool,
    pub block_high_risk_commands: bool,
    pub shell_redirect_policy: ShellRedirectPolicy,
    pub shell_env_passthrough: Vec<String>,
    pub otp_gated_actions: HashSet<String>,
    pub otp_gated_domains: DomainMatcher,
    pub otp_validator: Option<Arc<OtpValidator>>,
    pub tracker: ActionTracker,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: PathBuf::from("."),
            workspace_only: true,
            allowed_commands: vec![],
            forbidden_paths: default_forbidden_paths(),
            allowed_roots: Vec::new(),
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            shell_redirect_policy: ShellRedirectPolicy::Block,
            shell_env_passthrough: vec![],
            otp_gated_actions: HashSet::new(),
            otp_gated_domains: DomainMatcher::default(),
            otp_validator: None,
            tracker: ActionTracker::new(),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn default_forbidden_paths() -> Vec<String> {
    [
        "/etc",
        "/root",
        "/home",
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/opt",
        "/boot",
        "/dev",
        "/proc",
        "/sys",
        "/var",
        "/tmp",
        "~/.ssh",
        "~/.gnupg",
        "~/.aws",
        "~/.config/secrets",
    ]
    .into_iter()
    .map(std::string::ToString::to_string)
    .collect()
}

fn protected_workspace_files() -> &'static [&'static str] {
    &[
        "AGENTS.md",
        "SOUL.md",
        "TOOLS.md",
        "IDENTITY.md",
        "USER.md",
        "HEARTBEAT.md",
        "BOOTSTRAP.md",
    ]
}

fn expand_user_path(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }

    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(path)
}

fn normalize_workspace_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::CurDir) {
            continue;
        }
        normalized.push(component.as_os_str());
    }
    normalized
}

// ── Shell Command Parsing Utilities ───────────────────────────────────────
// These helpers implement a minimal quote-aware shell lexer. They exist
// because security validation must reason about the *structure* of a
// command (separators, operators, quoting) rather than treating it as a
// flat string — otherwise an attacker could hide dangerous sub-commands
// inside quoted arguments or chained operators.
/// Skip leading environment variable assignments (e.g. `FOO=bar cmd args`).
/// Returns the remainder starting at the first non-assignment word.
fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s;
    loop {
        let Some(word) = rest.split_whitespace().next() else {
            return rest;
        };
        // Environment assignment: contains '=' and starts with a letter or underscore
        if word.contains('=')
            && word
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            // Advance past this word
            rest = rest[word.len()..].trim_start();
        } else {
            return rest;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteState {
    None,
    Single,
    Double,
}

// ── Quote-Aware Character Iterator ───────────────────────────────────────
// Shared state machine for all quote-aware shell parsing. Yields each
// character together with whether it sits outside any quotes.

struct QuoteAwareChars<I: Iterator<Item = char>> {
    inner: std::iter::Peekable<I>,
    quote: QuoteState,
    escaped: bool,
}

impl<I: Iterator<Item = char>> QuoteAwareChars<I> {
    fn new(chars: I) -> Self {
        Self {
            inner: chars.peekable(),
            quote: QuoteState::None,
            escaped: false,
        }
    }

    /// Consume the next raw character if it equals `expected`.
    fn next_if_eq(&mut self, expected: char) -> Option<char> {
        self.inner.next_if_eq(&expected)
    }
}

/// Yielded item: the character, whether it's unquoted, and the new quote state.
struct QuoteAwareChar {
    ch: char,
    unquoted: bool,
}

impl<I: Iterator<Item = char>> Iterator for QuoteAwareChars<I> {
    type Item = QuoteAwareChar;

    fn next(&mut self) -> Option<Self::Item> {
        let ch = self.inner.next()?;

        match self.quote {
            QuoteState::Single => {
                if ch == '\'' {
                    self.quote = QuoteState::None;
                }
                Some(QuoteAwareChar {
                    ch,
                    unquoted: false,
                })
            }
            QuoteState::Double => {
                if self.escaped {
                    self.escaped = false;
                    return Some(QuoteAwareChar {
                        ch,
                        unquoted: false,
                    });
                }
                if ch == '\\' {
                    self.escaped = true;
                    return Some(QuoteAwareChar {
                        ch,
                        unquoted: false,
                    });
                }
                if ch == '"' {
                    self.quote = QuoteState::None;
                }
                Some(QuoteAwareChar {
                    ch,
                    unquoted: false,
                })
            }
            QuoteState::None => {
                if self.escaped {
                    self.escaped = false;
                    return Some(QuoteAwareChar {
                        ch,
                        unquoted: false,
                    });
                }
                if ch == '\\' {
                    self.escaped = true;
                    return Some(QuoteAwareChar {
                        ch,
                        unquoted: false,
                    });
                }
                match ch {
                    '\'' => {
                        self.quote = QuoteState::Single;
                        Some(QuoteAwareChar {
                            ch,
                            unquoted: false,
                        })
                    }
                    '"' => {
                        self.quote = QuoteState::Double;
                        Some(QuoteAwareChar {
                            ch,
                            unquoted: false,
                        })
                    }
                    _ => Some(QuoteAwareChar { ch, unquoted: true }),
                }
            }
        }
    }
}

/// Split a shell command into sub-commands by unquoted separators.
///
/// Separators:
/// - `;` and newline
/// - `|`
/// - `&&`, `||`
///
/// Characters inside single or double quotes are treated as literals, so
/// `sqlite3 db "SELECT 1; SELECT 2;"` remains a single segment.
fn split_unquoted_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut iter = QuoteAwareChars::new(command.chars());

    let push_segment = |segments: &mut Vec<String>, current: &mut String| {
        let trimmed = current.trim();
        if !trimmed.is_empty() {
            segments.push(trimmed.to_string());
        }
        current.clear();
    };

    while let Some(item) = iter.next() {
        if !item.unquoted {
            current.push(item.ch);
            continue;
        }
        match item.ch {
            ';' | '\n' => push_segment(&mut segments, &mut current),
            '|' => {
                // Consume full `||`; both characters are separators.
                let _ = iter.next_if_eq('|');
                push_segment(&mut segments, &mut current);
            }
            '&' => {
                if iter.next_if_eq('&').is_some() {
                    // `&&` is a separator; single `&` is handled separately.
                    push_segment(&mut segments, &mut current);
                } else {
                    current.push(item.ch);
                }
            }
            _ => current.push(item.ch),
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }

    segments
}

/// Detect a single unquoted `&` operator (background/chain). `&&` is allowed.
///
/// We treat any standalone `&` as unsafe in policy validation because it can
/// chain hidden sub-commands and escape foreground timeout expectations.
fn contains_unquoted_single_ampersand(command: &str) -> bool {
    // Track the last unquoted character so we can recognize `>&` / `<&` stream
    // merge redirects (e.g. `2>&1`, `>&2`). POSIX shells treat those as part of
    // the redirect, not as a background operator, so they must not be flagged.
    let mut prev_unquoted: Option<char> = None;
    let mut iter = QuoteAwareChars::new(command.chars());
    while let Some(item) = iter.next() {
        if !item.unquoted {
            continue;
        }
        if item.ch == '&' {
            // `&&` is the logical-AND separator, consumed as a pair.
            if iter.next_if_eq('&').is_some() {
                prev_unquoted = Some('&');
                continue;
            }
            // `>&` / `<&` is a stream-merge redirect, not background execution.
            if matches!(prev_unquoted, Some('>' | '<')) {
                prev_unquoted = Some('&');
                continue;
            }
            return true;
        }
        prev_unquoted = Some(item.ch);
    }
    false
}

/// Detect an unquoted character in a shell command.
fn contains_unquoted_char(command: &str, target: char) -> bool {
    QuoteAwareChars::new(command.chars()).any(|item| item.unquoted && item.ch == target)
}

fn is_token_boundary_char(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, ';' | '\n' | '|' | '&' | ')' | '(')
}

fn has_token_boundary_before(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    chars
        .get(index - 1)
        .is_some_and(|ch| is_token_boundary_char(*ch))
}

fn starts_with_literal(chars: &[char], start: usize, literal: &str) -> bool {
    let literal_chars: Vec<char> = literal.chars().collect();
    chars
        .get(start..start + literal_chars.len())
        .is_some_and(|slice| slice == literal_chars)
}

fn consume_stream_merge_redirect(chars: &[char], start: usize) -> Option<usize> {
    // Matches:
    // - 2>&1
    // - 1>&2
    // - >&1
    // `n>&m` should not consume trailing digits from command words
    // (e.g. `python3>&1` should keep `python3`).
    if chars[start].is_ascii_digit() && !has_token_boundary_before(chars, start) {
        return None;
    }

    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '>' {
        return None;
    }
    i += 1;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '&' {
        return None;
    }
    i += 1;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    let fd_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == fd_start {
        return None;
    }
    Some(i - start)
}

fn consume_dev_null_redirect(chars: &[char], start: usize) -> Option<usize> {
    // Matches:
    // - [fd]>/dev/null
    // - [fd]>>/dev/null
    // - [fd]< /dev/null
    // - &>/dev/null
    // - >&/dev/null
    let mut i = start;
    if chars[i] == '&' {
        i += 1;
        if i >= chars.len() || chars[i] != '>' {
            return None;
        }
        i += 1;
    } else {
        if chars[i].is_ascii_digit() && !has_token_boundary_before(chars, start) {
            return None;
        }
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i >= chars.len() || !matches!(chars[i], '>' | '<') {
            return None;
        }
        let op = chars[i];
        i += 1;
        if op == '>' && i < chars.len() && chars[i] == '>' {
            i += 1;
        }
        if op == '>' && i < chars.len() && chars[i] == '&' {
            i += 1;
        }
    }

    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if !starts_with_literal(chars, i, "/dev/null") {
        return None;
    }
    i += "/dev/null".chars().count();
    if i < chars.len() && !is_token_boundary_char(chars[i]) {
        return None;
    }
    Some(i - start)
}

fn strip_supported_redirects(command: &str) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut out = String::with_capacity(command.len());
    let mut quote = QuoteState::None;
    let mut escaped = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                out.push(ch);
                i += 1;
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                    out.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    out.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::None;
                }
                out.push(ch);
                i += 1;
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    out.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    out.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '\'' {
                    quote = QuoteState::Single;
                    out.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '"' {
                    quote = QuoteState::Double;
                    out.push(ch);
                    i += 1;
                    continue;
                }

                // Normalize `|&` to `|` since shell tool already captures stderr.
                if ch == '|' && chars.get(i + 1).is_some_and(|next| *next == '&') {
                    out.push('|');
                    i += 2;
                    continue;
                }

                if let Some(consumed) = consume_stream_merge_redirect(&chars, i)
                    .or_else(|| consume_dev_null_redirect(&chars, i))
                {
                    i += consumed;
                    continue;
                }

                out.push(ch);
                i += 1;
            }
        }
    }

    out
}

fn shell_single_quote(raw: &str) -> String {
    let mut out = String::from("'");
    for ch in raw.chars() {
        if ch == '\'' {
            out.push_str("'\"'\"'");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn normalize_supported_heredoc_command(command: &str) -> String {
    let trimmed = command.trim();
    let Some(first_newline) = trimmed.find('\n') else {
        return command.to_string();
    };

    let header = trimmed[..first_newline].trim_end();
    let Some(heredoc_idx) = header.find("<<") else {
        return command.to_string();
    };

    let prefix = header[..heredoc_idx].trim_end();
    let delimiter = strip_wrapping_quotes(header[heredoc_idx + 2..].trim());
    if delimiter.is_empty() {
        return command.to_string();
    }

    let executable = prefix
        .split_whitespace()
        .next()
        .map(strip_wrapping_quotes)
        .and_then(|token| token.rsplit('/').next())
        .unwrap_or("");
    if !matches!(executable, "python" | "python3") {
        return command.to_string();
    }

    let mut prefix_tokens: Vec<&str> = prefix.split_whitespace().collect();
    if prefix_tokens.last().copied() == Some("-") {
        prefix_tokens.pop();
    }
    if prefix_tokens.is_empty() {
        return command.to_string();
    }

    let rest = &trimmed[first_newline + 1..];
    let lines: Vec<&str> = rest.lines().collect();
    let Some(last_nonempty_idx) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return command.to_string();
    };
    if lines[last_nonempty_idx].trim_end() != delimiter {
        return command.to_string();
    }

    let body = lines[..last_nonempty_idx].join("\n");
    format!(
        "{} -c {}",
        prefix_tokens.join(" "),
        shell_single_quote(&body)
    )
}

/// Detect unquoted shell variable expansions like `$HOME`, `$1`, `$?`.
///
/// Escaped dollars (`\$`) are ignored. Variables inside single quotes are
/// treated as literals and therefore ignored. Variables inside double quotes
/// ARE detected because shell expands them there.
///
/// Note: this function does NOT use `QuoteAwareChars` because it needs to
/// detect `$` inside double quotes (where the iterator marks `unquoted=false`).
fn contains_unquoted_shell_variable_expansion(command: &str) -> bool {
    let chars: Vec<char> = command.chars().collect();
    let mut quote = QuoteState::None;
    let mut escaped = false;

    for (i, &ch) in chars.iter().enumerate() {
        match quote {
            QuoteState::Single => {
                if ch == '\'' {
                    quote = QuoteState::None;
                }
                continue;
            }
            QuoteState::Double => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                    continue;
                } else if ch == '"' {
                    quote = QuoteState::None;
                    continue;
                }
            }
            QuoteState::None => {
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    continue;
                }
                match ch {
                    '\'' => {
                        quote = QuoteState::Single;
                        continue;
                    }
                    '"' => {
                        quote = QuoteState::Double;
                        continue;
                    }
                    _ => {}
                }
            }
        }

        if ch != '$' {
            continue;
        }

        if let Some(&next) = chars.get(i + 1) {
            if next.is_ascii_alphanumeric()
                || matches!(
                    next,
                    '_' | '{' | '(' | '#' | '?' | '!' | '$' | '*' | '@' | '-'
                )
            {
                return true;
            }
        }
    }

    false
}

fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

fn looks_like_path(candidate: &str) -> bool {
    candidate.starts_with('/')
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.starts_with('~')
        || candidate == "."
        || candidate == ".."
        || candidate.contains('/')
}

fn attached_short_option_value(token: &str) -> Option<&str> {
    // Examples:
    // -f/etc/passwd   -> /etc/passwd
    // -C../outside    -> ../outside
    // -I./include     -> ./include
    let body = token.strip_prefix('-')?;
    if body.starts_with('-') || body.len() < 2 {
        return None;
    }
    let value = body[1..].trim_start_matches('=').trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn redirection_target(token: &str) -> Option<&str> {
    let marker_idx = token.find(['<', '>'])?;
    let mut rest = &token[marker_idx + 1..];
    rest = rest.trim_start_matches(['<', '>']);
    rest = rest.trim_start_matches('&');
    rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn is_allowlist_entry_match(allowed: &str, executable: &str, executable_base: &str) -> bool {
    let allowed = strip_wrapping_quotes(allowed).trim();
    if allowed.is_empty() {
        return false;
    }

    // Explicit wildcard support for "allow any command name/path".
    if allowed == "*" {
        return true;
    }

    // Path-like allowlist entries must match the executable token exactly
    // after "~" expansion.
    if looks_like_path(allowed) {
        let allowed_path = expand_user_path(allowed);
        let executable_path = expand_user_path(executable);
        return executable_path == allowed_path;
    }

    // Command-name entries continue to match by basename.
    allowed == executable_base
}

impl SecurityPolicy {
    /// Apply configured redirect policy to a shell command before validation/execution.
    pub fn apply_shell_redirect_policy(&self, command: &str) -> String {
        match self.shell_redirect_policy {
            ShellRedirectPolicy::Block => command.to_string(),
            ShellRedirectPolicy::Strip => strip_supported_redirects(command),
            ShellRedirectPolicy::Allow => normalize_supported_heredoc_command(command),
        }
    }

    // ── Risk Classification ──────────────────────────────────────────────
    // Risk is assessed per-segment (split on shell operators), and the
    // highest risk across all segments wins. This prevents bypasses like
    // `ls && rm -rf /` from being classified as Low just because `ls` is safe.

    /// Classify command risk. Any high-risk segment marks the whole command high.
    pub fn command_risk_level(&self, command: &str) -> CommandRiskLevel {
        self.command_risk_level_with_segments(&split_unquoted_segments(command))
    }

    fn command_risk_level_with_segments(&self, segments: &[String]) -> CommandRiskLevel {
        let mut saw_medium = false;

        for segment in segments {
            let cmd_part = skip_env_assignments(segment);
            let mut words = cmd_part.split_whitespace();
            let Some(base_raw) = words.next() else {
                continue;
            };

            let base = base_raw
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();

            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            let joined_segment = cmd_part.to_ascii_lowercase();

            // High-risk commands
            if matches!(
                base.as_str(),
                "rm" | "mkfs"
                    | "dd"
                    | "shutdown"
                    | "reboot"
                    | "halt"
                    | "poweroff"
                    | "sudo"
                    | "su"
                    | "chown"
                    | "chmod"
                    | "useradd"
                    | "userdel"
                    | "usermod"
                    | "passwd"
                    | "mount"
                    | "umount"
                    | "iptables"
                    | "ufw"
                    | "firewall-cmd"
                    | "curl"
                    | "wget"
                    | "nc"
                    | "ncat"
                    | "netcat"
                    | "scp"
                    | "ssh"
                    | "ftp"
                    | "telnet"
            ) {
                return CommandRiskLevel::High;
            }

            if joined_segment.contains("rm -rf /")
                || joined_segment.contains("rm -fr /")
                || joined_segment.contains(":(){:|:&};:")
            {
                return CommandRiskLevel::High;
            }

            // Medium-risk commands (state-changing, but not inherently destructive)
            let medium = match base.as_str() {
                "git" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "commit"
                            | "push"
                            | "reset"
                            | "clean"
                            | "rebase"
                            | "merge"
                            | "cherry-pick"
                            | "revert"
                            | "branch"
                            | "checkout"
                            | "switch"
                            | "tag"
                    )
                }),
                "npm" | "pnpm" | "yarn" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "install" | "add" | "remove" | "uninstall" | "update" | "publish"
                    )
                }),
                "cargo" => args.first().is_some_and(|verb| {
                    matches!(
                        verb.as_str(),
                        "add" | "remove" | "install" | "clean" | "publish"
                    )
                }),
                "touch" | "mkdir" | "mv" | "cp" | "ln" => true,
                _ => false,
            };

            saw_medium |= medium;
        }

        if saw_medium {
            CommandRiskLevel::Medium
        } else {
            CommandRiskLevel::Low
        }
    }

    // ── Command Execution Policy Gate ──────────────────────────────────────
    // Validation follows a strict precedence order:
    //   1. Allowlist check (is the base command permitted at all?)
    //   2. Risk classification (high / medium / low)
    //   3. Policy flags (block_high_risk_commands, require_approval_for_medium_risk)
    //   4. Autonomy level × approval status (supervised requires explicit approval)
    // This ordering ensures deny-by-default: unknown commands are rejected
    // before any risk or autonomy logic runs.

    /// Validate full command execution policy (allowlist + risk gate).
    pub fn validate_command_execution(
        &self,
        command: &str,
        approved: bool,
    ) -> Result<CommandRiskLevel, String> {
        self.validate_command_execution_with_temporary_allowlist(command, approved, &[])
    }

    /// Validate full command execution policy with an optional turn-scoped
    /// exact-command allowlist granted by a human-approved execution plan.
    pub fn validate_command_execution_with_temporary_allowlist(
        &self,
        command: &str,
        approved: bool,
        temporary_allowed_commands: &[String],
    ) -> Result<CommandRiskLevel, String> {
        let effective_command = self.apply_shell_redirect_policy(command);

        // Pre-compute segments once; shared by allowlist, path, and risk checks.
        let segments = split_unquoted_segments(&effective_command);

        if !self.is_command_allowed_with_segments_and_temporary(
            &effective_command,
            &segments,
            temporary_allowed_commands,
        ) {
            return Err(format!("Command not allowed by security policy: {command}"));
        }

        if let Some(path) = self.forbidden_path_argument_with_segments(&segments) {
            return Err(format!("Path blocked by security policy: {path}"));
        }

        let risk = self.command_risk_level_with_segments(&segments);

        if risk == CommandRiskLevel::High {
            if self.block_high_risk_commands && !approved {
                return Err("Command blocked: high-risk command is disallowed by policy".into());
            }
            if self.autonomy == AutonomyLevel::Supervised && !approved {
                return Err(
                    "Command requires explicit approval (approved=true): high-risk operation"
                        .into(),
                );
            }
        }

        if risk == CommandRiskLevel::Medium
            && self.autonomy == AutonomyLevel::Supervised
            && self.require_approval_for_medium_risk
            && !approved
        {
            return Err(
                "Command requires explicit approval (approved=true): medium-risk operation".into(),
            );
        }

        Ok(risk)
    }

    /// Validate **only** structural safety of a shell command — subshell
    /// operators, redirections, dangerous arguments, and forbidden paths.
    ///
    /// Unlike [`validate_command_execution_with_temporary_allowlist`], this
    /// intentionally **skips** the per-binary allowlist check and risk-level
    /// enforcement.  It is designed for `approval_precheck` so that commands
    /// whose binaries are not in the static allowlist still pass the
    /// precheck and reach the non-CLI approval prompt (inline buttons).
    pub fn validate_command_structure(&self, command: &str) -> Result<(), String> {
        let effective_command = self.apply_shell_redirect_policy(command);
        let segments = split_unquoted_segments(&effective_command);

        if !self.is_command_structurally_safe(&effective_command, &segments) {
            return Err(format!(
                "Command blocked by structural safety policy: {command}"
            ));
        }

        if let Some(path) = self.forbidden_path_argument_with_segments(&segments) {
            return Err(format!("Path blocked by security policy: {path}"));
        }

        Ok(())
    }

    // ── Layered Command Allowlist ──────────────────────────────────────────
    // Defence-in-depth: five independent gates run in order before the
    // per-segment allowlist check. Each gate targets a specific bypass
    // technique. If any gate rejects, the whole command is blocked.

    /// Check if a shell command is allowed.
    ///
    /// Validates the **entire** command string, not just the first word:
    /// - Blocks subshell operators (`` ` ``, `$(`) that hide arbitrary execution
    /// - Splits on command separators (`|`, `&&`, `||`, `;`, newlines) and
    ///   validates each sub-command against the allowlist
    /// - Blocks single `&` background chaining (`&&` remains supported)
    /// - Blocks shell redirections (`<`, `>`, `>>`) that can bypass path policy
    /// - Blocks dangerous arguments (e.g. `find -exec`, `git config`)
    pub fn is_command_allowed(&self, command: &str) -> bool {
        self.is_command_allowed_with_temporary_allowlist(command, &[])
    }

    pub fn is_command_allowed_with_temporary_allowlist(
        &self,
        command: &str,
        temporary_allowed_commands: &[String],
    ) -> bool {
        self.is_command_allowed_with_segments_and_temporary(
            command,
            &split_unquoted_segments(command),
            temporary_allowed_commands,
        )
    }

    pub(crate) fn command_matches_temporary_allowlist(
        &self,
        command: &str,
        temporary_allowed_commands: &[String],
    ) -> bool {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return false;
        }

        let effective_trimmed = self.apply_shell_redirect_policy(command);
        let effective_trimmed = effective_trimmed.trim();

        temporary_allowed_commands.iter().any(|allowed| {
            let allowed_trimmed = allowed.trim();
            if allowed_trimmed.is_empty() {
                return false;
            }
            if allowed_trimmed == trimmed || allowed_trimmed == effective_trimmed {
                return true;
            }

            let effective_allowed = self.apply_shell_redirect_policy(allowed);
            let effective_allowed = effective_allowed.trim();
            effective_allowed == trimmed || effective_allowed == effective_trimmed
        })
    }

    fn is_command_allowed_with_segments_and_temporary(
        &self,
        command: &str,
        segments: &[String],
        temporary_allowed_commands: &[String],
    ) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        let temporarily_approved =
            self.command_matches_temporary_allowlist(command, temporary_allowed_commands);

        // Default mode blocks subshell/expansion operators because they can
        // hide arbitrary execution (`echo $(rm -rf /)`) and bypass path checks
        // through variable indirection. Capability-first allow mode opts into
        // full shell scripting, so keep the stricter behavior for every other
        // policy and let allow mode proceed to per-command validation instead.
        if self.shell_redirect_policy != ShellRedirectPolicy::Allow
            && (command.contains('`')
                || contains_unquoted_shell_variable_expansion(command)
                || command.contains("<(")
                || command.contains(">("))
        {
            return false;
        }

        // Block shell redirections (`<`, `>`, `>>`) — they can read/write
        // arbitrary paths and bypass path checks.
        // Ignore quoted literals, e.g. `echo "a>b"` and `echo "a<b"`.
        if self.shell_redirect_policy != ShellRedirectPolicy::Allow
            && (contains_unquoted_char(command, '>') || contains_unquoted_char(command, '<'))
        {
            return false;
        }

        // Block `tee` — it can write to arbitrary files, bypassing the
        // redirect check above (e.g. `echo secret | tee /etc/crontab`)
        if command
            .split_whitespace()
            .any(|w| w == "tee" || w.ends_with("/tee"))
        {
            return false;
        }

        // Block background command chaining (`&`), which can hide extra
        // sub-commands and outlive timeout expectations. Keep `&&` allowed.
        if contains_unquoted_single_ampersand(command) {
            return false;
        }

        for segment in segments {
            // Strip leading env var assignments (e.g. FOO=bar cmd)
            let cmd_part = skip_env_assignments(segment);

            let mut words = cmd_part.split_whitespace();
            let executable = strip_wrapping_quotes(words.next().unwrap_or("")).trim();
            let base_cmd = executable.rsplit('/').next().unwrap_or("");

            if base_cmd.is_empty() {
                continue;
            }

            if !temporarily_approved
                && !self
                    .allowed_commands
                    .iter()
                    .any(|allowed| is_allowlist_entry_match(allowed, executable, base_cmd))
            {
                return false;
            }

            // Validate arguments for the command
            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            if !self.is_args_safe(base_cmd, &args) {
                return false;
            }
        }

        // At least one command must be present
        let has_cmd = segments.iter().any(|s| {
            let s = skip_env_assignments(s.trim());
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        });

        has_cmd
    }

    /// Check structural safety of a command without checking the per-binary
    /// allowlist.  Returns `true` if the command passes all structural gates
    /// (subshell operators, redirections, `tee`, background `&`, dangerous
    /// arguments) and contains at least one non-empty command segment.
    fn is_command_structurally_safe(&self, command: &str, segments: &[String]) -> bool {
        if self.autonomy == AutonomyLevel::ReadOnly {
            return false;
        }

        // Subshell/expansion operators
        if self.shell_redirect_policy != ShellRedirectPolicy::Allow
            && (command.contains('`')
                || contains_unquoted_shell_variable_expansion(command)
                || command.contains("<(")
                || command.contains(">("))
        {
            return false;
        }

        // Shell redirections
        if self.shell_redirect_policy != ShellRedirectPolicy::Allow
            && (contains_unquoted_char(command, '>') || contains_unquoted_char(command, '<'))
        {
            return false;
        }

        // tee bypass
        if command
            .split_whitespace()
            .any(|w| w == "tee" || w.ends_with("/tee"))
        {
            return false;
        }

        // Background &
        if contains_unquoted_single_ampersand(command) {
            return false;
        }

        // Dangerous arguments per segment (find -exec, git config, etc.)
        for segment in segments {
            let cmd_part = skip_env_assignments(segment);
            let mut words = cmd_part.split_whitespace();
            let executable = strip_wrapping_quotes(words.next().unwrap_or("")).trim();
            let base_cmd = executable.rsplit('/').next().unwrap_or("");
            if base_cmd.is_empty() {
                continue;
            }
            let args: Vec<String> = words.map(|w| w.to_ascii_lowercase()).collect();
            if !self.is_args_safe(base_cmd, &args) {
                return false;
            }
        }

        // At least one command present
        segments.iter().any(|s| {
            let s = skip_env_assignments(s.trim());
            s.split_whitespace().next().is_some_and(|w| !w.is_empty())
        })
    }

    /// Check for dangerous arguments that allow sub-command execution.
    fn is_args_safe(&self, base: &str, args: &[String]) -> bool {
        let base = base.to_ascii_lowercase();
        match base.as_str() {
            "find" => {
                // find -exec and find -ok allow arbitrary command execution
                !args.iter().any(|arg| arg == "-exec" || arg == "-ok")
            }
            "git" => {
                // git config, alias, and -c can be used to set dangerous options
                // (e.g. git config core.editor "rm -rf /")
                !args.iter().any(|arg| {
                    arg == "config"
                        || arg.starts_with("config.")
                        || arg == "alias"
                        || arg.starts_with("alias.")
                        || arg == "-c"
                })
            }
            _ => true,
        }
    }

    /// Check a single candidate token for forbidden path smuggling.
    ///
    /// Covers inline redirections (`cat</etc/passwd`), option assignment forms
    /// (`--file=/etc/passwd`, `-f/etc/passwd`), and direct path tokens.
    fn check_candidate_path_token(&self, candidate: &str) -> Option<String> {
        let candidate = candidate.trim();
        if candidate.is_empty() || candidate.contains("://") {
            return None;
        }

        if let Some(target) = redirection_target(candidate) {
            if let Some(blocked) = self.forbidden_path_token(target) {
                return Some(blocked);
            }
        }

        if candidate.starts_with('-') {
            if let Some((_, value)) = candidate.split_once('=') {
                if let Some(blocked) = self.forbidden_path_token(value) {
                    return Some(blocked);
                }
            }
            if let Some(value) = attached_short_option_value(candidate) {
                if let Some(blocked) = self.forbidden_path_token(value) {
                    return Some(blocked);
                }
            }
            return None;
        }

        self.forbidden_path_token(candidate)
    }

    /// Return the first path-like argument blocked by path policy.
    ///
    /// This is best-effort token parsing for shell commands and is intended
    /// as a safety gate before command execution.
    pub fn forbidden_path_argument(&self, command: &str) -> Option<String> {
        self.forbidden_path_argument_with_segments(&split_unquoted_segments(command))
    }

    fn forbidden_path_argument_with_segments(&self, segments: &[String]) -> Option<String> {
        for segment in segments {
            let cmd_part = skip_env_assignments(segment);
            let mut words = cmd_part.split_whitespace();
            let Some(executable) = words.next() else {
                continue;
            };

            // Cover inline forms like `cat</etc/passwd`.
            if let Some(target) = redirection_target(strip_wrapping_quotes(executable)) {
                if let Some(blocked) = self.forbidden_path_token(target) {
                    return Some(blocked);
                }
            }

            for token in words {
                if let Some(blocked) = self.check_candidate_path_token(strip_wrapping_quotes(token))
                {
                    return Some(blocked);
                }
            }
        }

        None
    }

    /// Return the first argv token value blocked by path policy.
    ///
    /// Unlike `forbidden_path_argument()`, this inspects structured argv tokens
    /// directly and still covers smuggling forms like `--file=/etc/passwd`,
    /// `-f/etc/passwd`, and inline redirections.
    pub fn forbidden_path_argv(&self, argv: &[String]) -> Option<String> {
        for token in argv {
            if let Some(blocked) = self.check_candidate_path_token(strip_wrapping_quotes(token)) {
                return Some(blocked);
            }
        }

        None
    }

    // ── Path Validation ────────────────────────────────────────────────
    // Layered checks: null-byte injection → component-level traversal →
    // URL-encoded traversal → tilde expansion → absolute-path block →
    // forbidden-prefix match. Each layer addresses a distinct escape
    // technique; together they enforce workspace confinement.

    /// Check if a file path is allowed (no path traversal, within workspace)
    pub fn is_path_allowed(&self, path: &str) -> bool {
        // Block null bytes (can truncate paths in C-backed syscalls)
        if path.contains('\0') {
            return false;
        }

        // Block path traversal: check for ".." as a path component
        if Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        // Block URL-encoded traversal attempts (e.g. ..%2f)
        let lower = path.to_lowercase();
        if lower.contains("..%2f") || lower.contains("%2f..") {
            return false;
        }

        // Reject "~user" forms because the shell expands them at runtime and
        // they can escape workspace policy.
        if path.starts_with('~') && path != "~" && !path.starts_with("~/") {
            return false;
        }

        // Expand "~" for consistent matching with forbidden paths and allowlists.
        let expanded_path = expand_user_path(path);

        if self.is_protected_workspace_path(&expanded_path) {
            return false;
        }

        // Block absolute paths when workspace_only is set, unless the path is
        // explicitly headed toward an allowed root. Resolved-path validation
        // still runs later, so this only preserves the documented contract for
        // allowlisted absolute paths.
        if self.workspace_only
            && expanded_path.is_absolute()
            && !self
                .allowed_roots
                .iter()
                .any(|root| expanded_path.starts_with(root))
        {
            return false;
        }

        if self
            .allowed_roots
            .iter()
            .any(|root| expanded_path.starts_with(root))
        {
            return true;
        }

        // Block forbidden paths using path-component-aware matching
        for forbidden in &self.forbidden_paths {
            let forbidden_path = expand_user_path(forbidden);
            if expanded_path.starts_with(forbidden_path) {
                return false;
            }
        }

        true
    }

    fn forbidden_path_token(&self, raw: &str) -> Option<String> {
        let candidate = strip_wrapping_quotes(raw).trim();
        if candidate.is_empty() || candidate.contains("://") {
            return None;
        }
        if looks_like_path(candidate) && !self.is_path_allowed(candidate) {
            Some(candidate.to_string())
        } else {
            None
        }
    }

    /// Validate that a resolved path is inside the workspace or an allowed root.
    /// Call this AFTER joining `workspace_dir` + relative path and canonicalizing.
    pub fn is_resolved_path_allowed(&self, resolved: &Path) -> bool {
        if self.is_protected_workspace_path(resolved) {
            return false;
        }

        // Prefer canonical workspace root so `/a/../b` style config paths don't
        // cause false positives or negatives.
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        if resolved.starts_with(&workspace_root) {
            return true;
        }

        // Check extra allowed roots (e.g. shared skills directories) before
        // forbidden checks so explicit allowlists can coexist with broad
        // default forbidden roots such as `/home` and `/tmp`.
        for root in &self.allowed_roots {
            let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
            if resolved.starts_with(&canonical) {
                return true;
            }
        }

        // For paths outside workspace/allowlist, block forbidden roots to
        // prevent symlink escapes and sensitive directory access.
        for forbidden in &self.forbidden_paths {
            let forbidden_path = expand_user_path(forbidden);
            if resolved.starts_with(&forbidden_path) {
                return false;
            }
        }

        // When workspace_only is disabled the user explicitly opted out of
        // workspace confinement after forbidden-path checks are applied.
        if !self.workspace_only {
            return true;
        }

        false
    }

    pub fn resolved_path_violation_message(&self, resolved: &Path) -> String {
        if self.is_protected_workspace_path(resolved) {
            return format!(
                "Path is protected by security policy and must be changed manually: {}",
                resolved.display()
            );
        }

        let guidance = if self.allowed_roots.is_empty() {
            "Add the directory to [autonomy].allowed_roots (for example: allowed_roots = [\"/absolute/path\"]), or move the file into the workspace."
        } else {
            "Add a matching parent directory to [autonomy].allowed_roots, or move the file into the workspace."
        };

        format!(
            "Resolved path escapes workspace allowlist: {}. {}",
            resolved.display(),
            guidance
        )
    }

    /// Check if autonomy level permits any action at all
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    fn is_protected_workspace_path(&self, path: &Path) -> bool {
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());

        let relative = if path.is_absolute() {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            match canonical.strip_prefix(&workspace_root) {
                Ok(stripped) => normalize_workspace_relative_path(stripped),
                Err(_) => return false,
            }
        } else {
            normalize_workspace_relative_path(path)
        };

        let mut components = relative.components();
        let Some(first) = components.next() else {
            return false;
        };
        if components.next().is_some() {
            return false;
        }

        let candidate = first.as_os_str().to_string_lossy();
        protected_workspace_files()
            .iter()
            .any(|protected| candidate.eq_ignore_ascii_case(protected))
    }

    fn normalize_tool_name(name: &str) -> String {
        name.trim().to_ascii_lowercase().replace('-', "_")
    }

    fn validate_otp_code(&self, otp_code: Option<&str>, context: &str) -> Result<(), String> {
        let Some(validator) = self.otp_validator.as_ref() else {
            return Err(format!(
                "OTP gating is configured for {context}, but no validator is available"
            ));
        };

        let code = otp_code
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("OTP code required for {context}; pass otp_code"))?;

        let valid = validator
            .validate(code)
            .map_err(|err| format!("Failed to validate OTP code for {context}: {err}"))?;
        if !valid {
            return Err(format!("Invalid OTP code for {context}"));
        }

        Ok(())
    }

    pub fn requires_otp_for_tool(&self, tool_name: &str) -> bool {
        self.otp_validator.is_some()
            && self
                .otp_gated_actions
                .contains(&Self::normalize_tool_name(tool_name))
    }

    pub fn enforce_otp_for_tool(
        &self,
        tool_name: &str,
        otp_code: Option<&str>,
    ) -> Result<(), String> {
        if !self.requires_otp_for_tool(tool_name) {
            return Ok(());
        }

        self.validate_otp_code(otp_code, &format!("tool '{tool_name}'"))
    }

    pub fn enforce_otp_for_url(
        &self,
        tool_name: &str,
        url: &str,
        otp_code: Option<&str>,
    ) -> Result<(), String> {
        if self.otp_validator.is_none() || !self.otp_gated_domains.is_gated(url) {
            return Ok(());
        }

        self.validate_otp_code(otp_code, &format!("domain-gated access in '{tool_name}'"))
    }

    pub fn enforce_sensitive_tool_operation(
        &self,
        tool_name: &str,
        operation: ToolOperation,
        otp_code: Option<&str>,
    ) -> Result<(), String> {
        if operation == ToolOperation::Act {
            self.enforce_otp_for_tool(tool_name, otp_code)?;
        }

        self.enforce_tool_operation(operation, tool_name)
    }

    // ── Tool Operation Gating ──────────────────────────────────────────────
    // Read operations bypass autonomy and rate checks because they have
    // no side effects. Act operations must pass both the autonomy gate
    // (not read-only) and the sliding-window rate limiter.

    /// Enforce policy for a tool operation.
    ///
    /// Read operations are always allowed by autonomy/rate gates.
    /// Act operations require non-readonly autonomy and available action budget.
    pub fn enforce_tool_operation(
        &self,
        operation: ToolOperation,
        operation_name: &str,
    ) -> Result<(), String> {
        match operation {
            ToolOperation::Read => Ok(()),
            ToolOperation::Act => {
                if !self.can_act() {
                    return Err(format!(
                        "Security policy: read-only mode, cannot perform '{operation_name}'"
                    ));
                }

                if !self.record_action() {
                    return Err("Rate limit exceeded: action budget exhausted".to_string());
                }

                Ok(())
            }
        }
    }

    /// Check the rate limit and, if allowed, record the action.
    /// Returns `true` if the action is allowed, `false` if rate-limited.
    pub fn record_action(&self) -> bool {
        if self.tracker.count() >= self.max_actions_per_hour as usize {
            return false;
        }
        self.tracker.record();
        true
    }

    /// Check if the rate limit would be exceeded without recording.
    pub fn is_rate_limited(&self) -> bool {
        self.tracker.count() >= self.max_actions_per_hour as usize
    }

    /// Build from config sections
    pub fn from_config(
        autonomy_config: &crate::config::AutonomyConfig,
        workspace_dir: &Path,
    ) -> Self {
        Self {
            autonomy: autonomy_config.level,
            workspace_dir: workspace_dir.to_path_buf(),
            workspace_only: autonomy_config.workspace_only,
            allowed_commands: autonomy_config.allowed_commands.clone(),
            forbidden_paths: autonomy_config.forbidden_paths.clone(),
            allowed_roots: autonomy_config
                .allowed_roots
                .iter()
                .map(|root| {
                    let expanded = expand_user_path(root);
                    if expanded.is_absolute() {
                        expanded
                    } else {
                        workspace_dir.join(expanded)
                    }
                })
                .collect(),
            max_actions_per_hour: autonomy_config.max_actions_per_hour,
            max_cost_per_day_cents: autonomy_config.max_cost_per_day_cents,
            require_approval_for_medium_risk: autonomy_config.require_approval_for_medium_risk,
            block_high_risk_commands: autonomy_config.block_high_risk_commands,
            shell_redirect_policy: autonomy_config.shell_redirect_policy,
            shell_env_passthrough: autonomy_config.shell_env_passthrough.clone(),
            otp_gated_actions: HashSet::new(),
            otp_gated_domains: DomainMatcher::default(),
            otp_validator: None,
            tracker: ActionTracker::new(),
        }
    }

    pub fn from_runtime_config(config: &crate::config::Config) -> anyhow::Result<Self> {
        let mut policy = Self::from_config(&config.autonomy, &config.workspace_dir);
        policy.otp_gated_actions = config
            .security
            .otp
            .gated_actions
            .iter()
            .map(|entry| Self::normalize_tool_name(entry))
            .collect();
        policy.otp_gated_domains = DomainMatcher::new(
            &config.security.otp.gated_domains,
            &config.security.otp.gated_domain_categories,
        )
        .context("Invalid OTP domain-gating configuration")?;

        if config.security.otp.enabled {
            let config_dir = config
                .config_path
                .parent()
                .context("Config path must have a parent directory")?;
            let store = SecretStore::new(config_dir, config.secrets.encrypt);
            let (validator, _enrollment_uri) =
                OtpValidator::from_config(&config.security.otp, config_dir, &store)?;
            policy.otp_validator = Some(Arc::new(validator));
        }

        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OtpConfig;
    use crate::security::SecretStore;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn test_default_allowed_commands() -> Vec<String> {
        [
            "ls", "git", "cargo", "cat", "grep", "date", "echo", "wc", "find",
        ]
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect()
    }

    fn default_policy() -> SecurityPolicy {
        SecurityPolicy {
            allowed_commands: test_default_allowed_commands(),
            ..SecurityPolicy::default()
        }
    }

    fn readonly_policy() -> SecurityPolicy {
        SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        }
    }

    fn full_policy() -> SecurityPolicy {
        SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            allowed_commands: test_default_allowed_commands(),
            ..SecurityPolicy::default()
        }
    }

    fn otp_test_policy(
        gated_actions: &[&str],
        gated_domains: &[&str],
    ) -> (TempDir, SecurityPolicy, String) {
        let tmp = TempDir::new().unwrap();
        let otp_config = OtpConfig {
            enabled: true,
            token_ttl_secs: 3600,
            cache_valid_secs: 7200,
            gated_actions: gated_actions.iter().map(|v| (*v).to_string()).collect(),
            gated_domains: gated_domains.iter().map(|v| (*v).to_string()).collect(),
            ..OtpConfig::default()
        };
        let store = SecretStore::new(tmp.path(), false);
        let (validator, _) = OtpValidator::from_config(&otp_config, tmp.path(), &store).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = validator.code_for_timestamp(now);
        let policy = SecurityPolicy {
            otp_gated_actions: gated_actions
                .iter()
                .map(|entry| SecurityPolicy::normalize_tool_name(entry))
                .collect(),
            otp_gated_domains: DomainMatcher::new(&otp_config.gated_domains, &[]).unwrap(),
            otp_validator: Some(Arc::new(validator)),
            ..SecurityPolicy::default()
        };
        (tmp, policy, code)
    }

    // ── AutonomyLevel ────────────────────────────────────────

    #[test]
    fn autonomy_default_is_supervised() {
        assert_eq!(AutonomyLevel::default(), AutonomyLevel::Supervised);
    }

    #[test]
    fn autonomy_serde_roundtrip() {
        let json = serde_json::to_string(&AutonomyLevel::Full).unwrap();
        assert_eq!(json, "\"full\"");
        let parsed: AutonomyLevel = serde_json::from_str("\"readonly\"").unwrap();
        assert_eq!(parsed, AutonomyLevel::ReadOnly);
        let parsed2: AutonomyLevel = serde_json::from_str("\"supervised\"").unwrap();
        assert_eq!(parsed2, AutonomyLevel::Supervised);
    }

    #[test]
    fn can_act_readonly_false() {
        assert!(!readonly_policy().can_act());
    }

    #[test]
    fn can_act_supervised_true() {
        assert!(default_policy().can_act());
    }

    #[test]
    fn can_act_full_true() {
        assert!(full_policy().can_act());
    }

    #[test]
    fn enforce_tool_operation_read_allowed_in_readonly_mode() {
        let p = readonly_policy();
        assert!(p
            .enforce_tool_operation(ToolOperation::Read, "memory_recall")
            .is_ok());
    }

    #[test]
    fn enforce_tool_operation_act_blocked_in_readonly_mode() {
        let p = readonly_policy();
        let err = p
            .enforce_tool_operation(ToolOperation::Act, "memory_store")
            .unwrap_err();
        assert!(err.contains("read-only mode"));
    }

    #[test]
    fn enforce_tool_operation_act_uses_rate_budget() {
        let p = SecurityPolicy {
            max_actions_per_hour: 0,
            ..default_policy()
        };
        let err = p
            .enforce_tool_operation(ToolOperation::Act, "memory_store")
            .unwrap_err();
        assert!(err.contains("Rate limit exceeded"));
    }

    // ── is_command_allowed ───────────────────────────────────

    #[test]
    fn allowed_commands_basic() {
        let p = default_policy();
        assert!(p.is_command_allowed("ls"));
        assert!(p.is_command_allowed("git status"));
        assert!(p.is_command_allowed("cargo build --release"));
        assert!(p.is_command_allowed("cat file.txt"));
        assert!(p.is_command_allowed("grep -r pattern ."));
        assert!(p.is_command_allowed("date"));
    }

    #[test]
    fn blocked_commands_basic() {
        let p = default_policy();
        assert!(!p.is_command_allowed("rm -rf /"));
        assert!(!p.is_command_allowed("sudo apt install"));
        assert!(!p.is_command_allowed("curl http://evil.com"));
        assert!(!p.is_command_allowed("wget http://evil.com"));
        assert!(!p.is_command_allowed("python3 exploit.py"));
        assert!(!p.is_command_allowed("node malicious.js"));
    }

    #[test]
    fn readonly_blocks_all_commands() {
        let p = readonly_policy();
        assert!(!p.is_command_allowed("ls"));
        assert!(!p.is_command_allowed("cat file.txt"));
        assert!(!p.is_command_allowed("echo hello"));
    }

    #[test]
    fn full_autonomy_still_uses_allowlist() {
        let p = full_policy();
        assert!(p.is_command_allowed("ls"));
        assert!(!p.is_command_allowed("rm -rf /"));
    }

    #[test]
    fn command_with_absolute_path_extracts_basename() {
        let p = default_policy();
        assert!(p.is_command_allowed("/usr/bin/git status"));
        assert!(p.is_command_allowed("/bin/ls -la"));
    }

    #[test]
    fn allowlist_supports_explicit_executable_paths() {
        let p = SecurityPolicy {
            allowed_commands: vec!["/usr/bin/antigravity".into()],
            ..SecurityPolicy::default()
        };

        assert!(p.is_command_allowed("/usr/bin/antigravity"));
        assert!(!p.is_command_allowed("antigravity"));
    }

    #[test]
    fn allowlist_supports_wildcard_entry() {
        let p = SecurityPolicy {
            allowed_commands: vec!["*".into()],
            ..SecurityPolicy::default()
        };

        assert!(p.is_command_allowed("python3 --version"));
        assert!(p.is_command_allowed("/usr/bin/antigravity"));

        // Wildcard still respects risk gates in validate_command_execution,
        // but explicit approval (approved=true) overrides the high-risk block.
        let allowed = p.validate_command_execution("rm -rf tmp_test_dir", true);
        assert!(allowed.is_ok());
    }

    #[test]
    fn empty_command_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed(""));
        assert!(!p.is_command_allowed("   "));
    }

    #[test]
    fn command_with_pipes_validates_all_segments() {
        let p = default_policy();
        // Both sides of the pipe are in the allowlist
        assert!(p.is_command_allowed("ls | grep foo"));
        assert!(p.is_command_allowed("cat file.txt | wc -l"));
        // Second command not in allowlist — blocked
        assert!(!p.is_command_allowed("ls | curl http://evil.com"));
        assert!(!p.is_command_allowed("echo hello | python3 -"));
    }

    #[test]
    fn custom_allowlist() {
        let p = SecurityPolicy {
            allowed_commands: vec!["docker".into(), "kubectl".into()],
            ..SecurityPolicy::default()
        };
        assert!(p.is_command_allowed("docker ps"));
        assert!(p.is_command_allowed("kubectl get pods"));
        assert!(!p.is_command_allowed("ls"));
        assert!(!p.is_command_allowed("git status"));
    }

    #[test]
    fn empty_allowlist_blocks_everything() {
        let p = SecurityPolicy {
            allowed_commands: vec![],
            ..SecurityPolicy::default()
        };
        assert!(!p.is_command_allowed("ls"));
        assert!(!p.is_command_allowed("echo hello"));
    }

    #[test]
    fn command_risk_low_for_read_commands() {
        let p = default_policy();
        assert_eq!(p.command_risk_level("git status"), CommandRiskLevel::Low);
        assert_eq!(p.command_risk_level("ls -la"), CommandRiskLevel::Low);
    }

    #[test]
    fn command_risk_medium_for_mutating_commands() {
        let p = SecurityPolicy {
            allowed_commands: vec!["git".into(), "touch".into()],
            ..SecurityPolicy::default()
        };
        assert_eq!(
            p.command_risk_level("git reset --hard HEAD~1"),
            CommandRiskLevel::Medium
        );
        assert_eq!(
            p.command_risk_level("touch file.txt"),
            CommandRiskLevel::Medium
        );
    }

    #[test]
    fn command_risk_high_for_dangerous_commands() {
        let p = SecurityPolicy {
            allowed_commands: vec!["rm".into()],
            ..SecurityPolicy::default()
        };
        assert_eq!(
            p.command_risk_level("rm -rf /tmp/test"),
            CommandRiskLevel::High
        );
    }

    #[test]
    fn validate_command_requires_approval_for_medium_risk() {
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            require_approval_for_medium_risk: true,
            allowed_commands: vec!["touch".into()],
            ..SecurityPolicy::default()
        };

        let denied = p.validate_command_execution("touch test.txt", false);
        assert!(denied.is_err());
        assert!(denied.unwrap_err().contains("requires explicit approval"),);

        let allowed = p.validate_command_execution("touch test.txt", true);
        assert_eq!(allowed.unwrap(), CommandRiskLevel::Medium);
    }

    #[test]
    fn validate_command_allows_exact_temporary_plan_command() {
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            require_approval_for_medium_risk: true,
            allowed_commands: vec![],
            ..SecurityPolicy::default()
        };

        let allowed = p.validate_command_execution_with_temporary_allowlist(
            "touch test.txt",
            true,
            &["touch test.txt".to_string()],
        );
        assert_eq!(allowed.unwrap(), CommandRiskLevel::Medium);
    }

    #[test]
    fn absolute_paths_allowed_when_matching_allowed_root() {
        let allowed_root = std::env::temp_dir().join("topclaw-policy-allowed-root");
        let p = SecurityPolicy {
            allowed_roots: vec![allowed_root.clone()],
            ..SecurityPolicy::default()
        };

        assert!(p.is_path_allowed(&allowed_root.join("file.txt").display().to_string()));
        assert!(!p.is_path_allowed("/var/tmp/file.txt"));
    }

    #[test]
    fn protected_workspace_rule_files_blocked() {
        let workspace = PathBuf::from("/tmp/topclaw-protected-policy");
        let p = SecurityPolicy {
            workspace_dir: workspace.clone(),
            ..SecurityPolicy::default()
        };

        assert!(!p.is_path_allowed("AGENTS.md"));
        assert!(!p.is_path_allowed("./SOUL.md"));
        assert!(!p.is_path_allowed(&workspace.join("TOOLS.md").display().to_string()));
        assert!(p.is_path_allowed("notes/AGENTS.md"));
    }

    #[test]
    fn forbidden_paths_blocked() {
        let p = SecurityPolicy {
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_path_allowed("/etc/passwd"));
        assert!(!p.is_path_allowed("/root/.bashrc"));
        assert!(!p.is_path_allowed("~/.ssh/id_rsa"));
        assert!(!p.is_path_allowed("~/.gnupg/pubring.kbx"));
    }

    #[test]
    fn empty_path_allowed() {
        let p = default_policy();
        assert!(p.is_path_allowed(""));
    }

    #[test]
    fn dotfile_in_workspace_allowed() {
        let p = default_policy();
        assert!(p.is_path_allowed(".gitignore"));
        assert!(p.is_path_allowed(".env"));
    }

    // ── from_config ─────────────────────────────────────────

    #[test]
    fn from_config_maps_all_fields() {
        let autonomy_config = crate::config::AutonomyConfig {
            level: AutonomyLevel::Full,
            workspace_only: false,
            allowed_commands: vec!["docker".into()],
            forbidden_paths: vec!["/secret".into()],
            max_actions_per_hour: 100,
            max_cost_per_day_cents: 1000,
            require_approval_for_medium_risk: false,
            block_high_risk_commands: false,
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            shell_env_passthrough: vec!["DATABASE_URL".into()],
            ..crate::config::AutonomyConfig::default()
        };
        let workspace = PathBuf::from("/tmp/test-workspace");
        let policy = SecurityPolicy::from_config(&autonomy_config, &workspace);

        assert_eq!(policy.autonomy, AutonomyLevel::Full);
        assert!(!policy.workspace_only);
        assert_eq!(policy.allowed_commands, vec!["docker"]);
        assert_eq!(policy.forbidden_paths, vec!["/secret"]);
        assert_eq!(policy.max_actions_per_hour, 100);
        assert_eq!(policy.max_cost_per_day_cents, 1000);
        assert!(!policy.require_approval_for_medium_risk);
        assert!(!policy.block_high_risk_commands);
        assert_eq!(policy.shell_redirect_policy, ShellRedirectPolicy::Strip);
        assert_eq!(policy.shell_env_passthrough, vec!["DATABASE_URL"]);
        assert_eq!(policy.workspace_dir, PathBuf::from("/tmp/test-workspace"));
    }

    #[test]
    fn from_config_normalizes_allowed_roots() {
        let autonomy_config = crate::config::AutonomyConfig {
            allowed_roots: vec!["~/Desktop".into(), "shared-data".into()],
            ..crate::config::AutonomyConfig::default()
        };
        let workspace = PathBuf::from("/tmp/test-workspace");
        let policy = SecurityPolicy::from_config(&autonomy_config, &workspace);

        let expected_home_root = if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join("Desktop")
        } else {
            PathBuf::from("~/Desktop")
        };

        assert_eq!(policy.allowed_roots[0], expected_home_root);
        assert_eq!(policy.allowed_roots[1], workspace.join("shared-data"));
    }

    #[test]
    fn resolved_path_violation_message_includes_allowed_roots_guidance() {
        let p = default_policy();
        let msg = p.resolved_path_violation_message(Path::new("/tmp/outside.txt"));
        assert!(msg.contains("escapes workspace"));
        assert!(msg.contains("allowed_roots"));
    }

    // ── Default policy ──────────────────────────────────────

    #[test]
    fn default_policy_has_sane_values() {
        let p = SecurityPolicy::default();
        assert_eq!(p.autonomy, AutonomyLevel::Supervised);
        assert!(p.workspace_only);
        assert!(p.allowed_commands.is_empty());
        assert!(p.forbidden_paths.iter().any(|p| p == "/etc"));
        assert_eq!(p.max_actions_per_hour, 20);
        assert_eq!(p.max_cost_per_day_cents, 500);
        assert!(p.require_approval_for_medium_risk);
        assert!(p.block_high_risk_commands);
        assert_eq!(p.shell_redirect_policy, ShellRedirectPolicy::Block);
        assert!(p.shell_env_passthrough.is_empty());
    }

    // ── ActionTracker / rate limiting ───────────────────────

    #[test]
    fn action_tracker_starts_at_zero() {
        let tracker = ActionTracker::new();
        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn action_tracker_records_actions() {
        let tracker = ActionTracker::new();
        assert_eq!(tracker.record(), 1);
        assert_eq!(tracker.record(), 2);
        assert_eq!(tracker.record(), 3);
        assert_eq!(tracker.count(), 3);
    }

    #[test]
    fn record_action_allows_within_limit() {
        let p = SecurityPolicy {
            max_actions_per_hour: 5,
            ..SecurityPolicy::default()
        };
        for _ in 0..5 {
            assert!(p.record_action(), "should allow actions within limit");
        }
    }

    #[test]
    fn record_action_blocks_over_limit() {
        let p = SecurityPolicy {
            max_actions_per_hour: 3,
            ..SecurityPolicy::default()
        };
        assert!(p.record_action()); // 1
        assert!(p.record_action()); // 2
        assert!(p.record_action()); // 3
        assert!(!p.record_action()); // 4 — over limit
    }

    #[test]
    fn is_rate_limited_reflects_count() {
        let p = SecurityPolicy {
            max_actions_per_hour: 2,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_rate_limited());
        p.record_action();
        assert!(!p.is_rate_limited());
        p.record_action();
        assert!(p.is_rate_limited());
    }

    #[test]
    fn action_tracker_clone_is_independent() {
        let tracker = ActionTracker::new();
        tracker.record();
        tracker.record();
        let cloned = tracker.clone();
        assert_eq!(cloned.count(), 2);
        tracker.record();
        assert_eq!(tracker.count(), 3);
        assert_eq!(cloned.count(), 2); // clone is independent
    }

    // ── Edge cases: command injection ────────────────────────

    #[test]
    fn command_injection_semicolon_blocked() {
        let p = default_policy();
        // First word is "ls;" (with semicolon) — doesn't match "ls" in allowlist.
        // This is a safe default: chained commands are blocked.
        assert!(!p.is_command_allowed("ls; rm -rf /"));
    }

    #[test]
    fn command_injection_semicolon_no_space() {
        let p = default_policy();
        assert!(!p.is_command_allowed("ls;rm -rf /"));
    }

    #[test]
    fn quoted_semicolons_do_not_split_sqlite_command() {
        let p = SecurityPolicy {
            allowed_commands: vec!["sqlite3".into()],
            ..SecurityPolicy::default()
        };
        assert!(p.is_command_allowed(
            "sqlite3 /tmp/test.db \"CREATE TABLE t(id INT); INSERT INTO t VALUES(1); SELECT * FROM t;\""
        ));
        assert_eq!(
            p.command_risk_level(
                "sqlite3 /tmp/test.db \"CREATE TABLE t(id INT); INSERT INTO t VALUES(1); SELECT * FROM t;\""
            ),
            CommandRiskLevel::Low
        );
    }

    #[test]
    fn unquoted_semicolon_after_quoted_sql_still_splits_commands() {
        let p = SecurityPolicy {
            allowed_commands: vec!["sqlite3".into()],
            ..SecurityPolicy::default()
        };
        assert!(!p.is_command_allowed("sqlite3 /tmp/test.db \"SELECT 1;\"; rm -rf /"));
    }

    #[test]
    fn command_injection_backtick_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo `whoami`"));
        assert!(!p.is_command_allowed("echo `rm -rf /`"));
    }

    #[test]
    fn command_injection_dollar_paren_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo $(cat /etc/passwd)"));
        assert!(!p.is_command_allowed("echo $(rm -rf /)"));
    }

    #[test]
    fn command_injection_dollar_paren_literal_inside_single_quotes_allowed() {
        let p = default_policy();
        assert!(p.is_command_allowed("echo '$(cat /etc/passwd)'"));
    }

    #[test]
    fn command_injection_dollar_brace_literal_inside_single_quotes_allowed() {
        let p = default_policy();
        assert!(p.is_command_allowed("echo '${HOME}'"));
    }

    #[test]
    fn command_injection_dollar_brace_unquoted_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo ${HOME}"));
    }

    #[test]
    fn command_with_env_var_prefix() {
        let p = default_policy();
        // "FOO=bar" is the first word — not in allowlist
        assert!(!p.is_command_allowed("FOO=bar rm -rf /"));
    }

    #[test]
    fn command_newline_injection_blocked() {
        let p = default_policy();
        // Newline splits into two commands; "rm" is not in allowlist
        assert!(!p.is_command_allowed("ls\nrm -rf /"));
        // Both allowed — OK
        assert!(p.is_command_allowed("ls\necho hello"));
    }

    #[test]
    fn command_injection_and_chain_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("ls && rm -rf /"));
        assert!(!p.is_command_allowed("echo ok && curl http://evil.com"));
        // Both allowed — OK
        assert!(p.is_command_allowed("ls && echo done"));
    }

    #[test]
    fn command_injection_or_chain_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("ls || rm -rf /"));
        // Both allowed — OK
        assert!(p.is_command_allowed("ls || echo fallback"));
    }

    #[test]
    fn command_injection_background_chain_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("ls & rm -rf /"));
        assert!(!p.is_command_allowed("ls&rm -rf /"));
        assert!(!p.is_command_allowed("echo ok & python3 -c 'print(1)'"));
    }

    #[test]
    fn command_injection_redirect_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo secret > /etc/crontab"));
        assert!(!p.is_command_allowed("ls >> /tmp/exfil.txt"));
        assert!(!p.is_command_allowed("cat </etc/passwd"));
        assert!(!p.is_command_allowed("cat</etc/passwd"));
    }

    #[test]
    fn strip_policy_normalizes_common_redirect_patterns() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            ..default_policy()
        };

        let merged = p.apply_shell_redirect_policy("echo hello 2>&1");
        assert!(!merged.contains("2>&1"));
        assert!(merged.contains("echo hello"));

        let devnull = p.apply_shell_redirect_policy("echo hello 2>/dev/null");
        assert!(!devnull.contains("/dev/null"));
        assert!(devnull.contains("echo hello"));

        let pipeline = p.apply_shell_redirect_policy("echo hello |& cat");
        assert!(!pipeline.contains("|&"));
        assert!(pipeline.contains("| cat"));

        let quoted = p.apply_shell_redirect_policy("echo '2>&1' \"|&\" '2>/dev/null'");
        assert_eq!(quoted, "echo '2>&1' \"|&\" '2>/dev/null'");
    }

    #[test]
    fn strip_policy_preserves_command_trailing_digits_when_stripping() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            ..default_policy()
        };

        let merged = p.apply_shell_redirect_policy("python3>&1 -V");
        assert_eq!(merged, "python3 -V");

        let devnull = p.apply_shell_redirect_policy("python3>/dev/null -V");
        assert_eq!(devnull, "python3 -V");

        let stdin_devnull = p.apply_shell_redirect_policy("python3</dev/null -V");
        assert_eq!(stdin_devnull, "python3 -V");
    }

    #[test]
    fn strip_policy_keeps_digit_suffixed_commands_allowlisted() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            allowed_commands: vec!["python3".into()],
            ..default_policy()
        };

        assert!(p.validate_command_execution("python3>&1 -V", false).is_ok());
        assert!(p
            .validate_command_execution("python3>/dev/null -V", false)
            .is_ok());
    }

    #[test]
    fn strip_policy_allows_normalized_stderr_redirects() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            allowed_commands: vec!["echo".into()],
            ..default_policy()
        };

        assert!(p
            .validate_command_execution("echo hello 2>&1", false)
            .is_ok());
        assert!(p
            .validate_command_execution("echo hello 2>/dev/null", false)
            .is_ok());
    }

    #[test]
    fn strip_policy_keeps_unsupported_redirects_blocked() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Strip,
            ..default_policy()
        };

        assert!(p
            .validate_command_execution("echo hello > out.txt", false)
            .is_err());
        assert!(p
            .validate_command_execution("cat </etc/passwd", false)
            .is_err());
    }

    #[test]
    fn allow_policy_normalizes_python_heredoc_commands() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Allow,
            ..default_policy()
        };

        let normalized = p.apply_shell_redirect_policy("python3 - <<'PY'\nprint('ok')\nPY");
        assert_eq!(normalized, "python3 -c 'print('\"'\"'ok'\"'\"')'");
    }

    #[test]
    fn allow_policy_accepts_normalized_python_heredoc_commands() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Allow,
            allowed_commands: vec!["python3".into()],
            ..default_policy()
        };

        assert!(p
            .validate_command_execution("python3 - <<'PY'\nprint('ok')\nPY", false)
            .is_ok());
    }

    #[test]
    fn allow_policy_accepts_agent_style_multiline_shell_scripts() {
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Allow,
            allowed_commands: vec!["*".into()],
            require_approval_for_medium_risk: false,
            ..default_policy()
        };

        let command = concat!(
            "set -euo pipefail\n",
            "repo_dir=\"tmp/topagent_$(date +%s)\"\n",
            "printf '%s\\n' \"${repo_dir}\""
        );

        assert!(p.validate_command_execution(command, false).is_ok());
    }

    #[test]
    fn allow_policy_accepts_stream_merge_redirects_in_pipelines() {
        // Regression: `2>&1` was being flagged as a background `&` even though
        // shell_redirect_policy=Allow opts into full shell redirect semantics.
        // This blocked legitimate `git clone ... 2>&1 | tail -5` pipelines.
        let p = SecurityPolicy {
            shell_redirect_policy: ShellRedirectPolicy::Allow,
            allowed_commands: vec!["*".into()],
            require_approval_for_medium_risk: false,
            ..default_policy()
        };

        assert!(p
            .validate_command_execution(
                "git clone --depth 1 https://example.com/repo.git 2>&1 | tail -5",
                false,
            )
            .is_ok());
        assert!(p.validate_command_execution("echo ok >&2", false).is_ok());
        assert!(p.validate_command_execution("cmd 2>&1 1>&2", false).is_ok());

        // Real background-execution `&` must still be blocked.
        assert!(p
            .validate_command_execution("long_task & echo done", false)
            .is_err());
    }

    #[test]
    fn quoted_ampersand_and_redirect_literals_are_not_treated_as_operators() {
        let p = default_policy();
        assert!(p.is_command_allowed("echo \"A&B\""));
        assert!(p.is_command_allowed("echo \"A>B\""));
        assert!(p.is_command_allowed("echo \"A<B\""));
    }

    #[test]
    fn command_argument_injection_blocked() {
        let p = default_policy();
        // find -exec is a common bypass
        assert!(!p.is_command_allowed("find . -exec rm -rf {} +"));
        assert!(!p.is_command_allowed("find / -ok cat {} \\;"));
        // git config/alias can execute commands
        assert!(!p.is_command_allowed("git config core.editor \"rm -rf /\""));
        assert!(!p.is_command_allowed("git alias.st status"));
        assert!(!p.is_command_allowed("git -c core.editor=calc.exe commit"));
        // Legitimate commands should still work
        assert!(p.is_command_allowed("find . -name '*.txt'"));
        assert!(p.is_command_allowed("git status"));
        assert!(p.is_command_allowed("git add ."));
    }

    #[test]
    fn command_injection_dollar_brace_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo ${IFS}cat${IFS}/etc/passwd"));
    }

    #[test]
    fn command_injection_plain_dollar_var_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("cat $HOME/.ssh/id_rsa"));
        assert!(!p.is_command_allowed("cat $SECRET_FILE"));
    }

    #[test]
    fn command_injection_tee_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("echo secret | tee /etc/crontab"));
        assert!(!p.is_command_allowed("ls | /usr/bin/tee outfile"));
        assert!(!p.is_command_allowed("tee file.txt"));
    }

    #[test]
    fn command_injection_process_substitution_blocked() {
        let p = default_policy();
        assert!(!p.is_command_allowed("cat <(echo pwned)"));
        assert!(!p.is_command_allowed("ls >(cat /etc/passwd)"));
    }

    #[test]
    fn command_env_var_prefix_with_allowed_cmd() {
        let p = default_policy();
        // env assignment + allowed command — OK
        assert!(p.is_command_allowed("FOO=bar ls"));
        assert!(p.is_command_allowed("LANG=C grep pattern file"));
        // env assignment + disallowed command — blocked
        assert!(!p.is_command_allowed("FOO=bar rm -rf /"));
    }

    #[test]
    fn forbidden_path_argument_detects_absolute_path() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("cat /etc/passwd"),
            Some("/etc/passwd".into())
        );
    }

    #[test]
    fn validate_command_execution_rejects_forbidden_paths() {
        let p = default_policy();
        let err = p
            .validate_command_execution("cat /etc/shadow", false)
            .unwrap_err();
        assert!(err.contains("Path blocked by security policy"));
    }

    #[test]
    fn forbidden_path_argument_detects_parent_dir_reference() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("cat ../secret.txt"),
            Some("../secret.txt".into())
        );
        assert_eq!(
            p.forbidden_path_argument("find .. -name '*.rs'"),
            Some("..".into())
        );
    }

    #[test]
    fn forbidden_path_argument_allows_workspace_relative_paths() {
        let p = default_policy();
        assert_eq!(p.forbidden_path_argument("cat src/main.rs"), None);
        assert_eq!(p.forbidden_path_argument("grep -r todo ./src"), None);
    }

    #[test]
    fn forbidden_path_argument_detects_option_assignment_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("grep --file=/etc/passwd root ./src"),
            Some("/etc/passwd".into())
        );
        assert_eq!(
            p.forbidden_path_argument("cat --input=../secret.txt"),
            Some("../secret.txt".into())
        );
    }

    #[test]
    fn forbidden_path_argument_allows_safe_option_assignment_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("grep --file=./patterns.txt root ./src"),
            None
        );
    }

    #[test]
    fn forbidden_path_argument_detects_short_option_attached_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("grep -f/etc/passwd root ./src"),
            Some("/etc/passwd".into())
        );
        assert_eq!(
            p.forbidden_path_argument("git -C../outside status"),
            Some("../outside".into())
        );
    }

    #[test]
    fn forbidden_path_argument_allows_safe_short_option_attached_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("grep -f./patterns.txt root ./src"),
            None
        );
        assert_eq!(p.forbidden_path_argument("git -C./repo status"), None);
    }

    #[test]
    fn forbidden_path_argument_detects_tilde_user_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("cat ~root/.ssh/id_rsa"),
            Some("~root/.ssh/id_rsa".into())
        );
        assert_eq!(
            p.forbidden_path_argument("ls ~nobody"),
            Some("~nobody".into())
        );
    }

    #[test]
    fn forbidden_path_argument_detects_input_redirection_paths() {
        let p = default_policy();
        assert_eq!(
            p.forbidden_path_argument("cat </etc/passwd"),
            Some("/etc/passwd".into())
        );
        assert_eq!(
            p.forbidden_path_argument("cat</etc/passwd"),
            Some("/etc/passwd".into())
        );
    }

    // ── Edge cases: path traversal ──────────────────────────

    #[test]
    fn path_traversal_encoded_dots() {
        let p = default_policy();
        // Literal ".." in path — always blocked
        assert!(!p.is_path_allowed("foo/..%2f..%2fetc/passwd"));
    }

    #[test]
    fn path_traversal_double_dot_in_filename() {
        let p = default_policy();
        // ".." in a filename (not a path component) is allowed
        assert!(p.is_path_allowed("my..file.txt"));
        // But actual traversal components are still blocked
        assert!(!p.is_path_allowed("../etc/passwd"));
        assert!(!p.is_path_allowed("foo/../etc/passwd"));
    }

    #[test]
    fn path_with_null_byte_blocked() {
        let p = default_policy();
        assert!(!p.is_path_allowed("file\0.txt"));
    }

    #[test]
    fn path_symlink_style_absolute() {
        let p = default_policy();
        assert!(!p.is_path_allowed("/proc/self/root/etc/passwd"));
    }

    #[test]
    fn path_home_tilde_ssh() {
        let p = SecurityPolicy {
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_path_allowed("~/.ssh/id_rsa"));
        assert!(!p.is_path_allowed("~/.gnupg/secring.gpg"));
        assert!(!p.is_path_allowed("~root/.ssh/id_rsa"));
        assert!(!p.is_path_allowed("~nobody"));
    }

    #[test]
    fn path_var_run_blocked() {
        let p = SecurityPolicy {
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_path_allowed("/var/run/docker.sock"));
    }

    // ── Edge cases: rate limiter boundary ────────────────────

    #[test]
    fn rate_limit_exactly_at_boundary() {
        let p = SecurityPolicy {
            max_actions_per_hour: 1,
            ..SecurityPolicy::default()
        };
        assert!(p.record_action()); // 1 — exactly at limit
        assert!(!p.record_action()); // 2 — over
        assert!(!p.record_action()); // 3 — still over
    }

    #[test]
    fn rate_limit_zero_blocks_everything() {
        let p = SecurityPolicy {
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        };
        assert!(!p.record_action());
    }

    #[test]
    fn rate_limit_high_allows_many() {
        let p = SecurityPolicy {
            max_actions_per_hour: 10000,
            ..SecurityPolicy::default()
        };
        for _ in 0..100 {
            assert!(p.record_action());
        }
    }

    // ── Edge cases: autonomy + command combos ────────────────

    #[test]
    fn readonly_blocks_even_safe_commands() {
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            allowed_commands: vec!["ls".into(), "cat".into()],
            ..SecurityPolicy::default()
        };
        assert!(!p.is_command_allowed("ls"));
        assert!(!p.is_command_allowed("cat"));
        assert!(!p.can_act());
    }

    #[test]
    fn validate_command_allows_high_risk_when_explicitly_approved() {
        // block_high_risk_commands is true by default, but explicit approval
        // (approved=true) overrides it. The user's judgment wins.
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            allowed_commands: vec!["rm".into()],
            ..SecurityPolicy::default()
        };

        let result = p.validate_command_execution("rm -rf tmp_test_dir", true);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), CommandRiskLevel::High);
    }

    #[test]
    fn validate_command_blocks_high_risk_without_approval() {
        // Without explicit approval, high-risk commands are blocked.
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            allowed_commands: vec!["rm".into()],
            ..SecurityPolicy::default()
        };

        let result = p.validate_command_execution("rm -rf tmp_test_dir", false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("high-risk"));
    }

    #[test]
    fn full_autonomy_still_respects_forbidden_paths() {
        let p = SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_path_allowed("/etc/shadow"));
        assert!(!p.is_path_allowed("/root/.bashrc"));
    }

    #[test]
    fn workspace_only_false_allows_resolved_outside_workspace() {
        let workspace = std::env::temp_dir().join("topclaw_test_ws_only_false");
        let _ = std::fs::create_dir_all(&workspace);
        let canonical_workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.clone());

        let p = SecurityPolicy {
            workspace_dir: canonical_workspace.clone(),
            workspace_only: false,
            forbidden_paths: vec!["/etc".into(), "/var".into()],
            ..SecurityPolicy::default()
        };

        // Path outside workspace should be allowed when workspace_only=false
        let outside = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home"))
            .join("topclaw_outside_ws");
        assert!(
            p.is_resolved_path_allowed(&outside),
            "workspace_only=false must allow resolved paths outside workspace"
        );

        // Forbidden paths must still be blocked even with workspace_only=false
        assert!(
            !p.is_resolved_path_allowed(Path::new("/etc/passwd")),
            "forbidden paths must be blocked even when workspace_only=false"
        );
        assert!(
            !p.is_resolved_path_allowed(Path::new("/var/run/docker.sock")),
            "forbidden /var must be blocked even when workspace_only=false"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn workspace_only_true_blocks_resolved_outside_workspace() {
        let workspace = std::env::temp_dir().join("topclaw_test_ws_only_true");
        let _ = std::fs::create_dir_all(&workspace);
        let canonical_workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.clone());

        let p = SecurityPolicy {
            workspace_dir: canonical_workspace.clone(),
            workspace_only: true,
            ..SecurityPolicy::default()
        };

        // Path inside workspace — allowed
        let inside = canonical_workspace.join("subdir");
        assert!(
            p.is_resolved_path_allowed(&inside),
            "path inside workspace must be allowed"
        );

        // Path outside workspace — blocked
        let outside = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("topclaw_outside_ws_true");
        assert!(
            !p.is_resolved_path_allowed(&outside),
            "workspace_only=true must block resolved paths outside workspace"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    // ── Edge cases: from_config preserves tracker ────────────

    #[test]
    fn from_config_creates_fresh_tracker() {
        let autonomy_config = crate::config::AutonomyConfig {
            level: AutonomyLevel::Full,
            workspace_only: false,
            allowed_commands: vec![],
            forbidden_paths: vec![],
            max_actions_per_hour: 10,
            max_cost_per_day_cents: 100,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            ..crate::config::AutonomyConfig::default()
        };
        let workspace = PathBuf::from("/tmp/test");
        let policy = SecurityPolicy::from_config(&autonomy_config, &workspace);
        assert_eq!(policy.tracker.count(), 0);
        assert!(!policy.is_rate_limited());
    }

    // ══════════════════════════════════════════════════════════
    // SECURITY CHECKLIST TESTS
    // Checklist: gateway not public, pairing required,
    //            filesystem scoped (no /), access via tunnel
    // ══════════════════════════════════════════════════════════

    // ── Checklist #3: Filesystem scoped (no /) ──────────────

    #[test]
    fn checklist_root_path_blocked() {
        let p = default_policy();
        assert!(!p.is_path_allowed("/"));
        assert!(!p.is_path_allowed("/anything"));
    }

    #[test]
    fn checklist_all_system_dirs_blocked() {
        let p = SecurityPolicy {
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        for dir in [
            "/etc", "/root", "/home", "/usr", "/bin", "/sbin", "/lib", "/opt", "/boot", "/dev",
            "/proc", "/sys", "/var", "/tmp",
        ] {
            assert!(
                !p.is_path_allowed(dir),
                "System dir should be blocked: {dir}"
            );
            assert!(
                !p.is_path_allowed(&format!("{dir}/subpath")),
                "Subpath of system dir should be blocked: {dir}/subpath"
            );
        }
    }

    #[test]
    fn checklist_sensitive_dotfiles_blocked() {
        let p = SecurityPolicy {
            workspace_only: false,
            ..SecurityPolicy::default()
        };
        for path in [
            "~/.ssh/id_rsa",
            "~/.gnupg/secring.gpg",
            "~/.aws/credentials",
            "~/.config/secrets",
        ] {
            assert!(
                !p.is_path_allowed(path),
                "Sensitive dotfile should be blocked: {path}"
            );
        }
    }

    #[test]
    fn checklist_null_byte_injection_blocked() {
        let p = default_policy();
        assert!(!p.is_path_allowed("safe\0/../../../etc/passwd"));
        assert!(!p.is_path_allowed("\0"));
        assert!(!p.is_path_allowed("file\0"));
    }

    #[test]
    fn checklist_workspace_only_blocks_all_absolute() {
        let p = SecurityPolicy {
            workspace_only: true,
            ..SecurityPolicy::default()
        };
        assert!(!p.is_path_allowed("/any/absolute/path"));
        assert!(p.is_path_allowed("relative/path.txt"));
    }

    #[test]
    fn checklist_resolved_path_must_be_in_workspace() {
        let p = SecurityPolicy {
            workspace_dir: PathBuf::from("/home/user/project"),
            ..SecurityPolicy::default()
        };
        // Inside workspace — allowed
        assert!(p.is_resolved_path_allowed(Path::new("/home/user/project/src/main.rs")));
        // Outside workspace — blocked (symlink escape)
        assert!(!p.is_resolved_path_allowed(Path::new("/etc/passwd")));
        assert!(!p.is_resolved_path_allowed(Path::new("/home/user/other_project/file")));
        // Root — blocked
        assert!(!p.is_resolved_path_allowed(Path::new("/")));
    }

    #[test]
    fn checklist_default_policy_is_workspace_only() {
        let p = SecurityPolicy::default();
        assert!(
            p.workspace_only,
            "Default policy must be workspace_only=true"
        );
    }

    #[test]
    fn checklist_default_forbidden_paths_comprehensive() {
        let p = SecurityPolicy::default();
        // Must contain all critical system dirs
        for dir in ["/etc", "/root", "/proc", "/sys", "/dev", "/var", "/tmp"] {
            assert!(
                p.forbidden_paths.iter().any(|f| f == dir),
                "Default forbidden_paths must include {dir}"
            );
        }
        // Must contain sensitive dotfiles
        for dot in ["~/.ssh", "~/.gnupg", "~/.aws"] {
            assert!(
                p.forbidden_paths.iter().any(|f| f == dot),
                "Default forbidden_paths must include {dot}"
            );
        }
    }

    // ── §1.2 Path resolution / symlink bypass tests ──────────

    #[test]
    fn resolved_path_blocks_outside_workspace() {
        let workspace = std::env::temp_dir().join("topclaw_test_resolved_path");
        let _ = std::fs::create_dir_all(&workspace);

        // Use the canonicalized workspace so starts_with checks match
        let canonical_workspace = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.clone());

        let policy = SecurityPolicy {
            workspace_dir: canonical_workspace.clone(),
            ..SecurityPolicy::default()
        };

        // A resolved path inside the workspace should be allowed
        let inside = canonical_workspace.join("subdir").join("file.txt");
        assert!(
            policy.is_resolved_path_allowed(&inside),
            "path inside workspace should be allowed"
        );

        // A resolved path outside the workspace should be blocked
        let canonical_temp = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        let outside = canonical_temp.join("outside_workspace_topclaw");
        assert!(
            !policy.is_resolved_path_allowed(&outside),
            "path outside workspace must be blocked"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn resolved_path_blocks_root_escape() {
        let policy = SecurityPolicy {
            workspace_dir: PathBuf::from("/home/topclaw_user/project"),
            ..SecurityPolicy::default()
        };

        assert!(
            !policy.is_resolved_path_allowed(Path::new("/etc/passwd")),
            "resolved path to /etc/passwd must be blocked"
        );
        assert!(
            !policy.is_resolved_path_allowed(Path::new("/root/.bashrc")),
            "resolved path to /root/.bashrc must be blocked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolved_path_blocks_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join("topclaw_test_symlink_escape");
        let workspace = root.join("workspace");
        let outside = root.join("outside_target");

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // Create a symlink inside workspace pointing outside
        let link_path = workspace.join("escape_link");
        symlink(&outside, &link_path).unwrap();

        let policy = SecurityPolicy {
            workspace_dir: workspace.clone(),
            ..SecurityPolicy::default()
        };

        // The resolved symlink target should be outside workspace
        let resolved = link_path.canonicalize().unwrap();
        assert!(
            !policy.is_resolved_path_allowed(&resolved),
            "symlink-resolved path outside workspace must be blocked"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn allowed_roots_permits_paths_outside_workspace() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join("topclaw_test_allowed_roots");
        let workspace = root.join("workspace");
        let extra = root.join("extra_root");
        let extra_file = extra.join("data.txt");

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::write(&extra_file, "test").unwrap();

        // Symlink inside workspace pointing to extra root
        let link_path = workspace.join("link_to_extra");
        symlink(&extra, &link_path).unwrap();

        let resolved = link_path.join("data.txt").canonicalize().unwrap();

        // Without allowed_roots — blocked (symlink escape)
        let policy_without = SecurityPolicy {
            workspace_dir: workspace.clone(),
            allowed_roots: vec![],
            ..SecurityPolicy::default()
        };
        assert!(
            !policy_without.is_resolved_path_allowed(&resolved),
            "without allowed_roots, symlink target must be blocked"
        );

        // With allowed_roots — permitted
        let policy_with = SecurityPolicy {
            workspace_dir: workspace.clone(),
            allowed_roots: vec![extra.clone()],
            ..SecurityPolicy::default()
        };
        assert!(
            policy_with.is_resolved_path_allowed(&resolved),
            "with allowed_roots containing the target, symlink must be allowed"
        );

        // Unrelated path still blocked
        let unrelated = root.join("unrelated");
        std::fs::create_dir_all(&unrelated).unwrap();
        assert!(
            !policy_with.is_resolved_path_allowed(&unrelated.canonicalize().unwrap()),
            "paths outside workspace and allowed_roots must still be blocked"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_path_allowed_blocks_null_bytes() {
        let policy = default_policy();
        assert!(
            !policy.is_path_allowed("file\0.txt"),
            "paths with null bytes must be blocked"
        );
    }

    #[test]
    fn is_path_allowed_blocks_url_encoded_traversal() {
        let policy = default_policy();
        assert!(
            !policy.is_path_allowed("..%2fetc%2fpasswd"),
            "URL-encoded path traversal must be blocked"
        );
        assert!(
            !policy.is_path_allowed("subdir%2f..%2f..%2fetc"),
            "URL-encoded parent dir traversal must be blocked"
        );
    }

    #[test]
    fn sensitive_tool_operation_requires_valid_otp_for_gated_tool() {
        let (_tmp, policy, code) = otp_test_policy(&["shell"], &[]);

        let missing = policy.enforce_sensitive_tool_operation("shell", ToolOperation::Act, None);
        assert!(missing.unwrap_err().contains("OTP code required"));

        let invalid =
            policy.enforce_sensitive_tool_operation("shell", ToolOperation::Act, Some("000000"));
        assert!(invalid.unwrap_err().contains("Invalid OTP code"));

        policy
            .enforce_sensitive_tool_operation("shell", ToolOperation::Act, Some(&code))
            .expect("valid OTP should allow gated tool");
    }

    #[test]
    fn otp_is_required_for_gated_domains() {
        let (_tmp, policy, code) = otp_test_policy(&[], &["accounts.google.com"]);

        let missing =
            policy.enforce_otp_for_url("browser_open", "https://accounts.google.com", None);
        assert!(missing.unwrap_err().contains("OTP code required"));

        policy
            .enforce_otp_for_url("browser_open", "https://accounts.google.com", Some(&code))
            .expect("valid OTP should allow gated domain");
        policy
            .enforce_otp_for_url("browser_open", "https://example.com", None)
            .expect("non-gated domains should not require OTP");
    }
}
