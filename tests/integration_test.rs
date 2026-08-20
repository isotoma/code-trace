use serde_json::json;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn end_to_end_simple_transcript() {
    let mut transcript = NamedTempFile::new().unwrap();
    writeln!(transcript, r#"{{"type":"user","message":{{"role":"user","content":"Hello"}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"assistant","message":{{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{{"type":"text","text":"Hi there!"}}],"usage":{{"input_tokens":12,"output_tokens":34,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#).unwrap();

    let mut ss = code_trace::state::SessionState::default();
    let msgs = code_trace::transcript::read_new_jsonl(transcript.path(), &mut ss);
    assert_eq!(msgs.len(), 2);

    let turns = code_trace::turns::build_turns(msgs);
    assert_eq!(turns.len(), 1);

    let tags = vec!["claude-code".to_string(), "test".to_string()];
    let events = code_trace::emit::build_ingestion_batch(
        "test-session",
        1,
        &turns[0],
        transcript.path(),
        &tags,
        code_trace::source::Source::ClaudeCode,
        None,
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "trace-create");
    assert_eq!(events[0]["body"]["tags"], json!(["claude-code", "test"]));
    assert_eq!(events[0]["body"]["sessionId"], "test-session");
    assert_eq!(events[1]["type"], "generation-create");
    assert_eq!(events[1]["body"]["model"], "claude-sonnet-4-20250514");
    assert_eq!(events[1]["body"]["usageDetails"]["input"], 12);
    assert_eq!(events[1]["body"]["usageDetails"]["output"], 34);
}

#[test]
fn end_to_end_tool_transcript() {
    let mut transcript = NamedTempFile::new().unwrap();
    writeln!(transcript, r#"{{"type":"user","message":{{"role":"user","content":"List files"}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"assistant","message":{{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{{"type":"text","text":"Checking."}},{{"type":"tool_use","id":"tu_1","name":"Bash","input":{{"command":"ls"}}}}]}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"tu_1","content":"README.md"}}]}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"assistant","message":{{"id":"msg_2","role":"assistant","model":"claude-sonnet-4-20250514","content":[{{"type":"text","text":"Found README.md"}}]}}}}"#).unwrap();

    let mut ss = code_trace::state::SessionState::default();
    let msgs = code_trace::transcript::read_new_jsonl(transcript.path(), &mut ss);
    let turns = code_trace::turns::build_turns(msgs);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].tool_results_by_id.contains_key("tu_1"));

    let events = code_trace::emit::build_ingestion_batch(
        "sess",
        1,
        &turns[0],
        transcript.path(),
        &["claude-code".to_string()],
        code_trace::source::Source::ClaudeCode,
        None,
    );

    assert_eq!(events.len(), 3);
    assert_eq!(events[2]["type"], "span-create");
    assert_eq!(events[2]["body"]["name"], "Tool: Bash");
    assert_eq!(events[2]["body"]["output"], json!("README.md"));
}

#[test]
fn end_to_end_opencode_transcript() {
    let msgs = vec![
        json!({
            "info": { "id": "msg_1", "role": "user" },
            "parts": [{ "type": "text", "text": "Hello" }]
        }),
        json!({
            "info": { "id": "msg_2", "role": "assistant", "providerID": "anthropic", "modelID": "claude-sonnet-4-20250514", "tokens": { "input": 12, "output": 34, "reasoning": 6, "cache": { "read": 0, "write": 0 } } },
            "parts": [
                { "type": "text", "text": "Hi there!" },
                { "type": "tool_use", "id": "tu_1", "name": "Bash", "input": { "command": "ls" } }
            ]
        }),
        json!({
            "info": { "id": "msg_3", "role": "assistant" },
            "parts": [
                { "type": "tool_result", "tool_use_id": "tu_1", "content": "file1.txt" }
            ]
        }),
        json!({
            "info": { "id": "msg_4", "role": "assistant", "providerID": "anthropic", "modelID": "claude-sonnet-4-20250514", "tokens": { "input": 200, "output": 15, "reasoning": 9, "cache": { "read": 4, "write": 1 } } },
            "parts": [{ "type": "text", "text": "Done!" }]
        }),
    ];

    let normalized = code_trace::opencode::normalize_opencode_messages(msgs);
    let turns = code_trace::turns::build_turns(normalized);
    assert_eq!(turns.len(), 1);
    // All three assistant steps survive as separate messages in order. msg_3 is
    // the tool_result carrier (its content is folded into msg_2's
    // tool_results, but it still emits its own empty assistant message with no
    // usage). msg_4 is the final text-only step carrying its own usage.
    let assistant_ids: Vec<&str> = turns[0]
        .assistant_msgs
        .iter()
        .filter_map(|m| m["message"]["id"].as_str())
        .collect();
    assert_eq!(assistant_ids, vec!["msg_2", "msg_3", "msg_4"]);

    let tags = vec!["opencode".to_string()];
    let events = code_trace::emit::build_ingestion_batch(
        "oc-session",
        1,
        &turns[0],
        std::path::Path::new("opencode"),
        &tags,
        code_trace::source::Source::Opencode,
        None,
    );

    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["type"], "trace-create");
    assert_eq!(events[0]["body"]["name"], "OpenCode - Turn 1");
    assert_eq!(events[0]["body"]["metadata"]["source"], "opencode");
    assert_eq!(events[1]["body"]["model"], "claude-sonnet-4-20250514");
    // Summed usage across the two assistant steps with usage (msg_2 + msg_4):
    // input 12+200=212, output 34+15=49 (msg_3 carries no tokens → no usage).
    assert_eq!(events[1]["body"]["usageDetails"]["input"], 212);
    assert_eq!(events[1]["body"]["usageDetails"]["output"], 49);
    // current_context_size = last step (msg_4) TUI formula: 200+15+9+4+1 = 229.
    // Differs from summed input 212 → AC1.2 (last-not-sum).
    assert_eq!(events[1]["body"]["usageDetails"]["current_context_size"], 229);
    // AC3.2: no reasoning_tokens leaked into emitted usageDetails.
    assert!(events[1]["body"]["usageDetails"].get("reasoning_tokens").is_none());
}

#[test]
fn end_to_end_pi_agent_transcript() {
    let msgs = vec![
        json!({
            "role": "user",
            "content": "list files"
        }),
        json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Let me check" },
                { "type": "toolCall", "id": "tc1", "name": "bash", "arguments": { "command": "ls" } }
            ],
            "model": "claude-sonnet-4-5"
        }),
        json!({
            "role": "toolResult",
            "toolCallId": "tc1",
            "toolName": "bash",
            "content": [{ "type": "text", "text": "file1.txt" }],
            "isError": false
        }),
    ];

    let normalized = code_trace::pi_agent::normalize_pi_agent_messages(msgs);
    let turns = code_trace::turns::build_turns(normalized);
    assert_eq!(turns.len(), 1);

    let tags = vec!["pi-agent".to_string()];
    let events = code_trace::emit::build_ingestion_batch(
        "sess-pi-1",
        1,
        &turns[0],
        std::path::Path::new("pi-agent"),
        &tags,
        code_trace::source::Source::PiAgent,
        None,
    );

    assert!(events.len() >= 2);
    assert_eq!(events[0]["type"], "trace-create");
    assert!(events[0]["body"]["name"].as_str().unwrap().starts_with("Pi Agent"));
    assert_eq!(events[0]["body"]["metadata"]["source"], "pi-agent");
}
