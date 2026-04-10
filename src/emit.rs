use crate::log;
use crate::transcript;
use crate::turns::Turn;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_CHARS: usize = 20_000;

fn truncate(s: &str) -> (String, Value) {
    let orig_len = s.len();
    if orig_len <= MAX_CHARS {
        return (
            s.to_string(),
            json!({"truncated": false, "orig_len": orig_len}),
        );
    }
    let head: String = s.chars().take(MAX_CHARS).collect();
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    (
        head.clone(),
        json!({
            "truncated": true,
            "orig_len": orig_len,
            "kept_len": head.len(),
            "sha256": format!("{:x}", hasher.finalize()),
        }),
    )
}

fn tool_calls_from_assistants(assistant_msgs: &[Value]) -> Vec<Value> {
    let mut calls = Vec::new();
    for am in assistant_msgs {
        for tu in transcript::iter_tool_uses(transcript::get_content(am)) {
            let id = tu
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = tu
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let input = tu.get("input").cloned().unwrap_or(json!({}));
            calls.push(json!({
                "id": id,
                "name": name,
                "input": input,
            }));
        }
    }
    calls
}

/// Build a batch of ingestion events for one turn.
pub fn build_ingestion_batch(
    session_id: &str,
    turn_num: u32,
    turn: &Turn,
    transcript_path: &Path,
    tags: &[String],
) -> Vec<Value> {
    let user_text_raw = transcript::extract_text(transcript::get_content(&turn.user_msg));
    let (user_text, user_text_meta) = truncate(&user_text_raw);

    let last_assistant = turn.assistant_msgs.last().unwrap();
    let assistant_text_raw = transcript::extract_text(transcript::get_content(last_assistant));
    let (assistant_text, assistant_text_meta) = truncate(&assistant_text_raw);

    let model = transcript::get_model(&turn.assistant_msgs[0]);
    let tool_calls = tool_calls_from_assistants(&turn.assistant_msgs);

    let trace_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut events: Vec<Value> = Vec::new();

    // 1. trace-create
    events.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "trace-create",
        "timestamp": now,
        "body": {
            "id": trace_id,
            "timestamp": now,
            "name": format!("Claude Code - Turn {turn_num}"),
            "sessionId": session_id,
            "input": json!({"role": "user", "content": user_text}),
            "output": json!({"role": "assistant", "content": assistant_text}),
            "tags": tags,
            "metadata": {
                "source": "claude-code",
                "session_id": session_id,
                "turn_number": turn_num,
                "transcript_path": transcript_path.to_string_lossy(),
                "user_text": user_text_meta,
            },
        }
    }));

    // 2. generation-create for the LLM response
    let gen_id = uuid::Uuid::new_v4().to_string();
    events.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "generation-create",
        "timestamp": now,
        "body": {
            "id": gen_id,
            "traceId": trace_id,
            "name": "Claude Response",
            "startTime": now,
            "endTime": now,
            "model": model,
            "input": json!({"role": "user", "content": user_text}),
            "output": json!({"role": "assistant", "content": assistant_text}),
            "metadata": {
                "assistant_text": assistant_text_meta,
                "tool_count": tool_calls.len(),
            },
        }
    }));

    // 3. span-create for each tool call
    for tc in &tool_calls {
        let tool_id_str = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let tool_name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");

        let input_val = tc.get("input").cloned().unwrap_or(json!({}));
        let (input_display, input_meta) = if let Some(s) = input_val.as_str() {
            let (trunc, meta) = truncate(s);
            (json!(trunc), Some(meta))
        } else {
            (input_val.clone(), None)
        };

        let output_val = turn
            .tool_results_by_id
            .get(tool_id_str)
            .cloned();

        let (output_display, output_meta) = match output_val {
            Some(v) => {
                let s = if v.is_string() {
                    v.as_str().unwrap().to_string()
                } else {
                    serde_json::to_string(&v).unwrap_or_default()
                };
                let (trunc, meta) = truncate(&s);
                (Some(json!(trunc)), Some(meta))
            }
            None => (None, None),
        };

        events.push(json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "type": "span-create",
            "timestamp": now,
            "body": {
                "id": uuid::Uuid::new_v4().to_string(),
                "traceId": trace_id,
                "parentObservationId": gen_id,
                "name": format!("Tool: {tool_name}"),
                "startTime": now,
                "endTime": now,
                "input": input_display,
                "output": output_display,
                "metadata": {
                    "tool_name": tool_name,
                    "tool_id": tool_id_str,
                    "input_meta": input_meta,
                    "output_meta": output_meta,
                },
            }
        }));
    }

    events
}

pub struct LangfuseConfig {
    pub host: String,
    pub public_key: String,
    pub secret_key: String,
}

/// Fire-and-forget: spawn a detached child process that sends the HTTP request.
/// The parent returns immediately.
pub fn send_batch_fire_and_forget(config: &LangfuseConfig, events: Vec<Value>) {
    let url = format!("{}/api/public/ingestion", config.host);
    let body = json!({
        "batch": events,
        "metadata": {}
    });
    let body_str = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => {
            log::error(&format!("Failed to serialize batch: {e}"));
            return;
        }
    };

    let auth = format!("{}:{}", config.public_key, config.secret_key);
    match std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-u",
            &auth,
            "-d",
            &body_str,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_child) => {
            log::debug("Spawned curl for fire-and-forget send");
        }
        Err(e) => {
            log::error(&format!("Failed to spawn curl: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turns::Turn;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_simple_turn() -> Turn {
        Turn {
            user_msg: json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            assistant_msgs: vec![json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi there"}]}})],
            tool_results_by_id: HashMap::new(),
        }
    }

    #[test]
    fn builds_trace_and_generation_events() {
        let turn = make_simple_turn();
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "trace-create");
        assert_eq!(events[1]["type"], "generation-create");
    }

    #[test]
    fn trace_event_has_tags() {
        let turn = make_simple_turn();
        let tags = vec!["claude-code".to_string(), "repo:myrepo".to_string()];
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &tags);
        let trace_tags = events[0]["body"]["tags"].as_array().unwrap();
        assert_eq!(trace_tags.len(), 2);
        assert_eq!(trace_tags[0], "claude-code");
        assert_eq!(trace_tags[1], "repo:myrepo");
    }

    #[test]
    fn builds_tool_spans() {
        let mut tool_results = HashMap::new();
        tool_results.insert("tu_1".to_string(), json!("file1.txt\nfile2.txt"));
        let turn = Turn {
            user_msg: json!({"type":"user","message":{"role":"user","content":"list files"}}),
            assistant_msgs: vec![json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[
                {"type":"text","text":"Let me check"},
                {"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}
            ]}})],
            tool_results_by_id: tool_results,
        };
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()]);
        assert_eq!(events.len(), 3);
        assert_eq!(events[2]["type"], "span-create");
        assert_eq!(events[2]["body"]["name"], "Tool: Bash");
    }

    #[test]
    fn truncates_long_text() {
        let long = "x".repeat(30_000);
        let (truncated, meta) = truncate(&long);
        assert_eq!(truncated.len(), MAX_CHARS);
        assert_eq!(meta["truncated"], true);
        assert_eq!(meta["orig_len"], 30_000);
    }
}
