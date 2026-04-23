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

    if looks_like_desktop_computer_use_task(trimmed) {
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

pub(crate) fn looks_like_rate_limit_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("action budget")
        || lower.contains("max actions per hour")
        || lower.contains("max_actions_per_hour")
}

pub(crate) fn looks_like_tool_inventory_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let english_hints = [
        "which tools",
        "what tools",
        "tools do you have",
        "available tools",
        "loaded tools",
        "tool list",
        "list out all the tools",
        "list all the tools",
    ];
    english_hints.iter().any(|hint| lower.contains(hint))
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

pub(crate) fn looks_like_tools_and_skills_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let explicit_hints = [
        "what tools/skills do you have",
        "what tools and skills do you have",
        "list out all the tools and skills you have",
        "list all the tools and skills you have",
        "tools/skills",
        "tools and skills",
    ];
    if explicit_hints.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    lower.contains("tool")
        && lower.contains("skill")
        && ["have", "list", "available"]
            .iter()
            .any(|hint| lower.contains(hint))
}

pub(crate) fn looks_like_computer_use_availability_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if !(lower.contains("computer_use") || lower.contains("computer use")) {
        return false;
    }

    [
        "why didn't you mention",
        "why didnt you mention",
        "supposed to have",
        "do you have",
        "is it available",
        "isn't it available",
        "isnt it available",
        "currently loaded",
        "currently available",
        "why wasn't it mentioned",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

pub(crate) fn looks_like_audio_file_question(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let references_audio = [
        "[audio:",
        "[voice]",
        "audio file",
        "voice message",
        "voice note",
        "transcrib",
        "transcript",
        ".m4a",
        ".mp3",
        ".wav",
        ".flac",
        ".ogg",
        ".opus",
        ".webm",
    ]
    .iter()
    .any(|hint| lower.contains(hint));

    if !references_audio {
        return false;
    }

    let asks_about_audio = [
        "read this file",
        "read file",
        "can you read",
        "able to read",
        "listen to",
        "summarize",
        "what does it say",
        "transcribe",
    ]
    .iter()
    .any(|hint| lower.contains(hint));

    asks_about_audio || lower.contains('?') || lower.contains("[audio:")
}

pub(crate) fn looks_like_audio_capability_question(user_message: &str) -> bool {
    if !looks_like_audio_file_question(user_message) {
        return false;
    }

    let lower = user_message.trim().to_ascii_lowercase();
    [
        "can you",
        "are you able",
        "able to",
        "do you support",
        "if i upload",
        "if i send",
        "read this file",
        "can you read",
        "can you transcribe",
        "can you summarize",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

pub(crate) fn looks_like_skill_workflow_request(user_message: &str) -> bool {
    let trimmed = user_message.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains("skill") {
        return false;
    }

    [
        "find ", "search", "discover", "install", "create", "build", "make ", "scaffold",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

pub(crate) fn looks_like_skill_workflow_advisory_question(user_message: &str) -> bool {
    if !looks_like_skill_workflow_request(user_message) {
        return false;
    }

    let lower = user_message.trim().to_ascii_lowercase();
    [
        "tell me whether",
        "would you use",
        "what would you use",
        "which would you use",
        "use first",
        "do you need skills.sh",
        "need skills.sh",
        "don't write files yet",
        "do not write files yet",
        "before writing files",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
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

fn extract_loaded_skill_names_from_system_prompt(system_prompt: &str) -> Vec<String> {
    let Some(section_start) = system_prompt.find("<available_skills>") else {
        return Vec::new();
    };
    let after_start = &system_prompt[section_start + "<available_skills>".len()..];
    let Some(section_end) = after_start.find("</available_skills>") else {
        return Vec::new();
    };
    let mut remaining = &after_start[..section_end];
    let mut names = Vec::new();

    while let Some(skill_start) = remaining.find("<skill>") {
        let after_skill_start = &remaining[skill_start + "<skill>".len()..];
        let Some(skill_end) = after_skill_start.find("</skill>") else {
            break;
        };
        let skill_block = &after_skill_start[..skill_end];
        if let Some(name_start) = skill_block.find("<name>") {
            let after_name_start = &skill_block[name_start + "<name>".len()..];
            if let Some(name_end) = after_name_start.find("</name>") {
                let name = after_name_start[..name_end].trim();
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
        remaining = &after_skill_start[skill_end + "</skill>".len()..];
    }

    names.sort();
    names.dedup();
    names
}

pub(crate) fn build_local_capability_response(
    content: &str,
    system_prompt: &str,
    provider_name: &str,
    model_name: &str,
    visible_tool_names: &[String],
) -> Option<String> {
    if looks_like_current_model_question(content) {
        return Some(format!(
            "I'm currently using provider `{provider_name}` with model `{model_name}`."
        ));
    }

    if looks_like_rate_limit_question(content) {
        return Some(format!(
            "The default action budget is {} actions per hour, configured by `autonomy.max_actions_per_hour`.",
            crate::config::AutonomyConfig::default().max_actions_per_hour
        ));
    }

    if looks_like_tools_and_skills_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let tool_section = if visible_tool_names.is_empty() {
            "Visible tools in this runtime turn: (none).".to_string()
        } else {
            format!(
                "Visible tools in this runtime turn ({}): {}.",
                visible_tool_names.len(),
                visible_tool_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let skill_section = if skill_names.is_empty() {
            "Loaded skills in the current prompt: (none).".to_string()
        } else {
            format!(
                "Loaded skills in the current prompt ({}): {}.",
                skill_names.len(),
                skill_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        return Some(format!(
            "{tool_section}\n{skill_section}\n`computer_use` is a runtime tool when compiled and allowed, not a marketplace skill."
        ));
    }

    if looks_like_tool_inventory_question(content) {
        if visible_tool_names.is_empty() {
            return Some(
                "Visible tools in this runtime turn: (none). I can still answer from chat context, but I should not claim tool access that is not actually loaded."
                    .to_string(),
            );
        }

        return Some(format!(
            "Visible tools in this runtime turn ({}): {}.",
            visible_tool_names.len(),
            visible_tool_names
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if looks_like_computer_use_availability_question(content) {
        let has_computer_use = visible_tool_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("computer_use"));
        let availability = if has_computer_use {
            "`computer_use` is currently loaded as a runtime tool in this turn."
        } else {
            "`computer_use` is not currently loaded in this runtime turn."
        };

        return Some(format!(
            "{availability} `computer_use` is a tool, not a skill. A separate curated skill such as `desktop-computer-use` can document workflows around that tool, but it does not replace the compiled runtime tool itself."
        ));
    }

    if looks_like_loaded_skills_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        if skill_names.is_empty() {
            return Some(
                "I don't have any advertised skills loaded in the current runtime prompt."
                    .to_string(),
            );
        }

        let visible = skill_names
            .iter()
            .take(8)
            .map(|name| format!("- `{name}`"))
            .collect::<Vec<_>>()
            .join("\n");
        let remaining = skill_names.len().saturating_sub(8);
        let suffix = if remaining > 0 {
            format!("\n- ... {remaining} more")
        } else {
            String::new()
        };

        return Some(format!(
            "I currently have {} advertised skills loaded:\n{}{suffix}\n\nAsk about any one by name, or just tell me the task and I'll use what fits.",
            skill_names.len(),
            visible
        ));
    }

    if looks_like_audio_capability_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let skill_creator_suffix = if skill_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("skill-creator"))
        {
            " If you want a reusable workflow around local audio files, I can also use `skill-creator` to help scaffold one."
        } else {
            ""
        };

        let attachment_prefix = if content.to_ascii_lowercase().contains("[audio:") {
            "That looks like an attached audio file, not a plain-text document. "
        } else {
            "An m4a/audio file is not plain text. "
        };

        return Some(format!(
            "{attachment_prefix}I can't read it the way I would a `.txt` or `.md` file. \
            If channel transcription is enabled, I can transcribe supported audio uploads \
            such as m4a, mp3, wav, flac, ogg, opus, and webm, then summarize or analyze the transcript. \
            If transcription is not enabled, I need a transcription tool or service first.{skill_creator_suffix}"
        ));
    }

    if looks_like_skill_workflow_advisory_question(content) {
        let skill_names = extract_loaded_skill_names_from_system_prompt(system_prompt);
        let has_skill_creator = skill_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("skill-creator"));
        let skill_creator_step = if has_skill_creator {
            "If no installed or curated local skill fits, I would use `skill-creator` next."
        } else {
            "If no installed or curated local skill fits, I would scaffold the new skill directly in the workspace."
        };

        return Some(format!(
            "I would check installed and curated local skills first, not `skills.sh` and not any desktop/browser tool. {skill_creator_step} \
            For a planning-only question like this, I do not need `skills.sh` as the first step."
        ));
    }

    None
}

pub(crate) fn should_answer_local_capability_response_immediately(content: &str) -> bool {
    looks_like_current_model_question(content)
        || looks_like_tools_and_skills_question(content)
        || looks_like_tool_inventory_question(content)
        || looks_like_computer_use_availability_question(content)
        || looks_like_loaded_skills_question(content)
        || looks_like_audio_capability_question(content)
        || looks_like_skill_workflow_advisory_question(content)
}

#[cfg(test)]
mod tests {
    use super::{
        build_local_capability_response, contains_make_command_hint,
        looks_like_audio_capability_question, looks_like_audio_file_question,
        looks_like_computer_use_availability_question, looks_like_desktop_computer_use_task,
        looks_like_repo_metrics_task, looks_like_shell_task,
        looks_like_skill_workflow_advisory_question, looks_like_skill_workflow_request,
        looks_like_tool_inventory_question, looks_like_tools_and_skills_question,
        looks_like_web_task, should_answer_local_capability_response_immediately,
        should_try_llm_capability_recovery,
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
    fn web_detection_does_not_claim_desktop_chrome_requests() {
        assert!(!looks_like_web_task(
            "open Google Chrome to https://example.com and scroll to the bottom"
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

    #[test]
    fn audio_file_detection_flags_attached_audio_questions() {
        assert!(looks_like_audio_file_question(
            "[Audio: 20260416 212230.m4a] /tmp/20260416_212230.m4a\n\nAre you able to read this file?"
        ));
        assert!(looks_like_audio_file_question(
            "are you able to read m4a audio file?"
        ));
    }

    #[test]
    fn audio_capability_detection_stays_narrow() {
        assert!(looks_like_audio_capability_question(
            "Can you read or transcribe an m4a audio file if I upload it here?"
        ));
        assert!(!looks_like_audio_capability_question(
            "Transcribe this attached m4a audio file now."
        ));
    }

    #[test]
    fn skill_workflow_detection_flags_find_and_create_skill_requests() {
        assert!(looks_like_skill_workflow_request("find a skill yourself"));
        assert!(looks_like_skill_workflow_request(
            "create a skill that can transcribe m4a audio file"
        ));
    }

    #[test]
    fn skill_workflow_advisory_detection_flags_meta_planning_questions() {
        assert!(looks_like_skill_workflow_advisory_question(
            "Create a skill that can transcribe m4a audio file, but do not write files yet. Tell me whether you would use local curated skills or skill-creator first, and whether you need skills.sh."
        ));
        assert!(!looks_like_skill_workflow_advisory_question(
            "Create a skill that can transcribe m4a audio file."
        ));
    }

    #[test]
    fn tools_and_skills_detection_flags_inventory_prompts() {
        assert!(looks_like_tools_and_skills_question(
            "list out all the tools and skills you have"
        ));
        assert!(looks_like_tool_inventory_question(
            "what tools do you have?"
        ));
        assert!(!looks_like_tool_inventory_question(
            "what skills do you have?"
        ));
    }

    #[test]
    fn computer_use_availability_detection_flags_runtime_question() {
        assert!(looks_like_computer_use_availability_question(
            "You are supposed to have computer_use skill too, why didn't you mention it?"
        ));
        assert!(!looks_like_computer_use_availability_question(
            "Open Google Chrome with computer_use."
        ));
    }

    #[test]
    fn build_local_capability_response_answers_model_question() {
        let response = build_local_capability_response(
            "which model are you using?",
            "",
            "openai",
            "gpt-4",
            &[],
        );
        assert!(response.is_some());
        assert!(response.unwrap().contains("openai"));
    }

    #[test]
    fn build_local_capability_response_answers_tool_inventory_question() {
        let tools: Vec<String> = vec!["browser_open".into(), "shell".into()];
        let response = build_local_capability_response(
            "what tools do you have?",
            "",
            "openai",
            "gpt-4",
            &tools,
        );
        assert!(response.is_some());
        let text = response.unwrap();
        assert!(text.contains("browser_open"));
    }

    #[test]
    fn build_local_capability_response_returns_none_for_non_capability_question() {
        let response = build_local_capability_response(
            "what is the weather today?",
            "",
            "openai",
            "gpt-4",
            &[],
        );
        assert!(response.is_none());
    }

    #[test]
    fn should_answer_local_capability_response_matches_detection() {
        assert!(should_answer_local_capability_response_immediately(
            "which model are you using?"
        ));
        assert!(should_answer_local_capability_response_immediately(
            "what tools do you have?"
        ));
        assert!(!should_answer_local_capability_response_immediately(
            "hello world"
        ));
    }

    #[test]
    fn extract_skill_names_from_system_prompt() {
        let prompt = "<available_skills><skill><name>foo</name></skill><skill><name>bar</name></skill></available_skills>";
        let names = super::extract_loaded_skill_names_from_system_prompt(prompt);
        assert_eq!(names, vec!["bar", "foo"]);
    }

    #[test]
    fn extract_skill_names_returns_empty_without_section() {
        let names = super::extract_loaded_skill_names_from_system_prompt("no skills here");
        assert!(names.is_empty());
    }
}
