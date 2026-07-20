use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::log;
use crate::state::SessionState;

/// Read new bytes from transcript since last offset. Handles partial lines via buffer.
pub fn read_new_jsonl(path: &Path, ss: &mut SessionState) -> Vec<Value> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            log::debug(&format!("read_new_jsonl open failed: {e}"));
            return vec![];
        }
    };

    if file.seek(SeekFrom::Start(ss.offset)).is_err() {
        return vec![];
    }

    let mut chunk = Vec::new();
    if file.read_to_end(&mut chunk).is_err() {
        return vec![];
    }

    let new_offset = ss.offset + chunk.len() as u64;

    if chunk.is_empty() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&chunk);
    let combined = format!("{}{}", ss.buffer, text);
    let lines: Vec<&str> = combined.split('\n').collect();

    // Last element may be incomplete
    ss.buffer = lines.last().unwrap_or(&"").to_string();
    ss.offset = new_offset;

    let mut msgs = Vec::new();
    for line in &lines[..lines.len().saturating_sub(1)] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => msgs.push(v),
            Err(_) => continue,
        }
    }

    msgs
}

/// Extract role from a transcript message. Returns "user" or "assistant" or None.
pub fn get_role(msg: &Value) -> Option<&str> {
    if let Some(t) = msg.get("type").and_then(|v| v.as_str()) {
        if t == "user" || t == "assistant" {
            return Some(t);
        }
    }
    msg.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .filter(|r| *r == "user" || *r == "assistant")
}

/// Get content from a message (message.content or top-level content).
pub fn get_content(msg: &Value) -> Option<&Value> {
    msg.get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| msg.get("content"))
}

/// Get message.id
pub fn get_message_id(msg: &Value) -> Option<&str> {
    msg.get("message")
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
}

/// Get message.model
pub fn get_model(msg: &Value) -> &str {
    msg.get("message")
        .and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("claude")
}

/// Token usage for one assistant message, mirroring Claude Code's
/// `message.usage` block.
#[derive(Debug, Default, Clone, Copy)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl std::ops::Add for Usage {
    type Output = Usage;

    fn add(self, other: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens
                + other.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens + other.cache_read_input_tokens,
        }
    }
}

/// Get message.usage (token counts Claude Code reports for one API call).
/// Returns None when the message has no usage block at all, distinguishing
/// "no usage data" from "zero usage".
pub fn get_usage(msg: &Value) -> Option<Usage> {
    let u = msg.get("message")?.get("usage")?;
    Some(Usage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_creation_input_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_input_tokens: u
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

/// Extract plain text from content (string or array of text blocks).
pub fn extract_text(content: Option<&Value>) -> String {
    let Some(c) = content else {
        return String::new();
    };
    if let Some(s) = c.as_str() {
        return s.to_string();
    }
    if let Some(arr) = c.as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|x| {
                if x.get("type").and_then(|t| t.as_str()) == Some("text") {
                    x.get("text").and_then(|t| t.as_str())
                } else if let Some(s) = x.as_str() {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        return parts.join("\n");
    }
    String::new()
}

/// Check if a message is a tool_result row.
pub fn is_tool_result(msg: &Value) -> bool {
    if get_role(msg) != Some("user") {
        return false;
    }
    let Some(content) = get_content(msg) else {
        return false;
    };
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .any(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
    }
    false
}

/// Extract tool_result blocks from content array.
pub fn iter_tool_results(content: Option<&Value>) -> Vec<&Value> {
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
        .collect()
}

/// Extract tool_use blocks from content array.
pub fn iter_tool_uses(content: Option<&Value>) -> Vec<&Value> {
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SessionState;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn reads_jsonl_incrementally() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, r#"{{"type":"user","message":{{"role":"user","content":"Hello"}}}}"#).unwrap();
        writeln!(tmp, r#"{{"type":"assistant","message":{{"id":"m1","role":"assistant","model":"claude","content":[{{"type":"text","text":"Hi"}}]}}}}"#).unwrap();

        let mut ss = SessionState::default();
        let msgs = read_new_jsonl(tmp.path(), &mut ss);
        assert_eq!(msgs.len(), 2);
        assert!(ss.offset > 0);

        // Second read with no new data returns empty
        let msgs2 = read_new_jsonl(tmp.path(), &mut ss);
        assert_eq!(msgs2.len(), 0);
    }

    #[test]
    fn get_role_top_level_type() {
        let v: Value = serde_json::from_str(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#).unwrap();
        assert_eq!(get_role(&v), Some("user"));
    }

    #[test]
    fn get_role_message_role() {
        let v: Value = serde_json::from_str(r#"{"message":{"role":"assistant","content":"hi"}}"#).unwrap();
        assert_eq!(get_role(&v), Some("assistant"));
    }

    #[test]
    fn extract_text_from_string() {
        let v: Value = serde_json::json!("hello");
        assert_eq!(extract_text(Some(&v)), "hello");
    }

    #[test]
    fn extract_text_from_content_blocks() {
        let v: Value = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "text", "text": "World"}
        ]);
        assert_eq!(extract_text(Some(&v)), "Hello\nWorld");
    }

    #[test]
    fn get_usage_reads_full_block() {
        let v: Value = serde_json::from_str(
            r#"{"message":{"role":"assistant","usage":{"input_tokens":10,"output_tokens":20,"cache_creation_input_tokens":5,"cache_read_input_tokens":3}}}"#,
        )
        .unwrap();
        let usage = get_usage(&v).unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_creation_input_tokens, 5);
        assert_eq!(usage.cache_read_input_tokens, 3);
    }

    #[test]
    fn get_usage_missing_fields_default_to_zero() {
        let v: Value =
            serde_json::from_str(r#"{"message":{"role":"assistant","usage":{"input_tokens":10}}}"#)
                .unwrap();
        let usage = get_usage(&v).unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.cache_creation_input_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn get_usage_absent_block_returns_none() {
        let v: Value = serde_json::from_str(r#"{"message":{"role":"assistant"}}"#).unwrap();
        assert!(get_usage(&v).is_none());
    }

    #[test]
    fn is_tool_result_detects_correctly() {
        let v: Value = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "output"}
            ]}
        });
        assert!(is_tool_result(&v));
    }
}
