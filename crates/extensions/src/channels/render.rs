//! Shared helpers for IM outbound renderers.

use moray_core::ChatCompletionResponseChunk;
use serde_json::Value;

/// Strip `<think>…</think>` from streamed completion chunks.
pub fn strip_think_tags(buf: &mut String, incoming: &str) -> String {
    buf.push_str(incoming);
    let mut out = String::with_capacity(buf.len());
    let bytes = buf.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        if bytes[i] == b'<' {
            let remaining = &buf[i..];
            let cmp_len = remaining.len().min(21);
            let lower: String = remaining[..cmp_len].to_ascii_lowercase();
            if lower.starts_with("<think>") {
                i += 21;
                continue;
            }
            if lower.starts_with("</think>") {
                i += 22;
                continue;
            }
            let could_be_open = "<think>".starts_with(lower.as_str());
            let could_be_close = "</think>".starts_with(lower.as_str());
            if (could_be_open || could_be_close) && cmp_len < 21 {
                break;
            }
            out.push('<');
            i += 1;
            continue;
        }
        let ch = match buf[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    *buf = buf[i..].to_string();
    out
}

pub fn chunk_text(chunk: &ChatCompletionResponseChunk) -> Option<String> {
    match chunk {
        ChatCompletionResponseChunk::TextBlock(t) => Some(t.clone()),
        // Reasoning/thinking is for the desktop UI only; IM channels show plain reply text.
        ChatCompletionResponseChunk::Think(_) => None,
        _ => None,
    }
}

pub fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_blank = false;
    for line in text.lines() {
        let blank = line.trim().is_empty();
        if blank {
            if !prev_blank && !out.is_empty() {
                out.push('\n');
            }
            prev_blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        prev_blank = false;
    }
    out
}

pub fn extract_output_preview(result: &Value, max_chars: usize) -> String {
    let text = result
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| result.get("error").and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_default();

    let truncated: String = text.chars().take(max_chars).collect();
    if truncated.chars().count() < text.chars().count() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

/// Truncate at or before `target` bytes without splitting a UTF-8 codepoint.
pub fn floor_utf8_char_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    let mut idx = target;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

pub fn friendly_tool_label(tool_name: &str, tool_display_name: &str) -> String {
    if tool_name.starts_with("path_permission:") {
        format!("path permission: {tool_display_name}")
    } else if tool_name.starts_with("seatbelt:") {
        format!("sandbox: {tool_display_name}")
    } else {
        tool_display_name.to_string()
    }
}
