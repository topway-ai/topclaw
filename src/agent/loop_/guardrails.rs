use regex::Regex;
use std::sync::LazyLock;

const TOOL_UNAVAILABLE_RETRY_PROMPT_PREFIX: &str = "Internal correction: your prior reply claimed required tools were unavailable. Use only the runtime-allowed tools listed below. If file changes are requested and `file_write`/`file_edit` are listed, call them directly.";

/// Detect completion claims that imply state-changing work already happened
/// without an accompanying tool call. The cue and the past-tense side-effect
/// verb must form an adjacent phrase (e.g. "I've created", "successfully
/// wrote", "I just deleted") so generic conversational text that merely
/// mentions tool capabilities (e.g. "I have a file_write tool to write files")
/// does not trip the guardrail.
static ACTION_COMPLETION_CLAIM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        \b(?:
            (?:i(?:'ve|\s+have)|we(?:'ve|\s+have))
                \s+(?:successfully\s+|already\s+|just\s+|now\s+)*
                (?:created|written|wrote|ran|executed|updated|deleted|removed|renamed|moved|installed|saved|made)
            | i\s+(?:successfully\s+|already\s+|just\s+)*
                (?:created|wrote|ran|executed|updated|deleted|removed|renamed|moved|installed|saved|made)
            | successfully\s+(?:created|wrote|written|ran|executed|updated|deleted|removed|renamed|moved|installed|saved|made)
        )\b",
    )
    .unwrap()
});

/// Concrete artifacts often referenced in file/system action completion claims.
/// Required as a co-occurrence signal so generic phrases like "I made it"
/// without any artifact reference do not trip the guardrail.
static SIDE_EFFECT_ACTION_OBJECT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)\b(file|files|folder|folders|directory|directories|workspace|cwd|current\s+working\s+directory|command|commands|script|scripts|path|paths)\b",
    )
    .unwrap()
});

/// Detect responses that incorrectly claim file tooling is unavailable even
/// when runtime policy allows file tools in this turn.
static TOOL_UNAVAILABLE_CLAIM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        \b(
            i\s+(?:do\s+not|don't)\s+have\s+access|
            i\s+(?:cannot|can't)\s+(?:access|use|perform|create|edit|write)|
            i\s+am\s+unable\s+to|
            no\s+(?:tool|tools|function|functions)\s+(?:available|access)
        )\b
        [^.!?\n]{0,220}
        \b(
            tool|tools|function|functions|file|file_write|file_edit|
            create|write|edit|delete
        )\b",
    )
    .unwrap()
});

pub(super) fn looks_like_unverified_action_completion_without_tool_call(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    ACTION_COMPLETION_CLAIM_REGEX.is_match(trimmed)
        && SIDE_EFFECT_ACTION_OBJECT_REGEX.is_match(trimmed)
}

pub(super) fn looks_like_tool_unavailability_claim(
    text: &str,
    tool_specs: &[crate::tools::ToolSpec],
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || !TOOL_UNAVAILABLE_CLAIM_REGEX.is_match(trimmed) {
        return false;
    }

    tool_specs
        .iter()
        .any(|spec| matches!(spec.name.as_str(), "file_write" | "file_edit"))
}

pub(super) fn build_tool_unavailable_retry_prompt(tool_specs: &[crate::tools::ToolSpec]) -> String {
    const MAX_TOOLS_IN_PROMPT: usize = 24;
    let tool_list = tool_specs
        .iter()
        .map(|spec| spec.name.as_str())
        .take(MAX_TOOLS_IN_PROMPT)
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "{TOOL_UNAVAILABLE_RETRY_PROMPT_PREFIX}\nRuntime tools: {tool_list}\nEmit the correct tool call now if tool use is required. Otherwise provide the final answer without claiming missing tools."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_completion_guardrail_does_not_fire_on_tool_capability_listing() {
        // Regression: a benign reply to "what tools do you have?" used to
        // trip the guardrail because it scattered "I have", "write", and
        // "files" across the text. The model has not actually performed
        // any side effect here.
        let listing = "I have several tools available: file_read to read files, \
                       file_write to write files to disk, file_edit to edit files, \
                       and shell to run commands.";
        assert!(!looks_like_unverified_action_completion_without_tool_call(
            listing
        ));
    }

    #[test]
    fn action_completion_guardrail_does_not_fire_on_welcome_message() {
        let welcome = "Hello there. I'm TopClaw. I've just woken up in your workspace \
                       and I'm ready to help you with coding, system tasks, or searching \
                       the web. I've already read your USER.md and SOUL.md.";
        assert!(!looks_like_unverified_action_completion_without_tool_call(
            welcome
        ));
    }

    #[test]
    fn action_completion_guardrail_still_fires_on_real_completion_claim() {
        assert!(looks_like_unverified_action_completion_without_tool_call(
            "Done — I've created the `names` folder in the current working directory."
        ));
        assert!(looks_like_unverified_action_completion_without_tool_call(
            "Finished successfully: I wrote the file to the workspace path."
        ));
        assert!(looks_like_unverified_action_completion_without_tool_call(
            "I just deleted the old script files for you."
        ));
    }
}
