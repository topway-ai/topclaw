use crate::config::NonCliNaturalLanguageApprovalMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelRuntimeCommand {
    ShowProviders,
    SetProvider(String),
    ShowModel,
    SetModel(String),
    NewSession,
    RequestAllToolsOnce,
    RequestToolApproval(String),
    ConfirmToolApproval(String),
    ConfirmToolApprovalAlways(String),
    ApprovePendingRequest(String),
    DenyToolApproval(String),
    ListPendingApprovals,
    ApproveTool(String),
    UnapproveTool(String),
    ListApprovals,
}

pub(crate) const APPROVAL_ALL_TOOLS_ONCE_TOKEN: &str = "__all_tools_once__";

fn supports_runtime_model_switch(channel_name: &str) -> bool {
    matches!(channel_name, "telegram" | "discord")
}

pub(crate) fn parse_runtime_command(
    channel_name: &str,
    content: &str,
) -> Option<ChannelRuntimeCommand> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return parse_natural_language_runtime_command(trimmed);
    }

    let mut parts = trimmed.split_whitespace();
    let command_token = parts.next()?;
    let base_command = command_token
        .split('@')
        .next()
        .unwrap_or(command_token)
        .to_ascii_lowercase();
    let args: Vec<&str> = parts.collect();
    let tail = args.join(" ").trim().to_string();

    match base_command.as_str() {
        "/new" | "/clear" => Some(ChannelRuntimeCommand::NewSession),
        "/approve-all-once" => Some(ChannelRuntimeCommand::RequestAllToolsOnce),
        "/approve-request" => Some(ChannelRuntimeCommand::RequestToolApproval(tail)),
        "/approve-confirm" => Some(ChannelRuntimeCommand::ConfirmToolApproval(tail)),
        "/approve-confirm-always" => Some(ChannelRuntimeCommand::ConfirmToolApprovalAlways(tail)),
        "/approve-allow" => Some(ChannelRuntimeCommand::ApprovePendingRequest(tail)),
        "/approve-deny" => Some(ChannelRuntimeCommand::DenyToolApproval(tail)),
        "/approve-pending" => Some(ChannelRuntimeCommand::ListPendingApprovals),
        "/approve" => Some(ChannelRuntimeCommand::ApproveTool(tail)),
        "/unapprove" => Some(ChannelRuntimeCommand::UnapproveTool(tail)),
        "/approvals" => Some(ChannelRuntimeCommand::ListApprovals),
        "/models" if supports_runtime_model_switch(channel_name) => {
            if let Some(provider) = args.first() {
                Some(ChannelRuntimeCommand::SetProvider(
                    provider.trim().to_string(),
                ))
            } else {
                Some(ChannelRuntimeCommand::ShowProviders)
            }
        }
        "/model" if supports_runtime_model_switch(channel_name) => {
            if tail.is_empty() {
                Some(ChannelRuntimeCommand::ShowModel)
            } else {
                Some(ChannelRuntimeCommand::SetModel(tail))
            }
        }
        _ => None,
    }
}

fn is_runtime_token(value: &str) -> bool {
    let token = value.trim();
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn extract_runtime_tail_token(text: &str, prefixes: &[&str]) -> Option<String> {
    prefixes.iter().find_map(|prefix| {
        text.strip_prefix(prefix).and_then(|rest| {
            let token = rest.trim();
            if is_runtime_token(token) {
                Some(token.to_string())
            } else {
                None
            }
        })
    })
}

fn contains_any_fragment(haystack: &str, fragments: &[&str]) -> bool {
    fragments.iter().any(|fragment| haystack.contains(fragment))
}

fn is_natural_language_all_tools_once_intent(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let has_allow_verb = contains_any_fragment(&lower, &["approve", "allow"])
        || contains_any_fragment(trimmed, &["授权", "放开", "允许"]);
    let has_all_tools_scope = contains_any_fragment(&lower, &["all tools", "all commands"])
        || contains_any_fragment(trimmed, &["所有工具", "全部工具", "所有命令", "全部命令"]);
    let has_one_time_scope = contains_any_fragment(&lower, &["once", "one-time", "one time"])
        || contains_any_fragment(trimmed, &["一次", "这次"]);

    has_allow_verb && has_all_tools_scope && has_one_time_scope
}

pub(crate) fn approval_target_label(tool_name: &str) -> String {
    if tool_name == APPROVAL_ALL_TOOLS_ONCE_TOKEN {
        "all tools/commands (one-time bypass token)".to_string()
    } else {
        tool_name.to_string()
    }
}

fn parse_natural_language_runtime_command(content: &str) -> Option<ChannelRuntimeCommand> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "show pending approvals" | "list pending approvals" | "pending approvals"
    ) {
        return Some(ChannelRuntimeCommand::ListPendingApprovals);
    }
    if trimmed == "查看授权"
        || matches!(
            lower.as_str(),
            "show approvals" | "list approvals" | "approvals"
        )
    {
        return Some(ChannelRuntimeCommand::ListApprovals);
    }
    if is_natural_language_all_tools_once_intent(trimmed)
        || matches!(
            lower.as_str(),
            "approve all tools once" | "allow all tools once" | "approve all once"
        )
    {
        return Some(ChannelRuntimeCommand::RequestAllToolsOnce);
    }

    if let Some(request_id) = extract_runtime_tail_token(&lower, &["confirm "]) {
        if request_id.starts_with("apr-") {
            return Some(ChannelRuntimeCommand::ConfirmToolApproval(request_id));
        }
    }
    if let Some(request_id) = extract_runtime_tail_token(trimmed, &["确认授权 "]) {
        if request_id.starts_with("apr-") {
            return Some(ChannelRuntimeCommand::ConfirmToolApproval(request_id));
        }
    }
    if matches!(lower.as_str(), "approve" | "allow") || matches!(trimmed, "批准" | "同意" | "允许")
    {
        return Some(ChannelRuntimeCommand::ConfirmToolApproval(String::new()));
    }
    if matches!(lower.as_str(), "deny" | "reject" | "decline")
        || matches!(trimmed, "拒绝" | "不同意")
    {
        return Some(ChannelRuntimeCommand::DenyToolApproval(String::new()));
    }

    if let Some(tool) =
        extract_runtime_tail_token(&lower, &["revoke tool ", "unapprove ", "revoke "])
    {
        return Some(ChannelRuntimeCommand::UnapproveTool(tool));
    }
    if let Some(tool) = extract_runtime_tail_token(trimmed, &["撤销工具 ", "取消授权 "]) {
        return Some(ChannelRuntimeCommand::UnapproveTool(tool));
    }

    if let Some(tool) = extract_runtime_tail_token(&lower, &["approve tool ", "approve "]) {
        return Some(ChannelRuntimeCommand::RequestToolApproval(tool));
    }
    if let Some(tool) = extract_runtime_tail_token(trimmed, &["授权工具 ", "请放开 ", "放开 "])
    {
        return Some(ChannelRuntimeCommand::RequestToolApproval(tool));
    }

    None
}

pub(crate) fn is_approval_management_command(command: &ChannelRuntimeCommand) -> bool {
    matches!(
        command,
        ChannelRuntimeCommand::RequestAllToolsOnce
            | ChannelRuntimeCommand::RequestToolApproval(_)
            | ChannelRuntimeCommand::ConfirmToolApproval(_)
            | ChannelRuntimeCommand::ConfirmToolApprovalAlways(_)
            | ChannelRuntimeCommand::ApprovePendingRequest(_)
            | ChannelRuntimeCommand::DenyToolApproval(_)
            | ChannelRuntimeCommand::ListPendingApprovals
            | ChannelRuntimeCommand::ApproveTool(_)
            | ChannelRuntimeCommand::UnapproveTool(_)
            | ChannelRuntimeCommand::ListApprovals
    )
}

pub(crate) fn non_cli_natural_language_mode_label(
    mode: NonCliNaturalLanguageApprovalMode,
) -> &'static str {
    match mode {
        NonCliNaturalLanguageApprovalMode::Disabled => "disabled",
        NonCliNaturalLanguageApprovalMode::RequestConfirm => "request_confirm",
        NonCliNaturalLanguageApprovalMode::Direct => "direct",
    }
}
