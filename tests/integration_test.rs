use serde_json::json;
use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn end_to_end_simple_transcript() {
    let mut transcript = NamedTempFile::new().unwrap();
    writeln!(transcript, r#"{{"type":"user","message":{{"role":"user","content":"Hello"}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"assistant","message":{{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{{"type":"text","text":"Hi there!"}}]}}}}"#).unwrap();

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
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "trace-create");
    assert_eq!(events[0]["body"]["tags"], json!(["claude-code", "test"]));
    assert_eq!(events[0]["body"]["sessionId"], "test-session");
    assert_eq!(events[1]["type"], "generation-create");
    assert_eq!(events[1]["body"]["model"], "claude-sonnet-4-20250514");
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
    );

    assert_eq!(events.len(), 3);
    assert_eq!(events[2]["type"], "span-create");
    assert_eq!(events[2]["body"]["name"], "Tool: Bash");
    assert_eq!(events[2]["body"]["output"], json!("README.md"));
}
