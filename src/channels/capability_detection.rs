pub(crate) fn looks_like_remote_repo_review_request(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let has_repo_url = lower.contains("github.com/")
        || lower.contains("gitlab.com/")
        || lower.contains("bitbucket.org/");

    if !has_repo_url {
        return false;
    }

    let english_review_hints = [
        "review",
        "inspect",
        "audit",
        "analyze",
        "analyse",
        "check",
        "look at",
        "look through",
        "codebase",
        "repository",
        "repo",
        "source code",
        "what is wrong",
        "obvious issue",
    ];
    if english_review_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

fn message_contains_url(user_message: &str) -> bool {
    let lower = user_message.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || lower.contains("github.com/")
        || lower.contains("gitlab.com/")
        || lower.contains("bitbucket.org/")
}

pub(crate) fn looks_like_web_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || !message_contains_url(trimmed) {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "review",
        "inspect",
        "audit",
        "analyze",
        "analyse",
        "check",
        "look at",
        "look through",
        "read",
        "summarize",
        "open",
        "browse",
        "visit",
        "fetch",
        "search",
        "look up",
        "what is on",
        "what's on",
    ];
    if english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

pub(crate) fn looks_like_shell_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "run ", "execute ", "terminal", "shell", "command", "build", "compile", "test", "cargo ",
        "npm ", "pnpm ", "yarn ", "pip ", "python ", "pytest", "cmake", "docker ", "kubectl ",
    ];
    let repo_metrics_hints = [
        "cloc",
        "lines of code",
        "line count",
        "count the lines",
        "count lines",
    ];
    if english_hints.iter().any(|hint| lower.contains(hint))
        || repo_metrics_hints.iter().any(|hint| lower.contains(hint))
        || contains_make_command_hint(&lower)
    {
        return true;
    }

    false
}

pub(crate) fn looks_like_repo_metrics_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let metric_hints = [
        "lines of code",
        "line count",
        "count the lines",
        "count lines",
        "cloc",
        "tokei",
    ];
    if !metric_hints.iter().any(|hint| lower.contains(hint)) {
        return false;
    }

    message_contains_url(trimmed)
        || ["repo", "repository", "codebase", "source tree"]
            .iter()
            .any(|hint| lower.contains(hint))
}

pub(crate) fn looks_like_desktop_computer_use_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let explicit_app_hints = [
        "open chrome",
        "open google chrome",
        "launch chrome",
        "launch google chrome",
        "open firefox",
        "launch firefox",
        "open the chrome application",
        "open the google chrome application",
    ];
    if explicit_app_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    let control_hints = [
        "open ", "launch ", "focus ", "click", "scroll", "type ", "press ", "drag ", "capture",
    ];
    let desktop_hints = [
        "on the computer",
        "on my computer",
        "desktop",
        "application",
        "app ",
        "window",
        "screen",
        "mouse",
        "keyboard",
    ];

    control_hints.iter().any(|hint| lower.contains(hint))
        && desktop_hints.iter().any(|hint| lower.contains(hint))
}

fn contains_make_command_hint(lower: &str) -> bool {
    lower.starts_with("make ")
        || lower.contains("\nmake ")
        || lower.contains("`make ")
        || lower.contains("'make ")
        || lower.contains("\"make ")
        || lower.contains(" run make ")
        || lower.contains(" execute make ")
        || lower.contains(" command make ")
}

pub(crate) fn looks_like_file_read_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let mentions_path = trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(".md");
    let english_hints = [
        "read file",
        "open file",
        "show file",
        "inspect file",
        "cat ",
    ];
    if mentions_path && english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

pub(crate) fn looks_like_file_write_task(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let mentions_path = trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(".rs");
    let english_hints = [
        "edit file",
        "modify file",
        "update file",
        "change file",
        "write file",
        "create file",
        "patch file",
    ];
    if mentions_path && english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

pub(crate) fn looks_like_current_model_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "which model",
        "what model",
        "current model",
        "model are you using",
        "model are you on",
        "model specifically",
        "specific model",
    ];
    if english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

pub(crate) fn looks_like_loaded_skills_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "which skills",
        "what skills",
        "skills do you have",
        "available skills",
        "loaded skills",
    ];
    if english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    false
}

pub(crate) fn should_try_llm_capability_recovery(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_problem_hints = [
        "can't",
        "cannot",
        "unable",
        "couldn't",
        "could not",
        "failed",
        "failure",
        "blocked",
        "missing",
        "unavailable",
        "denied",
        "refused",
    ];
    let english_capability_hints = [
        "capability",
        "skill",
        "tool",
        "access",
        "permission",
        "approval",
        "browser",
        "channel",
    ];
    let english_repair_hints = ["fix", "solve", "recover", "restore", "unblock", "enable"];
    let english_diagnostic_hints = ["why", "how"];

    let has_problem = english_problem_hints
        .iter()
        .any(|hint| lower.contains(hint));
    let has_capability_context = english_capability_hints
        .iter()
        .any(|hint| lower.contains(hint));
    let has_repair = english_repair_hints.iter().any(|hint| lower.contains(hint));
    let has_diagnostic = english_diagnostic_hints
        .iter()
        .any(|hint| lower.contains(hint));

    if has_problem || has_capability_context && (has_repair || has_diagnostic) {
        return true;
    }

    false
}

pub(crate) fn extract_json_object(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if let Some(stripped) = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
    {
        return stripped
            .trim()
            .strip_suffix("```")
            .map(str::trim)
            .filter(|inner| inner.starts_with('{') && inner.ends_with('}'));
    }

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        Some(trimmed)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        contains_make_command_hint, looks_like_desktop_computer_use_task,
        looks_like_repo_metrics_task, looks_like_shell_task, should_try_llm_capability_recovery,
    };

    #[test]
    fn shell_detection_keeps_real_make_command_requests() {
        assert!(contains_make_command_hint("make test"));
        assert!(looks_like_shell_task("run make test in this repo"));
    }

    #[test]
    fn shell_detection_ignores_plain_english_make_phrasing() {
        assert!(!contains_make_command_hint(
            "what improvements you can do make yourself better and smarter?"
        ));
        assert!(!looks_like_shell_task(
            "https://github.com/topway-ai/topclaw This is your codebase, tell me what improvements you can do make yourself better and smarter?"
        ));
    }

    #[test]
    fn repo_metrics_detection_flags_remote_repo_line_count_requests() {
        assert!(looks_like_repo_metrics_task(
            "How many lines of code does this repo have? https://github.com/topway-ai/topclaw"
        ));
    }

    #[test]
    fn desktop_detection_flags_open_chrome_requests() {
        assert!(looks_like_desktop_computer_use_task(
            "open the Google Chrome application on the computer, then go to https://github.com/topway-ai/topagent"
        ));
    }

    #[test]
    fn llm_capability_recovery_detection_ignores_normal_how_questions() {
        assert!(!should_try_llm_capability_recovery(
            "How many lines of code does this repo have? https://github.com/topway-ai/topclaw"
        ));
    }

    #[test]
    fn llm_capability_recovery_detection_keeps_real_capability_questions() {
        assert!(should_try_llm_capability_recovery(
            "why can't you use that desktop skill?"
        ));
    }
}
