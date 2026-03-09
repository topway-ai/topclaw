use super::traits::ToolResult;
use crate::security::policy::ToolOperation;
use crate::security::SecurityPolicy;

pub(super) fn enforce_action(security: &SecurityPolicy, action: &str) -> Option<ToolResult> {
    match security.enforce_tool_operation(ToolOperation::Act, action) {
        Ok(()) => None,
        Err(error) => Some(ToolResult {
            success: false,
            output: String::new(),
            error: Some(error),
        }),
    }
}
