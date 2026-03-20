use crate::providers;
use std::fmt::Write;

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
        let _ = writeln!(
            response,
            "\nNo cached model list found for `{provider_name}`. Ask the operator to run `topclaw models refresh --provider {provider_name}`."
        );
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
    response.push_str("\nSwitch provider with `/models <provider>`.\n");
    response.push_str("Switch model with `/model <model-id>`.\n\n");
    append_approval_management_help(&mut response);
    response.push('\n');
    response.push_str("Available providers:\n");
    for provider in providers::list_providers() {
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
        assert!(response.contains("Available providers:"));
        assert!(response.contains("Switch provider with `/models <provider>`."));
    }
}
