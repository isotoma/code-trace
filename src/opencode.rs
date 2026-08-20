use serde_json::{json, Value};
use std::collections::HashMap;

pub fn normalize_opencode_messages(messages: Vec<Value>) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    let mut pending_tool_results: HashMap<String, Value> = HashMap::new();
    let mut pending_assistant_idx: Option<usize> = None;

    for msg in messages {
        let role = get_opencode_role(&msg);
        let parts = get_opencode_parts(&msg);

        if role == Some("user") {
            let mut user_content: Vec<Value> = Vec::new();

            for part in &parts {
                if part.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                    let tool_use_id = part
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let content = part.get("content").cloned().unwrap_or(json!(null));
                    pending_tool_results.insert(tool_use_id.to_string(), content);
                }
            }

            if !pending_tool_results.is_empty() {
                let mut tr_arr: Vec<Value> = Vec::new();
                for (tid, content) in pending_tool_results.drain() {
                    tr_arr.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tid,
                        "content": content,
                    }));
                }
                user_content.extend(tr_arr);
            }

            let text_parts: Vec<Value> = parts
                .iter()
                .filter(|p| p.get("type").and_then(|v| v.as_str()) == Some("text"))
                .map(|p| {
                    let text = p.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    json!({ "type": "text", "text": text })
                })
                .collect();
            if !text_parts.is_empty() {
                user_content.extend(text_parts);
            }

            if !user_content.is_empty() {
                result.push(json!({
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": user_content,
                    }
                }));
            }
            pending_assistant_idx = None;
            continue;
        }

        if role == Some("assistant") {
            let id = msg
                .get("info")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let info = msg.get("info").unwrap_or(&Value::Null);
            // OpenCode v2 puts modelID at top level of info; v1 nests it
            // under info.metadata.assistant.modelID. Fall back to "opencode".
            let model = info
                .get("modelID")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    info.get("metadata")
                        .and_then(|m| m.get("assistant"))
                        .and_then(|a| a.get("modelID"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("opencode");

            let mut content_parts: Vec<Value> = Vec::new();
            let mut has_tool_use = false;
            let mut current_msg_tool_uses: Vec<String> = Vec::new();

            for part in &parts {
                let part_type = part.get("type").and_then(|v| v.as_str());
                match part_type {
                    Some("text") => {
                        let text = part.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        content_parts.push(json!({ "type": "text", "text": text }));
                    }
                    Some("tool_use") => {
                        has_tool_use = true;
                        let tool_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        current_msg_tool_uses.push(tool_id.to_string());
                        let tool_name = part.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let tool_input = part.get("input").cloned().unwrap_or(json!({}));
                        content_parts.push(json!({
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tool_name,
                            "input": tool_input,
                        }));
                    }
                    Some("tool_result") => {
                        let tool_use_id = part
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let content = part.get("content").cloned().unwrap_or(json!(null));
                        pending_tool_results.insert(tool_use_id.to_string(), content);
                    }
                    _ => {}
                }
            }

            let assistant_msg_idx = result.len();
            if has_tool_use {
                pending_assistant_idx = Some(assistant_msg_idx);
            }

            // OpenCode v2 carries token usage in info.tokens (with
            // cache.read / cache.write); v1 nests it under
            // info.metadata.assistant.tokens. Translate to the shape
            // transcript::get_usage expects so emit.rs can price the turn.
            let usage = extract_opencode_usage(info);
            let mut message = serde_json::Map::new();
            message.insert("id".to_string(), json!(id));
            message.insert("role".to_string(), json!("assistant"));
            message.insert("model".to_string(), json!(model));
            message.insert("content".to_string(), json!(content_parts));
            if let Some(u) = usage {
                message.insert("usage".to_string(), u);
            }

            let assistant_msg = json!({
                "type": "assistant",
                "message": Value::Object(message),
            });

            if !pending_tool_results.is_empty() {
                if let Some(prev_idx) = pending_assistant_idx {
                    if prev_idx < result.len() {
                        let prev_has_tool_use = result[prev_idx]["message"]["content"]
                            .as_array()
                            .map(|arr| {
                                arr.iter().any(|p| {
                                    p.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                                })
                            })
                            .unwrap_or(false);
                        if prev_has_tool_use {
                            let mut tr_arr: Vec<Value> = Vec::new();
                            for (tid, content) in pending_tool_results.drain() {
                                tr_arr.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": tid,
                                    "content": content,
                                }));
                            }
                            result[prev_idx]["message"]["tool_results"] = json!(tr_arr);
                        }
                    }
                }
            }

            result.push(assistant_msg);

            if !has_tool_use && !pending_tool_results.is_empty() {
                pending_assistant_idx = Some(assistant_msg_idx);
            }
            continue;
        }

        result.push(msg);
    }

    result
}

fn get_opencode_role(msg: &Value) -> Option<&str> {
    msg.get("info")
        .and_then(|v| v.get("role"))
        .and_then(|v| v.as_str())
        .filter(|r| *r == "user" || *r == "assistant")
}

fn get_opencode_parts(msg: &Value) -> Vec<Value> {
    msg.get("parts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Pull token usage out of an OpenCode assistant info block and translate it
/// to the `message.usage` shape `transcript::get_usage` consumes. Returns
/// None when no tokens block is present, so emit.rs omits usageDetails
/// rather than pricing the generation at $0.
///
/// Handles both v2 (`info.tokens`) and v1 (`info.metadata.assistant.tokens`).
fn extract_opencode_usage(info: &Value) -> Option<Value> {
    let tokens = info
        .get("tokens")
        .or_else(|| {
            info.get("metadata")
                .and_then(|m| m.get("assistant"))
                .and_then(|a| a.get("tokens"))
        })?;
    let get = |k: &str| tokens.get(k).and_then(Value::as_u64).unwrap_or(0);
    let cache_read = tokens
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_write = tokens
        .get("cache")
        .and_then(|c| c.get("write"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some(json!({
        "input_tokens": get("input"),
        "output_tokens": get("output"),
        "cache_creation_input_tokens": cache_write,
        "cache_read_input_tokens": cache_read,
        "reasoning_tokens": get("reasoning"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_user_message_with_text() {
        let msgs = vec![json!({
            "info": { "id": "msg1", "role": "user" },
            "parts": [{ "type": "text", "text": "Hello" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0]["type"], "user");
        assert_eq!(normalized[0]["message"]["role"], "user");
    }

    #[test]
    fn normalizes_assistant_message_with_tool_use() {
        let msgs = vec![json!({
            "info": { "id": "msg2", "role": "assistant", "modelID": "claude" },
            "parts": [
                { "type": "text", "text": "Let me check" },
                { "type": "tool_use", "id": "tu1", "name": "Bash", "input": { "command": "ls" } }
            ]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0]["type"], "assistant");
        assert_eq!(normalized[0]["message"]["model"], "claude");
        assert_eq!(normalized[0]["message"]["content"][1]["type"], "tool_use");
        assert_eq!(normalized[0]["message"]["content"][1]["name"], "Bash");
    }

    #[test]
    fn reads_model_from_v2_modelid() {
        let msgs = vec![json!({
            "info": { "id": "msg2", "role": "assistant", "modelID": "claude-opus-4-1" },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized[0]["message"]["model"], "claude-opus-4-1");
    }

    #[test]
    fn reads_model_from_v1_metadata_assistant_modelid() {
        let msgs = vec![json!({
            "info": {
                "id": "msg2",
                "role": "assistant",
                "metadata": { "assistant": { "modelID": "claude-3-5-sonnet" } }
            },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized[0]["message"]["model"], "claude-3-5-sonnet");
    }

    #[test]
    fn falls_back_to_opencode_when_model_missing() {
        let msgs = vec![json!({
            "info": { "id": "msg2", "role": "assistant" },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized[0]["message"]["model"], "opencode");
    }

    #[test]
    fn extracts_v2_tokens_into_usage() {
        let msgs = vec![json!({
            "info": {
                "id": "msg2", "role": "assistant", "modelID": "claude",
                "tokens": { "input": 100, "output": 50, "reasoning": 7, "cache": { "read": 10, "write": 20 } }
            },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        let usage = &normalized[0]["message"]["usage"];
        assert_eq!(usage["input_tokens"], 100);
        assert_eq!(usage["output_tokens"], 50);
        assert_eq!(usage["cache_read_input_tokens"], 10);
        assert_eq!(usage["cache_creation_input_tokens"], 20);
        assert_eq!(usage["reasoning_tokens"], 7);
    }

    #[test]
    fn omits_usage_when_no_tokens_block() {
        let msgs = vec![json!({
            "info": { "id": "msg2", "role": "assistant", "modelID": "claude" },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert!(normalized[0]["message"].get("usage").is_none());
    }

    #[test]
    fn extracts_v1_tokens_without_reasoning() {
        // v1 nests token usage under info.metadata.assistant.tokens and
        // predates the `reasoning` key, so reasoning_tokens must default to 0.
        let msgs = vec![json!({
            "info": {
                "id": "msg2", "role": "assistant",
                "metadata": { "assistant": { "modelID": "claude-3-5-sonnet", "tokens": { "input": 5, "output": 9, "cache": { "read": 1, "write": 2 } } } }
            },
            "parts": [{ "type": "text", "text": "hi" }]
        })];
        let normalized = normalize_opencode_messages(msgs);
        let usage = &normalized[0]["message"]["usage"];
        assert_eq!(usage["reasoning_tokens"], 0);
        assert_eq!(usage["input_tokens"], 5);
        assert_eq!(usage["output_tokens"], 9);
    }

    #[test]
    fn pending_tool_results_attached_to_previous_assistant() {
        let msgs = vec![
            json!({
                "info": { "id": "msg1", "role": "assistant" },
                "parts": [
                    { "type": "text", "text": "Running command" },
                    { "type": "tool_use", "id": "tu1", "name": "Bash", "input": {} }
                ]
            }),
            json!({
                "info": { "id": "msg2", "role": "assistant" },
                "parts": [
                    { "type": "tool_result", "tool_use_id": "tu1", "content": "file1.txt" }
                ]
            }),
        ];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized.len(), 2);
        let tool_results = normalized[0]["message"]["tool_results"].as_array().unwrap();
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0]["tool_use_id"], "tu1");
        assert_eq!(tool_results[0]["content"], "file1.txt");
    }
}
