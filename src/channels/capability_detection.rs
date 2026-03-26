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

    let cjk_review_hints = [
        "代码库",
        "仓库",
        "源码",
        "看看",
        "检查",
        "审查",
        "评审",
        "缺陷",
        "问题",
        "有什么明显",
    ];
    cjk_review_hints.iter().any(|hint| trimmed.contains(hint))
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

    let cjk_hints = [
        "看看",
        "检查",
        "审查",
        "评审",
        "读",
        "读取",
        "总结",
        "打开",
        "访问",
        "搜索",
        "查一下",
        "网页",
        "链接",
        "网址",
    ];
    cjk_hints.iter().any(|hint| trimmed.contains(hint))
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

    let cjk_hints = [
        "运行命令",
        "执行命令",
        "终端",
        "命令行",
        "编译",
        "构建",
        "测试",
        "跑一下",
    ];
    cjk_hints.iter().any(|hint| trimmed.contains(hint))
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

    mentions_path
        && ["读取文件", "打开文件", "查看文件", "看看文件"]
            .iter()
            .any(|hint| trimmed.contains(hint))
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

    mentions_path
        && ["修改文件", "更新文件", "编辑文件", "创建文件", "写入文件"]
            .iter()
            .any(|hint| trimmed.contains(hint))
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

    let cjk_hints = [
        "哪个模型",
        "什么模型",
        "当前模型",
        "具体模型",
        "你在用什么模型",
    ];
    cjk_hints.iter().any(|hint| trimmed.contains(hint))
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

    let cjk_hints = [
        "哪些技能",
        "什么技能",
        "你有什么技能",
        "可用技能",
        "已加载技能",
    ];
    cjk_hints.iter().any(|hint| trimmed.contains(hint))
}

pub(crate) fn should_try_llm_capability_recovery(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "can't",
        "cannot",
        "unable",
        "failed",
        "failure",
        "fix",
        "solve",
        "handle",
        "recover",
        "blocked",
        "why",
        "how",
        "capability",
        "skill",
    ];
    if english_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    let cjk_hints = [
        "不能",
        "无法",
        "失败",
        "修复",
        "解决",
        "处理",
        "恢复",
        "为什么",
        "怎么",
        "能力",
        "技能",
    ];
    cjk_hints.iter().any(|hint| trimmed.contains(hint))
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
    use super::{contains_make_command_hint, looks_like_shell_task};

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
}
