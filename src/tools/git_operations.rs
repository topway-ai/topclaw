use super::traits::{Tool, ToolResult};
use crate::security::{AutonomyLevel, SecurityPolicy};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Git operations tool for structured repository management.
/// Provides safe, parsed git operations with JSON output.
pub struct GitOperationsTool {
    security: Arc<SecurityPolicy>,
    workspace_dir: std::path::PathBuf,
}

impl GitOperationsTool {
    pub fn new(security: Arc<SecurityPolicy>, workspace_dir: std::path::PathBuf) -> Self {
        Self {
            security,
            workspace_dir,
        }
    }

    /// Sanitize git arguments to prevent injection attacks
    fn sanitize_git_args(&self, args: &str) -> anyhow::Result<Vec<String>> {
        let mut result = Vec::new();
        for arg in args.split_whitespace() {
            // Block dangerous git options that could lead to command injection
            let arg_lower = arg.to_lowercase();
            if arg_lower.starts_with("--exec=")
                || arg_lower.starts_with("--upload-pack=")
                || arg_lower.starts_with("--receive-pack=")
                || arg_lower.starts_with("--pager=")
                || arg_lower.starts_with("--editor=")
                || arg_lower == "--no-verify"
                || arg_lower.contains("$(")
                || arg_lower.contains('`')
                || arg.contains('|')
                || arg.contains(';')
                || arg.contains('>')
            {
                anyhow::bail!("Blocked potentially dangerous git argument: {arg}");
            }
            // Block `-c` config injection (exact match or `-c=...` prefix).
            // This must not false-positive on `--cached` or `-cached`.
            if arg_lower == "-c" || arg_lower.starts_with("-c=") {
                anyhow::bail!("Blocked potentially dangerous git argument: {arg}");
            }
            result.push(arg.to_string());
        }
        Ok(result)
    }

    /// Check if an operation requires write access
    fn requires_write_access(&self, operation: &str) -> bool {
        matches!(
            operation,
            "commit" | "add" | "checkout" | "stash" | "reset" | "revert" | "clone"
        )
    }

    /// Validate that a clone URL uses HTTPS and is not a smuggling vector.
    fn validate_clone_url(url: &str) -> anyhow::Result<()> {
        let url = url.trim();
        if url.is_empty() {
            anyhow::bail!("Clone URL must not be empty");
        }
        // Only allow HTTPS URLs to prevent SSH key leakage and git:// protocol.
        if !url.starts_with("https://") {
            anyhow::bail!(
                "Clone URL must use HTTPS (e.g. https://github.com/org/repo.git). \
                 SSH and git:// URLs are blocked to prevent credential leakage."
            );
        }
        // Block injection patterns in the URL.
        if url.contains("$(") || url.contains('`') || url.contains('\0') || url.contains('\n') {
            anyhow::bail!("Blocked potentially dangerous clone URL");
        }
        // Block URLs that contain shell metacharacters or whitespace.
        // Whitespace in URLs is invalid and would cause git to fail with a
        // confusing error, so we reject early with a clear message.
        if url.contains('|')
            || url.contains(';')
            || url.contains('&')
            || url.contains('>')
            || url.contains('<')
            || url.contains(' ')
        {
            anyhow::bail!("Clone URL contains invalid characters");
        }
        Ok(())
    }

    /// Validate the clone destination directory against security policy.
    fn validate_clone_destination(&self, dest: &str) -> anyhow::Result<()> {
        let dest = dest.trim();
        if dest.is_empty() {
            anyhow::bail!("Clone destination must not be empty");
        }
        // Block path traversal.
        if dest.contains("..") || dest.contains('\0') {
            anyhow::bail!("Clone destination contains invalid path components");
        }
        // Block shell metacharacters.
        if dest.contains('$')
            || dest.contains('`')
            || dest.contains('|')
            || dest.contains(';')
            || dest.contains('&')
            || dest.contains('>')
            || dest.contains('<')
        {
            anyhow::bail!("Clone destination contains invalid characters");
        }
        // Resolve destination relative to workspace_dir.
        let resolved = if std::path::Path::new(dest).is_absolute() {
            std::path::PathBuf::from(dest)
        } else {
            self.workspace_dir.join(dest)
        };

        // Check against security policy path rules.
        if !self.security.is_resolved_path_allowed(&resolved) {
            anyhow::bail!(
                "Clone destination blocked by security policy: {}. {}",
                resolved.display(),
                self.security.resolved_path_violation_message(&resolved)
            );
        }
        Ok(())
    }

    async fn run_git_command(&self, args: &[&str]) -> anyhow::Result<String> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(&self.workspace_dir)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Git command failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn git_status(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = self
            .run_git_command(&["status", "--porcelain=2", "--branch"])
            .await?;

        // Parse git status output into structured format
        let mut result = serde_json::Map::new();
        let mut branch = String::new();
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for line in output.lines() {
            if line.starts_with("# branch.head ") {
                branch = line.trim_start_matches("# branch.head ").to_string();
            } else if let Some(rest) = line.strip_prefix("1 ") {
                // Ordinary changed entry
                let mut parts = rest.splitn(3, ' ');
                if let (Some(staging), Some(path)) = (parts.next(), parts.next()) {
                    if !staging.is_empty() {
                        let status_char = staging.chars().next().unwrap_or(' ');
                        if status_char != '.' && status_char != ' ' {
                            staged.push(json!({"path": path, "status": status_char}));
                        }
                        let status_char = staging.chars().nth(1).unwrap_or(' ');
                        if status_char != '.' && status_char != ' ' {
                            unstaged.push(json!({"path": path, "status": status_char}));
                        }
                    }
                }
            } else if let Some(rest) = line.strip_prefix("? ") {
                untracked.push(rest.to_string());
            }
        }

        result.insert("branch".to_string(), json!(branch));
        result.insert("staged".to_string(), json!(staged));
        result.insert("unstaged".to_string(), json!(unstaged));
        result.insert("untracked".to_string(), json!(untracked));
        result.insert(
            "clean".to_string(),
            json!(staged.is_empty() && unstaged.is_empty() && untracked.is_empty()),
        );

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result).unwrap_or_default(),
            error: None,
        })
    }

    async fn git_diff(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let files = args.get("files").and_then(|v| v.as_str()).unwrap_or(".");
        let cached = args
            .get("cached")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Validate files argument against injection patterns
        self.sanitize_git_args(files)?;

        let mut git_args = vec!["diff", "--unified=3"];
        if cached {
            git_args.push("--cached");
        }
        git_args.push("--");
        git_args.push(files);

        let output = self.run_git_command(&git_args).await?;

        // Parse diff into structured hunks
        let mut result = serde_json::Map::new();
        let mut hunks = Vec::new();
        let mut current_file = String::new();
        let mut current_hunk = serde_json::Map::new();
        let mut lines = Vec::new();

        for line in output.lines() {
            if line.starts_with("diff --git ") {
                if !lines.is_empty() {
                    current_hunk.insert("lines".to_string(), json!(lines));
                    if !current_hunk.is_empty() {
                        hunks.push(serde_json::Value::Object(current_hunk.clone()));
                    }
                    lines = Vec::new();
                    current_hunk = serde_json::Map::new();
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    current_file = parts[3].trim_start_matches("b/").to_string();
                    current_hunk.insert("file".to_string(), json!(current_file));
                }
            } else if line.starts_with("@@ ") {
                if !lines.is_empty() {
                    current_hunk.insert("lines".to_string(), json!(lines));
                    if !current_hunk.is_empty() {
                        hunks.push(serde_json::Value::Object(current_hunk.clone()));
                    }
                    lines = Vec::new();
                    current_hunk = serde_json::Map::new();
                    current_hunk.insert("file".to_string(), json!(current_file));
                }
                current_hunk.insert("header".to_string(), json!(line));
            } else if !line.is_empty() {
                lines.push(json!({
                    "text": line,
                    "type": if line.starts_with('+') { "add" }
                           else if line.starts_with('-') { "delete" }
                           else { "context" }
                }));
            }
        }

        if !lines.is_empty() {
            current_hunk.insert("lines".to_string(), json!(lines));
            if !current_hunk.is_empty() {
                hunks.push(serde_json::Value::Object(current_hunk));
            }
        }

        result.insert("hunks".to_string(), json!(hunks));
        result.insert("file_count".to_string(), json!(hunks.len()));

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result).unwrap_or_default(),
            error: None,
        })
    }

    async fn git_log(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let limit_raw = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
        let limit = usize::try_from(limit_raw).unwrap_or(usize::MAX).min(1000);
        let limit_str = limit.to_string();

        let output = self
            .run_git_command(&[
                "log",
                &format!("-{limit_str}"),
                "--pretty=format:%H|%an|%ae|%ad|%s",
                "--date=iso",
            ])
            .await?;

        let mut commits = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 5 {
                commits.push(json!({
                    "hash": parts[0],
                    "author": parts[1],
                    "email": parts[2],
                    "date": parts[3],
                    "message": parts[4]
                }));
            }
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({ "commits": commits }))
                .unwrap_or_default(),
            error: None,
        })
    }

    async fn git_branch(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let output = self
            .run_git_command(&["branch", "--format=%(refname:short)|%(HEAD)"])
            .await?;

        let mut branches = Vec::new();
        let mut current = String::new();

        for line in output.lines() {
            if let Some((name, head)) = line.split_once('|') {
                let is_current = head == "*";
                if is_current {
                    current = name.to_string();
                }
                branches.push(json!({
                    "name": name,
                    "current": is_current
                }));
            }
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "current": current,
                "branches": branches
            }))
            .unwrap_or_default(),
            error: None,
        })
    }

    fn truncate_commit_message(message: &str) -> String {
        if message.chars().count() > 2000 {
            format!("{}...", message.chars().take(1997).collect::<String>())
        } else {
            message.to_string()
        }
    }

    async fn git_commit(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter"))?;

        // Sanitize commit message
        let sanitized = message
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        if sanitized.is_empty() {
            anyhow::bail!("Commit message cannot be empty");
        }

        // Limit message length
        let message = Self::truncate_commit_message(&sanitized);

        let output = self.run_git_command(&["commit", "-m", &message]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Committed: {message}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Commit failed: {e}")),
            }),
        }
    }

    async fn git_add(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let paths = args
            .get("paths")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'paths' parameter"))?;

        // Validate paths against injection patterns
        self.sanitize_git_args(paths)?;

        let output = self.run_git_command(&["add", "--", paths]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Staged: {paths}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Add failed: {e}")),
            }),
        }
    }

    async fn git_checkout(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let branch = args
            .get("branch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'branch' parameter"))?;

        // Sanitize branch name
        let sanitized = self.sanitize_git_args(branch)?;

        if sanitized.is_empty() || sanitized.len() > 1 {
            anyhow::bail!("Invalid branch specification");
        }

        let branch_name = &sanitized[0];

        // Block dangerous branch names
        if branch_name.contains('@') || branch_name.contains('^') || branch_name.contains('~') {
            anyhow::bail!("Branch name contains invalid characters");
        }

        let output = self.run_git_command(&["checkout", branch_name]).await;

        match output {
            Ok(_) => Ok(ToolResult {
                success: true,
                output: format!("Switched to branch: {branch_name}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Checkout failed: {e}")),
            }),
        }
    }

    async fn git_stash(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("push");

        let output = match action {
            "push" | "save" => {
                self.run_git_command(&["stash", "push", "-m", "auto-stash"])
                    .await
            }
            "pop" => self.run_git_command(&["stash", "pop"]).await,
            "list" => self.run_git_command(&["stash", "list"]).await,
            "drop" => {
                let index_raw = args.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                let index = i32::try_from(index_raw)
                    .map_err(|_| anyhow::anyhow!("stash index too large: {index_raw}"))?;
                self.run_git_command(&["stash", "drop", &format!("stash@{{{index}}}")])
                    .await
            }
            _ => anyhow::bail!("Unknown stash action: {action}. Use: push, pop, list, drop"),
        };

        match output {
            Ok(out) => Ok(ToolResult {
                success: true,
                output: out,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Stash {action} failed: {e}")),
            }),
        }
    }

    async fn git_clone(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'url' parameter".into()),
                })
            }
        };

        // Validate URL (HTTPS-only, no injection)
        if let Err(e) = Self::validate_clone_url(url) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            });
        }

        // Determine destination directory
        let dest = args
            .get("destination")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // When no destination is specified, git derives the directory name from the URL
        // and clones into the current working directory (which is workspace_dir, as set
        // in run_git_command). This is safe because workspace_dir is already validated
        // by the security policy at startup. When a destination IS specified, we must
        // validate it explicitly.
        if !dest.is_empty() {
            if let Err(e) = self.validate_clone_destination(dest) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                });
            }
        }

        // Depth (shallow clone by default for efficiency)
        let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1);
        let depth_str = depth.to_string();

        // Build git clone arguments
        let mut git_args = vec!["clone", "--depth", &depth_str];

        // Optional branch
        let branch = args.get("branch").and_then(|v| v.as_str()).unwrap_or("");
        let branch_sanitized;
        if !branch.is_empty() {
            let sanitized = self.sanitize_git_args(branch)?;
            if sanitized.len() != 1 {
                anyhow::bail!("Invalid branch specification");
            }
            branch_sanitized = sanitized[0].clone();
            git_args.push("--branch");
            git_args.push(&branch_sanitized);
        }

        git_args.push(url);

        // Add destination if specified
        let dest_owned;
        if !dest.is_empty() {
            // Resolve relative paths against workspace_dir
            if std::path::Path::new(dest).is_absolute() {
                dest_owned = dest.to_string();
            } else {
                dest_owned = self.workspace_dir.join(dest).to_string_lossy().into_owned();
            }
            git_args.push(&dest_owned);
        }

        let output = self.run_git_command(&git_args).await;

        match output {
            Ok(_) => {
                let result = serde_json::json!({
                    "url": url,
                    "destination": if dest.is_empty() { "(auto-derived from URL)" } else { dest },
                    "depth": depth,
                    "branch": if branch.is_empty() { "(default)" } else { branch },
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&result).unwrap_or_default(),
                    error: None,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Clone failed: {e}")),
            }),
        }
    }
}

#[async_trait]
impl Tool for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }

    fn description(&self) -> &str {
        "Perform structured Git operations (status, diff, log, branch, commit, add, checkout, stash, clone). Provides parsed JSON output and integrates with security policy for autonomy controls. Clone uses HTTPS-only and shallow depth by default."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "diff", "log", "branch", "commit", "add", "checkout", "stash", "clone"],
                    "description": "Git operation to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for 'commit' operation)"
                },
                "paths": {
                    "type": "string",
                    "description": "File paths to stage (for 'add' operation)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name. For 'checkout': the branch to switch to. For 'clone': the branch to clone (optional)."
                },
                "files": {
                    "type": "string",
                    "description": "File or path to diff (for 'diff' operation, default: '.')"
                },
                "cached": {
                    "type": "boolean",
                    "description": "Show staged changes (for 'diff' operation)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of log entries (for 'log' operation, default: 10)"
                },
                "action": {
                    "type": "string",
                    "enum": ["push", "pop", "list", "drop"],
                    "description": "Stash action (for 'stash' operation)"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index (for 'stash' with 'drop' action)"
                },
                "url": {
                    "type": "string",
                    "description": "Repository URL to clone (for 'clone' operation). Must use HTTPS."
                },
                "destination": {
                    "type": "string",
                    "description": "Target directory for clone (for 'clone' operation). Relative to workspace_dir. Optional — if omitted, git derives the name from the URL."
                },
                "depth": {
                    "type": "integer",
                    "description": "Clone depth / shallow clone (for 'clone' operation, default: 1)"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let operation = match args.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'operation' parameter".into()),
                });
            }
        };

        // Check if we're in a git repository (skip for clone — it creates one)
        if operation != "clone" {
            if !self.workspace_dir.join(".git").exists() {
                // Try to find .git in parent directories
                let mut current_dir = self.workspace_dir.as_path();
                let mut found_git = false;
                while current_dir.parent().is_some() {
                    if current_dir.join(".git").exists() {
                        found_git = true;
                        break;
                    }
                    current_dir = current_dir.parent().unwrap();
                }

                if !found_git {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("Not in a git repository".into()),
                    });
                }
            }
        }

        // Check autonomy level for write operations
        if self.requires_write_access(operation) {
            if !self.security.can_act() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "Action blocked: git write operations require higher autonomy level".into(),
                    ),
                });
            }

            match self.security.autonomy {
                AutonomyLevel::ReadOnly => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("Action blocked: read-only mode".into()),
                    });
                }
                AutonomyLevel::Supervised | AutonomyLevel::Full => {}
            }
        }

        // Record action for rate limiting
        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        // Execute the requested operation
        match operation {
            "status" => self.git_status(args).await,
            "diff" => self.git_diff(args).await,
            "log" => self.git_log(args).await,
            "branch" => self.git_branch(args).await,
            "commit" => self.git_commit(args).await,
            "add" => self.git_add(args).await,
            "checkout" => self.git_checkout(args).await,
            "stash" => self.git_stash(args).await,
            "clone" => self.git_clone(args).await,
            _ => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown operation: {operation}")),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecurityPolicy;
    use tempfile::TempDir;

    fn test_tool(dir: &std::path::Path) -> GitOperationsTool {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        });
        GitOperationsTool::new(security, dir.to_path_buf())
    }

    #[test]
    fn sanitize_git_blocks_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Should block dangerous arguments
        assert!(tool.sanitize_git_args("--exec=rm -rf /").is_err());
        assert!(tool.sanitize_git_args("$(echo pwned)").is_err());
        assert!(tool.sanitize_git_args("`malicious`").is_err());
        assert!(tool.sanitize_git_args("arg | cat").is_err());
        assert!(tool.sanitize_git_args("arg; rm file").is_err());
    }

    #[test]
    fn sanitize_git_blocks_pager_editor_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("--pager=less").is_err());
        assert!(tool.sanitize_git_args("--editor=vim").is_err());
    }

    #[test]
    fn sanitize_git_blocks_config_injection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Exact `-c` flag (config injection)
        assert!(tool.sanitize_git_args("-c core.sshCommand=evil").is_err());
        assert!(tool.sanitize_git_args("-c=core.pager=less").is_err());
    }

    #[test]
    fn sanitize_git_blocks_no_verify() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("--no-verify").is_err());
    }

    #[test]
    fn sanitize_git_blocks_redirect_in_args() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.sanitize_git_args("file.txt > /tmp/out").is_err());
    }

    #[test]
    fn sanitize_git_cached_not_blocked() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // --cached must NOT be blocked by the `-c` check
        assert!(tool.sanitize_git_args("--cached").is_ok());
        // Other safe flags starting with -c prefix
        assert!(tool.sanitize_git_args("-cached").is_ok());
    }

    #[test]
    fn sanitize_git_allows_safe() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Should allow safe arguments
        assert!(tool.sanitize_git_args("main").is_ok());
        assert!(tool.sanitize_git_args("feature/test-branch").is_ok());
        assert!(tool.sanitize_git_args("--cached").is_ok());
        assert!(tool.sanitize_git_args("src/main.rs").is_ok());
        assert!(tool.sanitize_git_args(".").is_ok());
    }

    #[test]
    fn requires_write_detection() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        assert!(tool.requires_write_access("commit"));
        assert!(tool.requires_write_access("add"));
        assert!(tool.requires_write_access("checkout"));
        assert!(tool.requires_write_access("clone"));

        assert!(!tool.requires_write_access("status"));
        assert!(!tool.requires_write_access("diff"));
        assert!(!tool.requires_write_access("log"));
    }

    #[test]
    fn branch_is_not_write_gated() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        // Branch listing is read-only; it must not require write access
        assert!(!tool.requires_write_access("branch"));
    }

    #[tokio::test]
    async fn blocks_readonly_mode_for_write_ops() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        let result = tool
            .execute(json!({"operation": "commit", "message": "test"}))
            .await
            .unwrap();
        assert!(!result.success);
        // can_act() returns false for ReadOnly, so we get the "higher autonomy level" message
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("higher autonomy"));
    }

    #[tokio::test]
    async fn allows_branch_listing_in_readonly_mode() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository so the command can succeed
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        let result = tool.execute(json!({"operation": "branch"})).await.unwrap();
        // Branch listing must not be blocked by read-only autonomy
        let error_msg = result.error.as_deref().unwrap_or("");
        assert!(
            !error_msg.contains("read-only") && !error_msg.contains("higher autonomy"),
            "branch listing should not be blocked in read-only mode, got: {error_msg}"
        );
    }

    #[tokio::test]
    async fn allows_readonly_ops_in_readonly_mode() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        // This will fail because there's no git repo, but it shouldn't be blocked by autonomy
        let result = tool.execute(json!({"operation": "status"})).await.unwrap();
        // The error should be about git (not about autonomy/read-only mode)
        assert!(!result.success, "Expected failure due to missing git repo");
        let error_msg = result.error.as_deref().unwrap_or("");
        assert!(
            !error_msg.is_empty(),
            "Expected a git-related error message"
        );
        assert!(
            !error_msg.contains("read-only") && !error_msg.contains("autonomy"),
            "Error should be about git, not about autonomy restrictions: {error_msg}"
        );
    }

    #[tokio::test]
    async fn rejects_missing_operation() {
        let tmp = TempDir::new().unwrap();
        let tool = test_tool(tmp.path());

        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Missing 'operation'"));
    }

    #[tokio::test]
    async fn rejects_unknown_operation() {
        let tmp = TempDir::new().unwrap();
        // Initialize a git repository
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let tool = test_tool(tmp.path());

        let result = tool.execute(json!({"operation": "push"})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Unknown operation"));
    }

    #[test]
    fn truncates_multibyte_commit_message_without_panicking() {
        let long = "🦀".repeat(2500);
        let truncated = GitOperationsTool::truncate_commit_message(&long);

        assert_eq!(truncated.chars().count(), 2000);
    }

    // ── Clone URL validation ────────────────────────────────────

    #[test]
    fn clone_url_accepts_https() {
        assert!(GitOperationsTool::validate_clone_url("https://github.com/org/repo.git").is_ok());
        assert!(
            GitOperationsTool::validate_clone_url("https://gitlab.com/user/project.git").is_ok()
        );
    }

    #[test]
    fn clone_url_rejects_ssh() {
        let err = GitOperationsTool::validate_clone_url("git@github.com:org/repo.git").unwrap_err();
        assert!(err.to_string().contains("HTTPS"));
    }

    #[test]
    fn clone_url_rejects_git_protocol() {
        let err =
            GitOperationsTool::validate_clone_url("git://github.com/org/repo.git").unwrap_err();
        assert!(err.to_string().contains("HTTPS"));
    }

    #[test]
    fn clone_url_rejects_http() {
        let err =
            GitOperationsTool::validate_clone_url("http://github.com/org/repo.git").unwrap_err();
        assert!(err.to_string().contains("HTTPS"));
    }

    #[test]
    fn clone_url_rejects_command_injection() {
        assert!(GitOperationsTool::validate_clone_url("https://evil.com/$(rm -rf /)").is_err());
        assert!(GitOperationsTool::validate_clone_url("https://evil.com/`whoami`").is_err());
        assert!(
            GitOperationsTool::validate_clone_url("https://evil.com/repo.git; rm -rf /").is_err()
        );
        assert!(GitOperationsTool::validate_clone_url("https://evil.com/repo.git | cat").is_err());
        assert!(GitOperationsTool::validate_clone_url("https://evil.com/repo.git&whoami").is_err());
    }

    #[test]
    fn clone_url_rejects_empty() {
        assert!(GitOperationsTool::validate_clone_url("").is_err());
    }

    // ── Clone destination validation ─────────────────────────────

    #[test]
    fn clone_destination_rejects_traversal() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        assert!(tool.validate_clone_destination("../../etc/evil").is_err());
        assert!(tool.validate_clone_destination("../outside").is_err());
    }

    #[test]
    fn clone_destination_rejects_shell_metacharacters() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        assert!(tool.validate_clone_destination("repo$(whoami)").is_err());
        assert!(tool.validate_clone_destination("repo`evil`").is_err());
        assert!(tool.validate_clone_destination("repo;evil").is_err());
        assert!(tool.validate_clone_destination("repo|evil").is_err());
    }

    #[test]
    fn clone_destination_rejects_empty() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        assert!(tool.validate_clone_destination("").is_err());
        assert!(tool.validate_clone_destination("  ").is_err());
    }

    #[test]
    fn clone_destination_accepts_safe_relative() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        assert!(tool.validate_clone_destination("my-repo").is_ok());
        assert!(tool.validate_clone_destination("projects/my-repo").is_ok());
    }

    #[test]
    fn clone_destination_rejects_forbidden_path() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        // /etc is in default forbidden_paths and workspace_only=true blocks absolute paths
        assert!(tool.validate_clone_destination("/etc/evil-repo").is_err());
        assert!(tool.validate_clone_destination("/tmp/evil-repo").is_err());
    }

    #[tokio::test]
    async fn clone_is_write_gated() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = GitOperationsTool::new(security, tmp.path().to_path_buf());

        let result = tool
            .execute(json!({"operation": "clone", "url": "https://github.com/org/repo.git"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("higher autonomy"));
    }

    #[tokio::test]
    async fn clone_rejects_missing_url() {
        let tmp = TempDir::new().unwrap();
        // Need a git repo for the non-clone check to pass
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let tool = test_tool(tmp.path());

        let result = tool.execute(json!({"operation": "clone"})).await.unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Missing 'url'"));
    }

    #[tokio::test]
    async fn clone_rejects_non_https_url() {
        let tmp = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let tool = test_tool(tmp.path());

        let result = tool
            .execute(json!({"operation": "clone", "url": "git@github.com:org/repo.git"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("HTTPS"));
    }
}
