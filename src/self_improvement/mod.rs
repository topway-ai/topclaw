use crate::config::schema::SelfImprovementConfig;
use crate::config::Config;
use crate::cron::{self, add_agent_job, CronJob, CronJobPatch, Schedule, SessionTarget};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SELF_IMPROVEMENT_JOB_NAME: &str = "__self_improve_queue";
const CANDIDATE_BRANCH_NAME: &str = "self-improve/candidate";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SelfImprovementTaskStatus {
    #[default]
    Queued,
    InProgress,
    Blocked,
    PrOpened,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelfImprovementTask {
    pub id: String,
    pub title: String,
    pub problem: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub requested_channel: Option<String>,
    #[serde(default)]
    pub status: SelfImprovementTaskStatus,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub branch_name: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub validation_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelfImprovementState {
    #[serde(default)]
    pub tasks: Vec<SelfImprovementTask>,
    #[serde(default)]
    pub candidate_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitReadiness {
    pub ready: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub repository_path: Option<String>,
    #[serde(default)]
    pub candidate_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncOutcome {
    pub action: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
}

pub struct SelfImprovementManager {
    workspace_dir: PathBuf,
}

impl SelfImprovementManager {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.workspace_dir
            .join("state")
            .join("self_improvement_tasks.json")
    }

    pub fn candidate_path(&self) -> PathBuf {
        self.workspace_dir
            .join("state")
            .join("self-improvement")
            .join("candidate")
    }

    pub async fn load_state(&self) -> Result<SelfImprovementState> {
        let path = self.state_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(SelfImprovementState::default())
            }
            Err(err) => Err(err.into()),
        }
    }

    pub async fn save_state(&self, state: &SelfImprovementState) -> Result<()> {
        let path = self.state_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(state)?;
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    pub async fn enqueue_task(
        &self,
        config: &Config,
        title: &str,
        problem: &str,
        evidence: Option<String>,
        requested_by: Option<String>,
        requested_channel: Option<String>,
    ) -> Result<SelfImprovementTask> {
        let mut state = self.load_state().await?;
        let now = Utc::now().to_rfc3339();
        let task_id = uuid::Uuid::new_v4().to_string();
        let branch_name = Some(default_branch_name(
            &config.self_improvement,
            requested_by.as_deref(),
            title,
            &task_id,
        ));
        let task = SelfImprovementTask {
            id: task_id,
            title: title.trim().to_string(),
            problem: problem.trim().to_string(),
            evidence,
            requested_by,
            requested_channel,
            status: SelfImprovementTaskStatus::Queued,
            created_at: now.clone(),
            updated_at: now,
            branch_name,
            pr_url: None,
            last_error: None,
            validation_summary: None,
        };
        state.tasks.push(task.clone());
        self.save_state(&state).await?;
        Ok(task)
    }

    pub async fn update_task(
        &self,
        task_id: &str,
        status: Option<SelfImprovementTaskStatus>,
        branch_name: Option<String>,
        pr_url: Option<String>,
        last_error: Option<String>,
        validation_summary: Option<String>,
    ) -> Result<SelfImprovementTask> {
        let mut state = self.load_state().await?;
        let task = state
            .tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .with_context(|| format!("self-improvement task '{task_id}' not found"))?;

        if let Some(status) = status {
            task.status = status;
        }
        if let Some(branch_name) = branch_name {
            task.branch_name = Some(branch_name);
        }
        if let Some(pr_url) = pr_url {
            task.pr_url = Some(pr_url);
        }
        if let Some(last_error) = last_error {
            task.last_error = Some(last_error);
        }
        if let Some(validation_summary) = validation_summary {
            task.validation_summary = Some(validation_summary);
        }
        task.updated_at = Utc::now().to_rfc3339();

        let updated = task.clone();
        self.save_state(&state).await?;
        Ok(updated)
    }

    pub fn next_runnable_task<'a>(
        &self,
        state: &'a SelfImprovementState,
    ) -> Option<&'a SelfImprovementTask> {
        state.tasks.iter().find(|task| {
            matches!(
                task.status,
                SelfImprovementTaskStatus::Queued | SelfImprovementTaskStatus::InProgress
            )
        })
    }
}

fn command_is_allowlisted(allowed_commands: &[String], command: &str) -> bool {
    if allowed_commands.iter().any(|entry| entry == "*") {
        return true;
    }
    allowed_commands
        .iter()
        .map(|entry| entry.trim())
        .any(|entry| {
            entry == command
                || Path::new(entry)
                    .file_name()
                    .is_some_and(|name| name == command)
        })
}

fn run_command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("{program} {} failed: {}", args.join(" "), stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn stable_repo_path(cfg: &SelfImprovementConfig) -> Option<PathBuf> {
    let path = cfg.repository_path.as_deref()?.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

pub async fn check_git_readiness(config: &Config) -> GitReadiness {
    let manager = SelfImprovementManager::new(&config.workspace_dir);
    let Some(repository_path) = stable_repo_path(&config.self_improvement) else {
        return GitReadiness {
            ready: false,
            reason: Some("self_improvement.repository_path is not configured".to_string()),
            repository_path: None,
            candidate_path: Some(manager.candidate_path().display().to_string()),
        };
    };

    if !config.self_improvement.enabled {
        return GitReadiness {
            ready: false,
            reason: Some("self-improvement automation is disabled".to_string()),
            repository_path: Some(repository_path.display().to_string()),
            candidate_path: Some(manager.candidate_path().display().to_string()),
        };
    }

    if !config.cron.enabled {
        return GitReadiness {
            ready: false,
            reason: Some("cron scheduler is disabled".to_string()),
            repository_path: Some(repository_path.display().to_string()),
            candidate_path: Some(manager.candidate_path().display().to_string()),
        };
    }

    for command in ["git", "cargo"] {
        if !command_is_allowlisted(&config.autonomy.allowed_commands, command) {
            return GitReadiness {
                ready: false,
                reason: Some(format!(
                    "required command '{command}' is not allowlisted for automation"
                )),
                repository_path: Some(repository_path.display().to_string()),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
    }
    if config.self_improvement.auto_open_draft_pr
        && !command_is_allowlisted(&config.autonomy.allowed_commands, "gh")
    {
        return GitReadiness {
            ready: false,
            reason: Some(
                "required command 'gh' is not allowlisted for draft PR creation".to_string(),
            ),
            repository_path: Some(repository_path.display().to_string()),
            candidate_path: Some(manager.candidate_path().display().to_string()),
        };
    }

    if !repository_path.exists() {
        return GitReadiness {
            ready: false,
            reason: Some("configured self-improvement repository_path does not exist".to_string()),
            repository_path: Some(repository_path.display().to_string()),
            candidate_path: Some(manager.candidate_path().display().to_string()),
        };
    }

    let repo = repository_path.display().to_string();
    let checks = vec![
        run_command("git", &["--version"]),
        run_command("cargo", &["--version"]),
        run_command("git", &["-C", &repo, "rev-parse", "--is-inside-work-tree"]),
        run_command("git", &["-C", &repo, "config", "--get", "user.name"]),
        run_command("git", &["-C", &repo, "config", "--get", "user.email"]),
        run_command("git", &["-C", &repo, "remote", "get-url", "origin"]),
        run_command("git", &["-C", &repo, "status", "--porcelain"]),
    ];
    for check in checks.iter().take(6) {
        if let Err(error) = check {
            return GitReadiness {
                ready: false,
                reason: Some(error.to_string()),
                repository_path: Some(repo),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
    }
    match checks.get(6) {
        Some(Ok(output)) if output.trim().is_empty() => {}
        Some(Ok(_)) => {
            return GitReadiness {
                ready: false,
                reason: Some("stable repository worktree is not clean".to_string()),
                repository_path: Some(repo),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
        Some(Err(error)) => {
            return GitReadiness {
                ready: false,
                reason: Some(error.to_string()),
                repository_path: Some(repo),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
        None => {
            return GitReadiness {
                ready: false,
                reason: Some("internal self-improvement readiness check failed".to_string()),
                repository_path: Some(repo),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
    }

    if config.self_improvement.auto_open_draft_pr {
        if let Err(error) = run_command("gh", &["auth", "status"]) {
            return GitReadiness {
                ready: false,
                reason: Some(format!("gh auth status failed: {error}")),
                repository_path: Some(repository_path.display().to_string()),
                candidate_path: Some(manager.candidate_path().display().to_string()),
            };
        }
    }

    GitReadiness {
        ready: true,
        reason: None,
        repository_path: Some(repository_path.display().to_string()),
        candidate_path: Some(manager.candidate_path().display().to_string()),
    }
}

fn sanitize_component(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !last_dash {
                out.push(mapped);
            }
            last_dash = true;
        } else {
            out.push(mapped);
            last_dash = false;
        }
        if out.len() >= 32 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn default_branch_name(
    cfg: &SelfImprovementConfig,
    requested_by: Option<&str>,
    title: &str,
    task_id: &str,
) -> String {
    let owner = sanitize_component(requested_by.unwrap_or("topclaw_user"));
    let slug = sanitize_component(title);
    let short_id = &task_id[..8.min(task_id.len())];
    format!(
        "{}/{}/{}-{}",
        sanitize_component(&cfg.branch_prefix),
        if owner.is_empty() {
            "topclaw_user"
        } else {
            &owner
        },
        if slug.is_empty() {
            "self-improve"
        } else {
            &slug
        },
        short_id
    )
}

fn build_task_prompt(
    task: &SelfImprovementTask,
    readiness: &GitReadiness,
    cfg: &SelfImprovementConfig,
) -> String {
    let repo = readiness.repository_path.as_deref().unwrap_or("(unset)");
    let candidate = readiness.candidate_path.as_deref().unwrap_or("(unset)");
    let branch_name = task
        .branch_name
        .as_deref()
        .unwrap_or("users/topclaw_user/self-improve");
    let mut prompt = format!(
        "[Self-Improvement Queue]\n\
         You are processing one explicit TopClaw improvement task.\n\n\
         Task ID: {task_id}\n\
         Title: {title}\n\
         Problem: {problem}\n",
        task_id = task.id,
        title = task.title,
        problem = task.problem
    );
    if let Some(evidence) = task.evidence.as_deref() {
        prompt.push_str(&format!("Evidence: {evidence}\n"));
    }
    prompt.push_str(&format!(
        "\nStable repository: `{repo}`\n\
         Candidate worktree: `{candidate}`\n\
         Target branch: `{branch_name}`\n\n\
         Rules:\n\
         1. Work only in the candidate worktree, not the stable repository.\n\
         2. Focus only on this one TopClaw task.\n\
         3. Start by using `self_improvement_task` to mark the task `in_progress`.\n\
         4. Implement the smallest effective fix.\n\
         5. Run validation from the candidate worktree with `cargo fmt --all -- --check`, `cargo check --locked`, and the most relevant tests.\n\
         6. If you are blocked, use `self_improvement_task` to mark the task `blocked` with the reason.\n"
    ));
    if cfg.auto_push_branch {
        prompt.push_str(
            "7. If the fix is effective and validation passes, use `self_improvement_task` action `publish_pr` to commit, push, and optionally open the draft PR automatically.\n",
        );
    } else {
        prompt.push_str(
            "7. If the fix is effective and validation passes, use `self_improvement_task` to mark the task `completed` with a validation summary.\n",
        );
    }
    prompt
}

fn find_self_improvement_jobs(config: &Config) -> Result<Vec<CronJob>> {
    Ok(cron::list_jobs(config)?
        .into_iter()
        .filter(|job| job.name.as_deref() == Some(SELF_IMPROVEMENT_JOB_NAME))
        .collect())
}

fn ensure_candidate_worktree(repository_path: &str, candidate_path: &Path) -> Result<()> {
    if let Some(parent) = candidate_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let candidate = candidate_path.display().to_string();
    if candidate_path.exists() {
        let _ = run_command(
            "git",
            &[
                "-C",
                repository_path,
                "worktree",
                "remove",
                "--force",
                &candidate,
            ],
        );
        let _ = std::fs::remove_dir_all(candidate_path);
    }
    let _ = run_command(
        "git",
        &[
            "-C",
            repository_path,
            "worktree",
            "add",
            "--force",
            "-B",
            CANDIDATE_BRANCH_NAME,
            &candidate,
            "HEAD",
        ],
    )?;
    Ok(())
}

pub async fn sync_scheduled_job(config: &Config) -> Result<SyncOutcome> {
    let manager = SelfImprovementManager::new(&config.workspace_dir);
    let mut state = manager.load_state().await?;
    let task = manager.next_runnable_task(&state).cloned();
    let mut jobs = find_self_improvement_jobs(config)?;

    if task.is_none() {
        for job in jobs {
            let _ = cron::remove_job(config, &job.id);
        }
        return Ok(SyncOutcome {
            action: "removed".to_string(),
            reason: Some("no queued self-improvement tasks".to_string()),
            task_id: None,
            job_id: None,
        });
    }

    let readiness = check_git_readiness(config).await;
    if !readiness.ready {
        for job in jobs {
            let _ = cron::remove_job(config, &job.id);
        }
        return Ok(SyncOutcome {
            action: "idle".to_string(),
            reason: readiness.reason,
            task_id: task.as_ref().map(|task| task.id.clone()),
            job_id: None,
        });
    }

    let task = task.expect("task checked above");
    if state.candidate_task_id.as_deref() != Some(task.id.as_str()) {
        ensure_candidate_worktree(
            readiness.repository_path.as_deref().unwrap_or_default(),
            Path::new(readiness.candidate_path.as_deref().unwrap_or_default()),
        )?;
        state.candidate_task_id = Some(task.id.clone());
        manager.save_state(&state).await?;
    }

    let prompt = build_task_prompt(&task, &readiness, &config.self_improvement);
    let schedule = Schedule::Every {
        every_ms: u64::from(config.self_improvement.interval_minutes.max(5)) * 60_000,
    };

    let job = if let Some(existing) = jobs.pop() {
        let updated = cron::update_job(
            config,
            &existing.id,
            CronJobPatch {
                prompt: Some(prompt),
                schedule: Some(schedule),
                enabled: Some(true),
                ..CronJobPatch::default()
            },
        )?;
        for extra in jobs {
            let _ = cron::remove_job(config, &extra.id);
        }
        updated
    } else {
        add_agent_job(
            config,
            Some(SELF_IMPROVEMENT_JOB_NAME.to_string()),
            schedule,
            &prompt,
            SessionTarget::Isolated,
            None,
            None,
            false,
        )?
    };

    Ok(SyncOutcome {
        action: "scheduled".to_string(),
        reason: None,
        task_id: Some(task.id),
        job_id: Some(job.id),
    })
}

pub async fn publish_draft_pr_for_task(
    config: &Config,
    task_id: &str,
    commit_message: &str,
    pr_title: &str,
    pr_body: &str,
    validation_summary: Option<String>,
) -> Result<SelfImprovementTask> {
    let manager = SelfImprovementManager::new(&config.workspace_dir);
    let state = manager.load_state().await?;
    let task = state
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .cloned()
        .with_context(|| format!("self-improvement task '{task_id}' not found"))?;
    let readiness = check_git_readiness(config).await;
    if !readiness.ready {
        anyhow::bail!(
            "self-improvement publishing blocked: {}",
            readiness
                .reason
                .unwrap_or_else(|| "git readiness failed".to_string())
        );
    }

    let candidate = readiness
        .candidate_path
        .as_deref()
        .context("candidate path missing from readiness")?;
    let branch = task.branch_name.clone().unwrap_or_else(|| {
        default_branch_name(
            &config.self_improvement,
            task.requested_by.as_deref(),
            &task.title,
            &task.id,
        )
    });

    run_command("git", &["-C", candidate, "checkout", "-B", &branch])?;
    run_command("git", &["-C", candidate, "add", "-A"])?;
    run_command("git", &["-C", candidate, "commit", "-m", commit_message])?;
    if config.self_improvement.auto_push_branch {
        run_command("git", &["-C", candidate, "push", "-u", "origin", &branch])?;
    }

    let mut pr_url = None;
    if config.self_improvement.auto_open_draft_pr {
        pr_url = Some(run_command(
            "gh",
            &[
                "pr", "create", "--draft", "--base", "main", "--head", &branch, "--title",
                pr_title, "--body", pr_body,
            ],
        )?);
    }

    manager
        .update_task(
            task_id,
            Some(if pr_url.is_some() {
                SelfImprovementTaskStatus::PrOpened
            } else {
                SelfImprovementTaskStatus::Completed
            }),
            Some(branch),
            pr_url,
            None,
            validation_summary,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");
        config.self_improvement.enabled = true;
        config.self_improvement.repository_path =
            Some(tmp.path().join("repo").display().to_string());
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    #[tokio::test]
    async fn enqueue_and_update_task_round_trip() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let manager = SelfImprovementManager::new(&config.workspace_dir);

        let task = manager
            .enqueue_task(
                &config,
                "Improve onboarding",
                "The onboarding flow is confusing",
                Some("user transcript".into()),
                Some("frank".into()),
                Some("telegram".into()),
            )
            .await
            .unwrap();
        assert_eq!(task.status, SelfImprovementTaskStatus::Queued);
        assert!(task.branch_name.as_deref().unwrap_or("").contains("frank"));

        let updated = manager
            .update_task(
                &task.id,
                Some(SelfImprovementTaskStatus::InProgress),
                None,
                None,
                None,
                Some("cargo check".into()),
            )
            .await
            .unwrap();
        assert_eq!(updated.status, SelfImprovementTaskStatus::InProgress);
        assert_eq!(updated.validation_summary.as_deref(), Some("cargo check"));
    }

    #[test]
    fn default_branch_name_is_user_scoped() {
        let cfg = SelfImprovementConfig::default();
        let branch = default_branch_name(
            &cfg,
            Some("Frank2085"),
            "Fix onboarding prompts",
            "abc12345xyz",
        );
        assert!(branch.starts_with("users/frank2085/"));
        assert!(branch.contains("fix-onboarding-prompts"));
    }

    #[test]
    fn build_task_prompt_mentions_candidate_and_publish_flow() {
        let task = SelfImprovementTask {
            id: "task-1".into(),
            title: "Fix onboarding".into(),
            problem: "onboarding is confusing".into(),
            evidence: Some("chat transcript".into()),
            branch_name: Some("users/topclaw_user/fix-onboarding-task1".into()),
            ..SelfImprovementTask::default()
        };
        let readiness = GitReadiness {
            ready: true,
            reason: None,
            repository_path: Some("/repo".into()),
            candidate_path: Some("/ws/state/self-improvement/candidate".into()),
        };
        let prompt = build_task_prompt(&task, &readiness, &SelfImprovementConfig::default());
        assert!(prompt.contains("Candidate worktree"));
        assert!(prompt.contains("self_improvement_task"));
        assert!(prompt.contains("publish_pr"));
    }
}
