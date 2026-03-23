//! Response sanitization for channel output.
//!
//! Strips tool-call XML tags, raw JSON tool artifacts, and other
//! internal protocol elements from LLM responses before forwarding
//! to end users on any channel.

use crate::tools::Tool;
use std::collections::HashSet;

/// Strip tool-call XML tags from outgoing messages.
///
/// LLM responses may contain `<function_calls>`, `<function_call>`,
/// `<tool_call>`, `<toolcall>`, `<tool-call>`, `<tool>`, or `<invoke>`
/// blocks that are internal protocol and must not be forwarded to end
/// users on any channel.
pub(super) fn strip_tool_call_tags(message: &str) -> String {
    const TOOL_CALL_OPEN_TAGS: [&str; 7] = [
        "<function_calls>",
        "<function_call>",
        "<tool_call>",
        "<toolcall>",
        "<tool-call>",
        "<tool>",
        "<invoke>",
    ];

    fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
        tags.iter()
            .filter_map(|tag| haystack.find(tag).map(|idx| (idx, *tag)))
            .min_by_key(|(idx, _)| *idx)
    }

    fn matching_close_tag(open_tag: &str) -> Option<&'static str> {
        match open_tag {
            "<function_calls>" => Some("</function_calls>"),
            "<function_call>" => Some("</function_call>"),
            "<tool_call>" => Some("</tool_call>"),
            "<toolcall>" => Some("</toolcall>"),
            "<tool-call>" => Some("</tool-call>"),
            "<tool>" => Some("</tool>"),
            "<invoke>" => Some("</invoke>"),
            _ => None,
        }
    }

    fn extract_first_json_end(input: &str) -> Option<usize> {
        let trimmed = input.trim_start();
        let trim_offset = input.len().saturating_sub(trimmed.len());

        for (byte_idx, ch) in trimmed.char_indices() {
            if ch != '{' && ch != '[' {
                continue;
            }

            let slice = &trimmed[byte_idx..];
            let mut stream =
                serde_json::Deserializer::from_str(slice).into_iter::<serde_json::Value>();
            if let Some(Ok(_value)) = stream.next() {
                let consumed = stream.byte_offset();
                if consumed > 0 {
                    return Some(trim_offset + byte_idx + consumed);
                }
            }
        }

        None
    }

    fn strip_leading_close_tags(mut input: &str) -> &str {
        loop {
            let trimmed = input.trim_start();
            if !trimmed.starts_with("</") {
                return trimmed;
            }

            let Some(close_end) = trimmed.find('>') else {
                return "";
            };
            input = &trimmed[close_end + 1..];
        }
    }

    let mut kept_segments = Vec::new();
    let mut remaining = message;

    while let Some((start, open_tag)) = find_first_tag(remaining, &TOOL_CALL_OPEN_TAGS) {
        let before = &remaining[..start];
        if !before.is_empty() {
            kept_segments.push(before.to_string());
        }

        let Some(close_tag) = matching_close_tag(open_tag) else {
            break;
        };
        let after_open = &remaining[start + open_tag.len()..];

        if let Some(close_idx) = after_open.find(close_tag) {
            remaining = &after_open[close_idx + close_tag.len()..];
            continue;
        }

        if let Some(consumed_end) = extract_first_json_end(after_open) {
            remaining = strip_leading_close_tags(&after_open[consumed_end..]);
            continue;
        }

        kept_segments.push(remaining[start..].to_string());
        remaining = "";
        break;
    }

    if !remaining.is_empty() {
        kept_segments.push(remaining.to_string());
    }

    let mut result = kept_segments.concat();

    // Clean up any resulting blank lines (but preserve paragraphs)
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

pub(crate) fn sanitize_channel_response(response: &str, tools: &[Box<dyn Tool>]) -> String {
    let without_think = strip_think_blocks(response);
    let without_tool_tags = strip_tool_call_tags(&without_think);
    let known_tool_names: HashSet<String> = tools
        .iter()
        .map(|tool| tool.name().to_ascii_lowercase())
        .collect();
    strip_isolated_tool_json_artifacts(&without_tool_tags, &known_tool_names)
}

/// Strip `<think>…</think>` blocks from final model output.
/// Defense-in-depth: the provider layer strips these on parse, but this
/// ensures no reasoning leaks to any channel even if a provider path misses it.
fn strip_think_blocks(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

fn is_tool_call_payload(value: &serde_json::Value, known_tool_names: &HashSet<String>) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };

    let (name, has_args) =
        if let Some(function) = object.get("function").and_then(|f| f.as_object()) {
            (
                function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| object.get("name").and_then(|v| v.as_str())),
                function.contains_key("arguments")
                    || function.contains_key("parameters")
                    || object.contains_key("arguments")
                    || object.contains_key("parameters"),
            )
        } else {
            (
                object.get("name").and_then(|v| v.as_str()),
                object.contains_key("arguments") || object.contains_key("parameters"),
            )
        };

    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return false;
    };

    has_args && known_tool_names.contains(&name.to_ascii_lowercase())
}

fn is_tool_result_payload(
    object: &serde_json::Map<String, serde_json::Value>,
    saw_tool_call_payload: bool,
) -> bool {
    if !saw_tool_call_payload || !object.contains_key("result") {
        return false;
    }

    object.keys().all(|key| {
        matches!(
            key.as_str(),
            "result" | "id" | "tool_call_id" | "name" | "tool"
        )
    })
}

fn sanitize_tool_json_value(
    value: &serde_json::Value,
    known_tool_names: &HashSet<String>,
    saw_tool_call_payload: bool,
) -> Option<(String, bool)> {
    if is_tool_call_payload(value, known_tool_names) {
        return Some((String::new(), true));
    }

    if let Some(array) = value.as_array() {
        if !array.is_empty()
            && array
                .iter()
                .all(|item| is_tool_call_payload(item, known_tool_names))
        {
            return Some((String::new(), true));
        }
        return None;
    }

    let object = value.as_object()?;

    if let Some(tool_calls) = object.get("tool_calls").and_then(|value| value.as_array()) {
        if !tool_calls.is_empty()
            && tool_calls
                .iter()
                .all(|call| is_tool_call_payload(call, known_tool_names))
        {
            let content = object
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            return Some((content, true));
        }
    }

    if is_tool_result_payload(object, saw_tool_call_payload) {
        return Some((String::new(), false));
    }

    None
}

fn is_line_isolated_json_segment(message: &str, start: usize, end: usize) -> bool {
    let line_start = message[..start].rfind('\n').map_or(0, |idx| idx + 1);
    let line_end = message[end..]
        .find('\n')
        .map_or(message.len(), |idx| end + idx);

    message[line_start..start].trim().is_empty() && message[end..line_end].trim().is_empty()
}

pub(super) fn strip_isolated_tool_json_artifacts(
    message: &str,
    known_tool_names: &HashSet<String>,
) -> String {
    let mut cleaned = String::with_capacity(message.len());
    let mut cursor = 0usize;
    let mut saw_tool_call_payload = false;

    while cursor < message.len() {
        let Some(rel_start) = message[cursor..].find(['{', '[']) else {
            cleaned.push_str(&message[cursor..]);
            break;
        };

        let start = cursor + rel_start;
        cleaned.push_str(&message[cursor..start]);

        let candidate = &message[start..];
        let mut stream =
            serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();

        if let Some(Ok(value)) = stream.next() {
            let consumed = stream.byte_offset();
            if consumed > 0 {
                let end = start + consumed;
                if is_line_isolated_json_segment(message, start, end) {
                    if let Some((replacement, marks_tool_call)) =
                        sanitize_tool_json_value(&value, known_tool_names, saw_tool_call_payload)
                    {
                        if marks_tool_call {
                            saw_tool_call_payload = true;
                        }
                        if !replacement.trim().is_empty() {
                            cleaned.push_str(replacement.trim());
                        }
                        cursor = end;
                        continue;
                    }
                }
            }
        }

        let Some(ch) = message[start..].chars().next() else {
            break;
        };
        cleaned.push(ch);
        cursor = start + ch.len_utf8();
    }

    let mut result = cleaned.replace("\r\n", "\n");
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}
