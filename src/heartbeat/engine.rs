use crate::config::HeartbeatConfig;
use crate::observability::{Observer, ObserverEvent};
use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{info, warn};

const DEFAULT_MIN_COOLDOWN_MINUTES: u32 = 60;
const FAILURE_RETRY_MINUTES: u32 = 5;
const MAX_FAILURE_BACKOFF_SHIFT: u32 = 5;
const MAX_TASKS_PER_TICK: usize = 3;

/// Parsed heartbeat task with stateful scheduling metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatTask {
    pub key: String,
    pub text: String,
    pub cooldown_minutes: u32,
    pub priority: i32,
    pub max_runs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HeartbeatTaskState {
    #[serde(default)]
    last_run_at: Option<String>,
    #[serde(default)]
    next_due_at: Option<String>,
    #[serde(default)]
    last_status: Option<String>,
    #[serde(default)]
    consecutive_failures: u32,
    #[serde(default)]
    total_runs: u64,
    #[serde(default)]
    total_successes: u64,
    #[serde(default)]
    total_failures: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HeartbeatState {
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    tasks: BTreeMap<String, HeartbeatTaskState>,
}

/// Heartbeat engine — reads HEARTBEAT.md, tracks due work, and executes on schedule.
pub struct HeartbeatEngine {
    config: HeartbeatConfig,
    workspace_dir: PathBuf,
    observer: Arc<dyn Observer>,
}

impl HeartbeatEngine {
    pub fn new(
        config: HeartbeatConfig,
        workspace_dir: PathBuf,
        observer: Arc<dyn Observer>,
    ) -> Self {
        Self {
            config,
            workspace_dir,
            observer,
        }
    }

    /// Start the heartbeat loop (runs until cancelled)
    pub async fn run(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Heartbeat disabled");
            return Ok(());
        }

        let interval_mins = self.config.interval_minutes.max(5);
        info!("Heartbeat started: every {} minutes", interval_mins);

        let mut interval = time::interval(Duration::from_secs(u64::from(interval_mins) * 60));

        loop {
            interval.tick().await;
            self.observer.record_event(&ObserverEvent::HeartbeatTick);

            match self.tick().await {
                Ok(tasks) => {
                    if tasks > 0 {
                        info!("Heartbeat: {} task(s) due", tasks);
                    }
                }
                Err(e) => {
                    warn!("Heartbeat error: {}", e);
                    self.observer.record_event(&ObserverEvent::Error {
                        component: "heartbeat".into(),
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    /// Single heartbeat tick — count due tasks.
    async fn tick(&self) -> Result<usize> {
        Ok(self.due_tasks().await?.len())
    }

    /// Backward-compatible plain task list for diagnostics and legacy callers.
    pub async fn collect_tasks(&self) -> Result<Vec<String>> {
        Ok(self
            .collect_task_specs()
            .await?
            .into_iter()
            .map(|task| task.text)
            .collect())
    }

    /// Read HEARTBEAT.md and return parsed task specs.
    pub async fn collect_task_specs(&self) -> Result<Vec<HeartbeatTask>> {
        let heartbeat_path = self.workspace_dir.join("HEARTBEAT.md");
        if !heartbeat_path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&heartbeat_path).await?;
        Ok(Self::parse_tasks(&content, self.config.interval_minutes))
    }

    /// Return due tasks for this tick, bounded by priority and per-tick batch size.
    pub async fn due_tasks(&self) -> Result<Vec<HeartbeatTask>> {
        let parsed = self.collect_task_specs().await?;
        let tasks = if parsed.is_empty() {
            self.fallback_tasks()
        } else {
            parsed
        };
        if tasks.is_empty() {
            return Ok(Vec::new());
        }

        let state = self.load_state().await?;
        let now = Utc::now();
        let mut due: Vec<(HeartbeatTask, DateTime<Utc>)> = tasks
            .into_iter()
            .filter_map(|task| {
                let entry = state.tasks.get(&task.key);
                if task.max_runs.is_some_and(|limit| {
                    entry.is_some_and(|task_state| task_state.total_runs >= limit)
                }) {
                    return None;
                }

                let due_at = entry
                    .and_then(|task_state| parse_rfc3339(task_state.next_due_at.as_deref()))
                    .unwrap_or(now);
                if due_at <= now {
                    Some((task, due_at))
                } else {
                    None
                }
            })
            .collect();

        due.sort_by(|(left_task, left_due), (right_task, right_due)| {
            right_task
                .priority
                .cmp(&left_task.priority)
                .then_with(|| left_due.cmp(right_due))
                .then_with(|| left_task.text.cmp(&right_task.text))
        });
        due.truncate(MAX_TASKS_PER_TICK);
        Ok(due.into_iter().map(|(task, _)| task).collect())
    }

    /// Record the result of a heartbeat task and schedule its next due time.
    pub async fn record_task_result(&self, task: &HeartbeatTask, success: bool) -> Result<()> {
        let mut state = self.load_state().await?;
        let now = Utc::now();
        let entry = state.tasks.entry(task.key.clone()).or_default();
        entry.last_run_at = Some(now.to_rfc3339());
        entry.total_runs = entry.total_runs.saturating_add(1);

        let next_delay = if success {
            entry.last_status = Some("ok".into());
            entry.consecutive_failures = 0;
            entry.total_successes = entry.total_successes.saturating_add(1);
            task.cooldown_minutes.max(1)
        } else {
            entry.last_status = Some("error".into());
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            entry.total_failures = entry.total_failures.saturating_add(1);

            let retry = FAILURE_RETRY_MINUTES.saturating_mul(
                1u32 << entry
                    .consecutive_failures
                    .saturating_sub(1)
                    .min(MAX_FAILURE_BACKOFF_SHIFT),
            );
            retry.min(task.cooldown_minutes.max(FAILURE_RETRY_MINUTES))
        };

        entry.next_due_at =
            Some((now + ChronoDuration::minutes(i64::from(next_delay))).to_rfc3339());
        state.updated_at = Some(now.to_rfc3339());
        self.save_state(&state).await
    }

    fn fallback_tasks(&self) -> Vec<HeartbeatTask> {
        self.config
            .message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| {
                vec![HeartbeatTask {
                    key: Self::task_key(
                        "fallback",
                        message,
                        self.default_cooldown_minutes(),
                        0,
                        None,
                    ),
                    text: message.to_string(),
                    cooldown_minutes: self.default_cooldown_minutes(),
                    priority: 0,
                    max_runs: None,
                }]
            })
            .unwrap_or_default()
    }

    fn default_cooldown_minutes(&self) -> u32 {
        self.config
            .interval_minutes
            .max(DEFAULT_MIN_COOLDOWN_MINUTES)
    }

    fn state_file_path(&self) -> PathBuf {
        self.workspace_dir
            .join("state")
            .join("heartbeat_state.json")
    }

    async fn load_state(&self) -> Result<HeartbeatState> {
        let path = self.state_file_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(state) => Ok(state),
                Err(err) => {
                    warn!(
                        "Heartbeat state file is invalid, resetting {}: {err}",
                        path.display()
                    );
                    Ok(HeartbeatState::default())
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HeartbeatState::default()),
            Err(err) => Err(err.into()),
        }
    }

    async fn save_state(&self, state: &HeartbeatState) -> Result<()> {
        let path = self.state_file_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(state)?;
        tokio::fs::write(&tmp, data).await?;
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&path).await;
        }
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Parse tasks from HEARTBEAT.md bullets with optional metadata like:
    /// `- [every=4h] [priority=2] Review calendar`
    fn parse_tasks(content: &str, interval_minutes: u32) -> Vec<HeartbeatTask> {
        content
            .lines()
            .filter_map(|line| Self::parse_task_line(line, interval_minutes))
            .collect()
    }

    fn parse_task_line(line: &str, interval_minutes: u32) -> Option<HeartbeatTask> {
        let trimmed = line.trim();
        let mut rest = trimmed.strip_prefix("- ")?;
        let mut cooldown_minutes = interval_minutes.max(DEFAULT_MIN_COOLDOWN_MINUTES);
        let mut priority = 0;
        let mut max_runs = None;

        while let Some(meta) = rest.strip_prefix('[') {
            let end = meta.find(']')?;
            let token = meta[..end].trim();
            let remaining = meta[end + 1..].trim_start();
            if !Self::apply_task_meta(token, &mut cooldown_minutes, &mut priority, &mut max_runs) {
                break;
            }
            rest = remaining;
        }

        let text = rest.trim();
        if text.is_empty() {
            return None;
        }

        Some(HeartbeatTask {
            key: Self::task_key("task", text, cooldown_minutes, priority, max_runs),
            text: text.to_string(),
            cooldown_minutes,
            priority,
            max_runs,
        })
    }

    fn apply_task_meta(
        token: &str,
        cooldown_minutes: &mut u32,
        priority: &mut i32,
        max_runs: &mut Option<u64>,
    ) -> bool {
        if let Some(value) = token
            .strip_prefix("every=")
            .or_else(|| token.strip_prefix("cooldown="))
        {
            if let Some(minutes) = parse_duration_minutes(value) {
                *cooldown_minutes = minutes.max(1);
            }
            return true;
        }
        if let Some(value) = token.strip_prefix("priority=") {
            if let Ok(parsed) = value.trim().parse::<i32>() {
                *priority = parsed;
            }
            return true;
        }
        if let Some(value) = token.strip_prefix("max_runs=") {
            if let Ok(parsed) = value.trim().parse::<u64>() {
                *max_runs = Some(parsed);
            }
            return true;
        }
        false
    }

    fn task_key(
        prefix: &str,
        text: &str,
        cooldown_minutes: u32,
        priority: i32,
        max_runs: Option<u64>,
    ) -> String {
        format!(
            "{prefix}|{}|{cooldown_minutes}|{priority}|{}",
            text.trim().to_ascii_lowercase(),
            max_runs.unwrap_or(0)
        )
    }

    /// Create a default HEARTBEAT.md if it doesn't exist.
    pub async fn ensure_heartbeat_file(workspace_dir: &Path) -> Result<()> {
        let path = workspace_dir.join("HEARTBEAT.md");
        if !path.exists() {
            let default = "# HEARTBEAT.md\n\n\
                           # Keep this file empty (or with only comments) to skip heartbeat work.\n\
                           # Add one task per line using `- ` bullets.\n\
                           #\n\
                           # Optional metadata prefixes:\n\
                           # - [every=4h] run at most every 4 hours\n\
                           # - [priority=2] higher numbers run first when multiple tasks are due\n\
                           # - [max_runs=1] run once and then stop\n\
                           #\n\
                           # Examples:\n\
                           # - [every=4h] Review my calendar for the next 24 hours\n\
                           # - [every=1d] [priority=2] Check my active repos for stale branches\n\
                           # - [every=30m] [max_runs=1] Remind me to finish onboarding notes\n";
            tokio::fs::write(&path, default).await?;
        }
        Ok(())
    }
}

fn parse_duration_minutes(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let digits_len = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }

    let value = trimmed[..digits_len].parse::<u32>().ok()?;
    let unit = trimmed[digits_len..].trim().to_ascii_lowercase();
    match unit.as_str() {
        "" | "m" | "min" | "mins" | "minute" | "minutes" => Some(value),
        "h" | "hr" | "hrs" | "hour" | "hours" => value.checked_mul(60),
        "d" | "day" | "days" => value.checked_mul(60 * 24),
        _ => None,
    }
}

fn parse_rfc3339(raw: Option<&str>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::NoopObserver;
    use tempfile::tempdir;

    fn test_engine(dir: &Path) -> HeartbeatEngine {
        HeartbeatEngine::new(
            HeartbeatConfig {
                enabled: true,
                interval_minutes: 30,
                message: None,
                target: None,
                to: None,
            },
            dir.to_path_buf(),
            Arc::new(NoopObserver),
        )
    }

    #[test]
    fn parse_duration_minutes_supports_common_units() {
        assert_eq!(parse_duration_minutes("15m"), Some(15));
        assert_eq!(parse_duration_minutes("2h"), Some(120));
        assert_eq!(parse_duration_minutes("1d"), Some(1440));
        assert_eq!(parse_duration_minutes("bad"), None);
    }

    #[test]
    fn parse_tasks_supports_metadata_prefixes() {
        let tasks = HeartbeatEngine::parse_tasks(
            "- [every=4h] [priority=2] [max_runs=1] Review calendar",
            30,
        );
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].text, "Review calendar");
        assert_eq!(tasks[0].cooldown_minutes, 240);
        assert_eq!(tasks[0].priority, 2);
        assert_eq!(tasks[0].max_runs, Some(1));
    }

    #[test]
    fn parse_tasks_defaults_to_human_cooldown_floor() {
        let tasks = HeartbeatEngine::parse_tasks("- Check email", 15);
        assert_eq!(tasks[0].cooldown_minutes, DEFAULT_MIN_COOLDOWN_MINUTES);
    }

    #[tokio::test]
    async fn ensure_heartbeat_file_creates_file() {
        let dir = tempdir().unwrap();
        HeartbeatEngine::ensure_heartbeat_file(dir.path())
            .await
            .unwrap();
        let content = tokio::fs::read_to_string(dir.path().join("HEARTBEAT.md"))
            .await
            .unwrap();
        assert!(content.contains("[every=4h]"));
    }

    #[tokio::test]
    async fn due_tasks_respects_state_and_priority() {
        let dir = tempdir().unwrap();
        tokio::fs::create_dir_all(dir.path().join("state"))
            .await
            .unwrap();
        tokio::fs::write(
            dir.path().join("HEARTBEAT.md"),
            "- [every=4h] [priority=1] Low\n- [every=4h] [priority=3] High\n",
        )
        .await
        .unwrap();
        let engine = test_engine(dir.path());

        let due = engine.due_tasks().await.unwrap();
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].text, "High");

        engine.record_task_result(&due[0], true).await.unwrap();
        let next_due = engine.due_tasks().await.unwrap();
        assert_eq!(next_due.len(), 1);
        assert_eq!(next_due[0].text, "Low");
    }

    #[tokio::test]
    async fn record_task_result_applies_failure_backoff() {
        let dir = tempdir().unwrap();
        let engine = test_engine(dir.path());
        let task = HeartbeatTask {
            key: "task|check email|60|0|0".into(),
            text: "Check email".into(),
            cooldown_minutes: 60,
            priority: 0,
            max_runs: None,
        };

        engine.record_task_result(&task, false).await.unwrap();
        let state_raw = tokio::fs::read_to_string(dir.path().join("state/heartbeat_state.json"))
            .await
            .unwrap();
        let state: HeartbeatState = serde_json::from_str(&state_raw).unwrap();
        let entry = state.tasks.get(&task.key).unwrap();
        let next_due = parse_rfc3339(entry.next_due_at.as_deref()).unwrap();
        let delay = next_due
            .signed_duration_since(parse_rfc3339(entry.last_run_at.as_deref()).unwrap())
            .num_minutes();
        assert_eq!(delay, i64::from(FAILURE_RETRY_MINUTES));
    }

    #[tokio::test]
    async fn due_tasks_respects_max_runs() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("HEARTBEAT.md"), "- [max_runs=1] Run once\n")
            .await
            .unwrap();
        let engine = test_engine(dir.path());
        let due = engine.due_tasks().await.unwrap();
        assert_eq!(due.len(), 1);
        engine.record_task_result(&due[0], true).await.unwrap();
        assert!(engine.due_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fallback_message_becomes_due_task() {
        let dir = tempdir().unwrap();
        let engine = HeartbeatEngine::new(
            HeartbeatConfig {
                enabled: true,
                interval_minutes: 30,
                message: Some("check london time".into()),
                target: None,
                to: None,
            },
            dir.path().to_path_buf(),
            Arc::new(NoopObserver),
        );
        let due = engine.due_tasks().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].text, "check london time");
    }
}
