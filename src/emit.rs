use crate::log;
use crate::source::Source;
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
    source: Source,
) -> Vec<Value> {
    let user_text_raw = transcript::extract_text(transcript::get_content(&turn.user_msg));
    let (user_text, user_text_meta) = truncate(&user_text_raw);

    let last_assistant = turn.assistant_msgs.last().unwrap();
    let assistant_text_raw = transcript::extract_text(transcript::get_content(last_assistant));
    let (assistant_text, assistant_text_meta) = truncate(&assistant_text_raw);

    let model = transcript::get_model(&turn.assistant_msgs[0]);
    let tool_calls = tool_calls_from_assistants(&turn.assistant_msgs);

    // A turn can hold several assistant messages (e.g. a tool-call loop before
    // the final answer); each carries its own per-call usage, so sum across
    // all of them rather than taking just the first.
    let total_usage = turn.assistant_msgs.iter().fold(
        None::<transcript::Usage>,
        |acc, m| match (acc, transcript::get_usage(m)) {
            (None, None) => None,
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (Some(a), Some(b)) => Some(a + b),
        },
    );

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
            "name": format!("{} - Turn {turn_num}", source.trace_name_prefix()),
            "sessionId": session_id,
            "input": json!({"role": "user", "content": user_text}),
            "output": json!({"role": "assistant", "content": assistant_text}),
            "tags": tags,
            "metadata": {
                "source": source.as_str(),
                "session_id": session_id,
                "turn_number": turn_num,
                "transcript_path": transcript_path.to_string_lossy(),
                "user_text": user_text_meta,
            },
        }
    }));

    // 2. generation-create for the LLM response
    let gen_id = uuid::Uuid::new_v4().to_string();
    let mut gen_body = serde_json::Map::new();
    gen_body.insert("id".to_string(), json!(gen_id));
    gen_body.insert("traceId".to_string(), json!(trace_id));
    gen_body.insert("name".to_string(), json!("Claude Response"));
    gen_body.insert("startTime".to_string(), json!(now));
    gen_body.insert("endTime".to_string(), json!(now));
    gen_body.insert("model".to_string(), json!(model));
    gen_body.insert(
        "input".to_string(),
        json!({"role": "user", "content": user_text}),
    );
    gen_body.insert(
        "output".to_string(),
        json!({"role": "assistant", "content": assistant_text}),
    );
    gen_body.insert(
        "metadata".to_string(),
        json!({
            "assistant_text": assistant_text_meta,
            "tool_count": tool_calls.len(),
        }),
    );
    // Omitted entirely (not zero-filled) when the turn has no usage data at
    // all, so Langfuse never prices a generation at $0 for a source that
    // simply doesn't report usage.
    if let Some(usage) = total_usage {
        gen_body.insert(
            "usageDetails".to_string(),
            json!({
                "input": usage.input_tokens,
                "output": usage.output_tokens,
                "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                "cache_read_input_tokens": usage.cache_read_input_tokens,
            }),
        );
    }
    events.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "generation-create",
        "timestamp": now,
        "body": Value::Object(gen_body),
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

/// Fire-and-forget: fork the process. Parent returns immediately,
/// child sends the HTTP request via ureq and exits.
///
/// With CODE_TRACE_SYNC_SEND=1 the send happens inline instead, so process
/// exit guarantees delivery — tests rely on this for exact "nothing was
/// sent" assertions against a forked child they cannot wait on.
pub fn send_batch_fire_and_forget(config: &LangfuseConfig, events: Vec<Value>) {
    let url = format!("{}/api/public/ingestion", config.host);

    if std::env::var("CODE_TRACE_SYNC_SEND").as_deref() == Ok("1") {
        send_batch_blocking(config, &url, events);
        return;
    }

    // Fork: parent returns, child does the HTTP call
    #[cfg(unix)]
    {
        let pid = unsafe { libc::fork() };
        match pid {
            -1 => {
                log::error("fork() failed");
            }
            0 => {
                // Child process — detach from parent's session
                unsafe { libc::setsid() };
                send_batch_blocking(config, &url, events);
                std::process::exit(0);
            }
            _ => {
                // Parent — return immediately
            }
        }
    }

    #[cfg(not(unix))]
    {
        // Fallback: blocking send in the same process
        send_batch_blocking(config, &url, events);
    }
}

fn send_batch_blocking(config: &LangfuseConfig, url: &str, events: Vec<Value>) {
    use base64::Engine;

    let body = json!({
        "batch": events,
        "metadata": {}
    });

    let credentials = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", config.public_key, config.secret_key));

    match ureq::post(url)
        .header("Authorization", &format!("Basic {credentials}"))
        .send_json(&body)
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                log::debug(&format!("Langfuse API: {status}"));
            } else {
                let text = resp.into_body().read_to_string().unwrap_or_default();
                log::error(&format!("Langfuse API error: {status} {text}"));
            }
        }
        Err(e) => {
            log::error(&format!("Langfuse HTTP request failed: {e}"));
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
        use crate::source::Source;
        let turn = make_simple_turn();
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()], Source::ClaudeCode);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "trace-create");
        assert_eq!(events[1]["type"], "generation-create");
    }

    #[test]
    fn trace_event_has_tags() {
        use crate::source::Source;
        let turn = make_simple_turn();
        let tags = vec!["claude-code".to_string(), "repo:myrepo".to_string()];
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &tags, Source::ClaudeCode);
        let trace_tags = events[0]["body"]["tags"].as_array().unwrap();
        assert_eq!(trace_tags.len(), 2);
        assert_eq!(trace_tags[0], "claude-code");
        assert_eq!(trace_tags[1], "repo:myrepo");
    }

    #[test]
    fn builds_tool_spans() {
        use crate::source::Source;
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
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()], Source::ClaudeCode);
        assert_eq!(events.len(), 3);
        assert_eq!(events[2]["type"], "span-create");
        assert_eq!(events[2]["body"]["name"], "Tool: Bash");
    }

    #[test]
    fn opencode_source_uses_correct_trace_name() {
        use crate::source::Source;
        let turn = make_simple_turn();
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["opencode".to_string()], Source::Opencode);
        assert_eq!(events[0]["body"]["name"], "OpenCode - Turn 1");
        assert_eq!(events[0]["body"]["metadata"]["source"], "opencode");
    }

    #[test]
    fn generation_event_has_no_usage_details_without_usage_block() {
        use crate::source::Source;
        let turn = make_simple_turn();
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()], Source::ClaudeCode);
        assert!(events[1]["body"].get("usageDetails").is_none());
    }

    #[test]
    fn generation_event_carries_usage_details() {
        use crate::source::Source;
        let turn = Turn {
            user_msg: json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            assistant_msgs: vec![json!({
                "type":"assistant",
                "message":{
                    "id":"m1",
                    "role":"assistant",
                    "model":"claude-sonnet-4-20250514",
                    "content":[{"type":"text","text":"Hi there"}],
                    "usage":{
                        "input_tokens":10,
                        "output_tokens":20,
                        "cache_creation_input_tokens":5,
                        "cache_read_input_tokens":3,
                    },
                }
            })],
            tool_results_by_id: HashMap::new(),
        };
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()], Source::ClaudeCode);
        let usage = &events[1]["body"]["usageDetails"];
        assert_eq!(usage["input"], 10);
        assert_eq!(usage["output"], 20);
        assert_eq!(usage["cache_creation_input_tokens"], 5);
        assert_eq!(usage["cache_read_input_tokens"], 3);
    }

    #[test]
    fn generation_event_sums_usage_across_assistant_messages() {
        use crate::source::Source;
        let turn = Turn {
            user_msg: json!({"type":"user","message":{"role":"user","content":"Do something"}}),
            assistant_msgs: vec![
                json!({
                    "type":"assistant",
                    "message":{
                        "id":"m1",
                        "role":"assistant",
                        "model":"claude",
                        "content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}],
                        "usage":{"input_tokens":10,"output_tokens":5},
                    }
                }),
                json!({
                    "type":"assistant",
                    "message":{
                        "id":"m2",
                        "role":"assistant",
                        "model":"claude",
                        "content":[{"type":"text","text":"Done"}],
                        "usage":{"input_tokens":15,"output_tokens":8},
                    }
                }),
            ],
            tool_results_by_id: HashMap::new(),
        };
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()], Source::ClaudeCode);
        let usage = &events[1]["body"]["usageDetails"];
        assert_eq!(usage["input"], 25);
        assert_eq!(usage["output"], 13);
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
