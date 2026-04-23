use crate::config::NonCliNaturalLanguageApprovalMode;
use crate::providers;
use std::fmt::Write;

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
        "/new" | "/clear" | "/reset" => Some(ChannelRuntimeCommand::NewSession),
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

fn append_current_route(response: &mut String, provider_name: &str, model_name: &str) {
    let _ = writeln!(
        response,
        "Current provider: `{provider_name}`\nCurrent model: `{model_name}`"
    );
}

fn append_approval_management_help(response: &mut String) {
    response.push_str("Request supervised tool approval with `/approve-request <tool-name>`.\n");
    response.push_str("Request one-time all-tools approval with `/approve-all-once`.\n");
    response.push_str("Confirm approval with `/approve-confirm <request-id>`.\n");
    response.push_str("Deny approval with `/approve-deny <request-id>`.\n");
    response.push_str("List pending requests with `/approve-pending`.\n");
    response.push_str("Approve supervised tools with `/approve <tool-name>`.\n");
    response.push_str("Revoke approval with `/unapprove <tool-name>`.\n");
    response.push_str("List approval state with `/approvals`.\n");
    response.push_str(
        "Natural language also works (policy controlled).\n\
        - `direct` mode (default): `授权工具 shell` grants immediately.\n\
        - `request_confirm` mode: `授权工具 shell` then `确认授权 apr-xxxxxx`.\n",
    );
}

pub(super) fn build_models_help_response(
    provider_name: &str,
    model_name: &str,
    cached_models: &[String],
) -> String {
    let mut response = String::new();
    append_current_route(&mut response, provider_name, model_name);
    response.push_str("\nSwitch model with `/model <model-id>`.\n");
    append_approval_management_help(&mut response);

    if cached_models.is_empty() {
        if providers::supports_model_catalog_refresh(provider_name) {
            let _ = writeln!(
                response,
                "\nNo cached model list found for `{provider_name}`. Ask the operator to run `topclaw models refresh --provider {provider_name}`."
            );
        } else {
            let _ = writeln!(
                response,
                "\n`{provider_name}` does not expose a live model catalog refresh path. Switch with `/models <product-priority-provider>` or set a model directly with `/model <model-id>`."
            );
        }
    } else {
        let _ = writeln!(
            response,
            "\nCached model IDs (top {}):",
            cached_models.len()
        );
        for model in cached_models {
            let _ = writeln!(response, "- `{model}`");
        }
    }

    response
}

pub(super) fn build_providers_help_response(provider_name: &str, model_name: &str) -> String {
    let mut response = String::new();
    append_current_route(&mut response, provider_name, model_name);
    response.push_str(
        "\nSwitch provider with `/models <provider>` for product-priority providers only.\n",
    );
    response.push_str("Switch model with `/model <model-id>`.\n\n");
    append_approval_management_help(&mut response);
    response.push('\n');
    response.push_str("Product-priority providers:\n");
    for provider in providers::list_providers()
        .into_iter()
        .filter(|provider| providers::is_product_priority_provider(provider.name))
    {
        if provider.aliases.is_empty() {
            let _ = writeln!(response, "- {}", provider.name);
        } else {
            let _ = writeln!(
                response,
                "- {} (aliases: {})",
                provider.name,
                provider.aliases.join(", ")
            );
        }
    }
    response.push_str("\nAdvanced/compatibility providers (configure via CLI or config.toml):\n");
    for provider in providers::list_providers()
        .into_iter()
        .filter(|provider| !providers::is_product_priority_provider(provider.name))
    {
        if provider.aliases.is_empty() {
            let _ = writeln!(response, "- {}", provider.name);
        } else {
            let _ = writeln!(
                response,
                "- {} (aliases: {})",
                provider.name,
                provider.aliases.join(", ")
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{build_models_help_response, build_providers_help_response};

    #[test]
    fn models_help_response_reports_missing_cache() {
        let response = build_models_help_response("openai", "gpt-4o", &[]);

        assert!(response.contains("Current provider: `openai`"));
        assert!(response.contains("Current model: `gpt-4o`"));
        assert!(response.contains("No cached model list found for `openai`"));
        assert!(response.contains("topclaw models refresh --provider openai"));
    }

    #[test]
    fn providers_help_response_lists_provider_section() {
        let response = build_providers_help_response("anthropic", "claude-sonnet-4");

        assert!(response.contains("Current provider: `anthropic`"));
        assert!(response.contains("Current model: `claude-sonnet-4`"));
        assert!(response.contains("Product-priority providers:"));
        assert!(response
            .contains("Advanced/compatibility providers (configure via CLI or config.toml):"));
        assert!(response.contains(
            "Switch provider with `/models <provider>` for product-priority providers only."
        ));
    }

    #[test]
    fn models_help_response_explains_missing_catalog_refresh_for_codex() {
        let response = build_models_help_response("openai-codex", "gpt-5.4", &[]);

        assert!(response.contains("does not expose a live model catalog refresh path"));
        assert!(!response.contains("topclaw models refresh --provider openai-codex"));
    }
}
