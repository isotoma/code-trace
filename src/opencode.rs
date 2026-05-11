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

            let model = msg
                .get("info")
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
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

            let assistant_msg = json!({
                "type": "assistant",
                "message": {
                    "id": id,
                    "role": "assistant",
                    "model": model,
                    "content": content_parts,
                }
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
            "info": { "id": "msg2", "role": "assistant", "model": "claude" },
            "parts": [
                { "type": "text", "text": "Let me check" },
                { "type": "tool_use", "id": "tu1", "name": "Bash", "input": { "command": "ls" } }
            ]
        })];
        let normalized = normalize_opencode_messages(msgs);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0]["type"], "assistant");
        assert_eq!(normalized[0]["message"]["content"][1]["type"], "tool_use");
        assert_eq!(normalized[0]["message"]["content"][1]["name"], "Bash");
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
