use serde_json::{json, Value};
use std::collections::HashMap;

pub fn normalize_pi_agent_messages(messages: Vec<Value>) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    let mut pending_tool_results: HashMap<String, Value> = HashMap::new();
    let mut pending_assistant_idx: Option<usize> = None;

    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str());

        match role {
            Some("user") => {
                // Flush any pending tool results onto the previous assistant message
                if !pending_tool_results.is_empty() {
                    if let Some(prev_idx) = pending_assistant_idx {
                        if prev_idx < result.len() {
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
                pending_assistant_idx = None;

                let user_content = normalize_user_content(&msg);
                if !user_content.is_empty() {
                    result.push(json!({
                        "type": "user",
                        "message": {
                            "role": "user",
                            "content": user_content,
                        }
                    }));
                }
            }

            Some("assistant") => {
                // Attach any pending tool results to the previous assistant message
                // before emitting the new one
                if !pending_tool_results.is_empty() {
                    if let Some(prev_idx) = pending_assistant_idx {
                        if prev_idx < result.len() {
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

                let id = msg
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

                let model = msg
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("pi");

                let mut content_parts: Vec<Value> = Vec::new();
                let mut has_tool_use = false;

                if let Some(blocks) = msg.get("content").and_then(|v| v.as_array()) {
                    for block in blocks {
                        let block_type = block.get("type").and_then(|v| v.as_str());
                        match block_type {
                            Some("text") => {
                                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                content_parts.push(json!({ "type": "text", "text": text }));
                            }
                            Some("thinking") => {
                                // Skip thinking blocks
                            }
                            Some("toolCall") => {
                                has_tool_use = true;
                                let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                                let tool_input = block.get("arguments").cloned().unwrap_or(json!({}));
                                content_parts.push(json!({
                                    "type": "tool_use",
                                    "id": tool_id,
                                    "name": tool_name,
                                    "input": tool_input,
                                }));
                            }
                            _ => {}
                        }
                    }
                }

                let assistant_msg_idx = result.len();
                if has_tool_use {
                    pending_assistant_idx = Some(assistant_msg_idx);
                }

                result.push(json!({
                    "type": "assistant",
                    "message": {
                        "id": id,
                        "role": "assistant",
                        "model": model,
                        "content": content_parts,
                    }
                }));
            }

            Some("toolResult") => {
                let tool_call_id = msg
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = msg.get("content").cloned().unwrap_or(json!([]));
                pending_tool_results.insert(tool_call_id.to_string(), content);
            }

            // Skip: session, compaction, branch_summary, custom, label,
            // model_change, thinking_level_change, session_info, etc.
            _ => {}
        }
    }

    // Flush any remaining pending tool results
    if !pending_tool_results.is_empty() {
        if let Some(prev_idx) = pending_assistant_idx {
            if prev_idx < result.len() {
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

    result
}

fn normalize_user_content(msg: &Value) -> Vec<Value> {
    match msg.get("content") {
        Some(Value::String(s)) => {
            vec![json!({ "type": "text", "text": s })]
        }
        Some(Value::Array(blocks)) => {
            blocks
                .iter()
                .filter_map(|block| {
                    let block_type = block.get("type").and_then(|v| v.as_str());
                    if block_type == Some("text") {
                        let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        Some(json!({ "type": "text", "text": text }))
                    } else {
                        None
                    }
                })
                .collect()
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_only_user_message_string() {
        let msgs = vec![json!({ "role": "user", "content": "Hello" })];
        let out = normalize_pi_agent_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "user");
        assert_eq!(out[0]["message"]["role"], "user");
        assert_eq!(out[0]["message"]["content"][0]["type"], "text");
        assert_eq!(out[0]["message"]["content"][0]["text"], "Hello");
    }

    #[test]
    fn text_only_user_message_array_content() {
        let msgs = vec![json!({
            "role": "user",
            "content": [{ "type": "text", "text": "Hello" }]
        })];
        let out = normalize_pi_agent_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "user");
        assert_eq!(out[0]["message"]["role"], "user");
        assert_eq!(out[0]["message"]["content"][0]["type"], "text");
        assert_eq!(out[0]["message"]["content"][0]["text"], "Hello");
    }

    #[test]
    fn assistant_message_with_tool_use() {
        let msgs = vec![json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Let me check" },
                { "type": "toolCall", "id": "tc1", "name": "bash", "arguments": { "command": "ls" } }
            ],
            "model": "claude"
        })];
        let out = normalize_pi_agent_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "assistant");
        let content = out[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Let me check");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["name"], "bash");
        assert_eq!(content[1]["input"]["command"], "ls");
    }

    #[test]
    fn tool_result_attached_to_previous_assistant() {
        // toolResult entries are not emitted as separate messages; they are
        // attached to the previous assistant message's tool_results field.
        let msgs = vec![
            json!({
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Running command" },
                    { "type": "toolCall", "id": "tc1", "name": "bash", "arguments": {} }
                ],
                "model": "claude"
            }),
            json!({
                "role": "toolResult",
                "toolCallId": "tc1",
                "toolName": "bash",
                "content": [{ "type": "text", "text": "file1.txt" }],
                "isError": false
            }),
        ];
        let out = normalize_pi_agent_messages(msgs);
        // toolResult is attached to the assistant message, not a separate entry
        assert_eq!(out.len(), 1);
        let tool_results = out[0]["message"]["tool_results"].as_array().unwrap();
        assert_eq!(tool_results.len(), 1);
        assert_eq!(tool_results[0]["tool_use_id"], "tc1");
    }

    #[test]
    fn skips_thinking_blocks() {
        let msgs = vec![json!({
            "role": "assistant",
            "content": [
                { "type": "thinking", "thinking": "internal thoughts" },
                { "type": "text", "text": "Result" }
            ],
            "model": "claude"
        })];
        let out = normalize_pi_agent_messages(msgs);
        assert_eq!(out.len(), 1);
        let content = out[0]["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn skips_unknown_roles() {
        let msgs = vec![
            json!({ "role": "session", "data": {} }),
            json!({ "role": "compaction", "data": {} }),
            json!({ "role": "user", "content": "Hi" }),
        ];
        let out = normalize_pi_agent_messages(msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "user");
    }
}
