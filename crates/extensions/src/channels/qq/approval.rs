//! QQ-specific tool approval: slash commands and plain-text prompts.

use serde_json::Value;

use moray_channels::ApprovalDecision;

#[derive(Debug, Clone)]
pub struct ApprovalCommand {
    pub request_id: String,
    pub decision: ApprovalDecision,
}

/// Parse `/approve <req_id>` or `/deny <req_id>` from QQ user text.
pub fn parse_approval_command(msg: &str) -> Option<ApprovalCommand> {
    let trimmed = msg.trim();
    if let Some(req_id) = trimmed.strip_prefix("/approve ") {
        let req_id = req_id.trim();
        if req_id.is_empty() {
            return None;
        }
        return Some(ApprovalCommand {
            request_id: req_id.to_string(),
            decision: ApprovalDecision::AllowOnce,
        });
    }
    if let Some(req_id) = trimmed.strip_prefix("/deny ") {
        let req_id = req_id.trim();
        if req_id.is_empty() {
            return None;
        }
        return Some(ApprovalCommand {
            request_id: req_id.to_string(),
            decision: ApprovalDecision::Deny,
        });
    }
    None
}

pub fn format_approval_prompt(
    tool_name: &str,
    tool_display_name: &str,
    request_id: &str,
    arguments: &Value,
    args_max_len: usize,
) -> String {
    let label = crate::channels::render::friendly_tool_label(tool_name, tool_display_name);
    let raw = arguments.to_string();
    let args_preview = if raw.len() > args_max_len {
        let end = crate::channels::render::floor_utf8_char_boundary(&raw, args_max_len);
        format!("{}...", &raw[..end])
    } else {
        raw
    };
    format!(
        "Tool approval required: {label}\nRequest ID: {request_id}\nArguments: {args_preview}\n\nReply with:\n/approve {request_id}\n/deny {request_id}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_approve_with_tool_call_id() {
        let cmd = parse_approval_command("/approve chatcmpl-tool-abc123").unwrap();
        assert_eq!(cmd.request_id, "chatcmpl-tool-abc123");
        assert_eq!(cmd.decision, ApprovalDecision::AllowOnce);
    }
}
