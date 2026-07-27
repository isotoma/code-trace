//! System-level tests for the OpenCode and PiAgent source paths: feed payloads
//! via stdin to the real `code-trace` binary and assert on events delivered to
//! the in-process fake Langfuse. Every test that runs the binary previously
//! exercised only the Claude Code Stop-hook path; these tests close that gap by
//! piping `{"source":"opencode",...}` and `{"source":"pi-agent",...}` payloads.
//!
//! All tests use `CODE_TRACE_SYNC_SEND=1` so process exit implies all HTTP sends
//! completed — making "nothing was sent" and exact-delivery assertions reliable.

mod support;

use serde_json::{json, Value};
use support::{opencode_payload, pi_agent_payload, FakeLangfuse, TestEnv};

const MODEL: &str = "claude-sonnet-4-20250514";

// ---------------------------------------------------------------------------
// OpenCode message builders (match src/opencode.rs normalizer expectations)
// ---------------------------------------------------------------------------

/// OpenCode user message: `info.role = "user"`, text part in `parts`.
fn oc_user(text: &str) -> Value {
    json!({
        "info": { "role": "user" },
        "parts": [{ "type": "text", "text": text }]
    })
}

/// OpenCode assistant message with text only.
fn oc_assistant(id: &str, text: &str) -> Value {
    json!({
        "info": { "id": id, "role": "assistant", "model": MODEL },
        "parts": [{ "type": "text", "text": text }]
    })
}

/// OpenCode assistant message with text + a tool_use part.
fn oc_assistant_with_tool(
    id: &str,
    text: &str,
    tool_id: &str,
    tool_name: &str,
    input: Value,
) -> Value {
    json!({
        "info": { "id": id, "role": "assistant", "model": MODEL },
        "parts": [
            { "type": "text", "text": text },
            { "type": "tool_use", "id": tool_id, "name": tool_name, "input": input }
        ]
    })
}

/// OpenCode assistant-role message carrying a tool_result part. The normalizer
/// attaches these to the previous tool-use assistant message's
/// `message.tool_results` field (assistant branch, opencode.rs:102-149).
fn oc_tool_result(id: &str, tool_id: &str, content: &str) -> Value {
    json!({
        "info": { "id": id, "role": "assistant" },
        "parts": [{ "type": "tool_result", "tool_use_id": tool_id, "content": content }]
    })
}

// ---------------------------------------------------------------------------
// PiAgent message builders (match src/pi_agent.rs normalizer expectations)
// ---------------------------------------------------------------------------

/// PiAgent user message: string content.
fn pi_user(text: &str) -> Value {
    json!({ "role": "user", "content": text })
}

/// PiAgent assistant message with text only.
fn pi_assistant(id: &str, text: &str) -> Value {
    json!({
        "role": "assistant",
        "id": id,
        "model": MODEL,
        "content": [{ "type": "text", "text": text }]
    })
}

/// PiAgent assistant message with text + a toolCall block.
fn pi_assistant_with_tool(
    id: &str,
    text: &str,
    tool_id: &str,
    tool_name: &str,
    args: Value,
) -> Value {
    json!({
        "role": "assistant",
        "id": id,
        "model": MODEL,
        "content": [
            { "type": "text", "text": text },
            { "type": "toolCall", "id": tool_id, "name": tool_name, "arguments": args }
        ]
    })
}

/// PiAgent assistant message containing a thinking block (skipped by normalizer).
fn pi_assistant_with_thinking(id: &str, text: &str) -> Value {
    json!({
        "role": "assistant",
        "id": id,
        "model": MODEL,
        "content": [
            { "type": "thinking", "thinking": "secret internal reasoning" },
            { "type": "text", "text": text }
        ]
    })
}

/// PiAgent toolResult entry: content is an array of text blocks.
fn pi_tool_result(tool_id: &str, content: &str) -> Value {
    json!({
        "role": "toolResult",
        "toolCallId": tool_id,
        "content": [{ "type": "text", "text": content }],
        "isError": false
    })
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

/// Ingestion events of a given type (trace-create, generation-create,
/// span-create) belonging to a session.
fn events_of_type(fake: &FakeLangfuse, session_id: &str, event_type: &str) -> Vec<Value> {
    fake.events_for_session(session_id)
        .into_iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some(event_type))
        .collect()
}

/// Turn numbers of all trace-create events for a session, in order.
fn turn_numbers(fake: &FakeLangfuse, session_id: &str) -> Vec<u64> {
    fake.events()
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("trace-create"))
        .filter(|e| e.pointer("/body/sessionId").and_then(Value::as_str) == Some(session_id))
        .filter_map(|e| e.pointer("/body/metadata/turn_number").and_then(Value::as_u64))
        .collect()
}

// ===========================================================================
// OpenCode system tests
// ===========================================================================

#[test]
fn opencode_basic_emit_produces_trace_and_generation() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-basic";

    let msgs = vec![oc_user("What is 2+2?"), oc_assistant("msg_1", "The answer is 4.")];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs)));
    assert_eq!(code, 0);

    // Exactly one trace-create with correct metadata.
    let traces = events_of_type(&fake, sess, "trace-create");
    assert_eq!(traces.len(), 1, "expected exactly one trace-create");
    let trace = &traces[0];
    assert_eq!(trace["body"]["metadata"]["source"], "opencode");
    assert_eq!(trace["body"]["sessionId"], sess);
    assert!(
        trace["body"]["name"].as_str().unwrap().starts_with("OpenCode"),
        "trace name should start with OpenCode: {}",
        trace["body"]["name"]
    );

    // Exactly one generation-create with the assistant's model.
    let gens = events_of_type(&fake, sess, "generation-create");
    assert_eq!(gens.len(), 1, "expected exactly one generation-create");
    assert_eq!(gens[0]["body"]["model"], MODEL);

    // trace_id linkage between trace and generation.
    let trace_id = trace["body"]["id"].as_str().unwrap();
    assert_eq!(gens[0]["body"]["traceId"], trace_id, "generation must link to trace");

    // Tags array contains the version tag from the payload's agentVersion.
    let tags = trace["body"]["tags"].as_array().unwrap();
    assert!(
        tags.iter().any(|t| t == "oc-version:0.4.5"),
        "tags must contain oc-version:0.4.5: {tags:?}"
    );
}

#[test]
fn opencode_tool_calls_produce_spans() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-tool";

    let msgs = vec![
        oc_user("List files"),
        oc_assistant_with_tool("msg_1", "Let me check.", "tu_1", "Bash", json!({"command": "ls"})),
    ];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let spans = events_of_type(&fake, sess, "span-create");
    assert_eq!(spans.len(), 1, "expected one span-create");
    assert_eq!(spans[0]["body"]["name"], "Tool: Bash");
    assert_eq!(spans[0]["body"]["input"]["command"], "ls");

    // Span is parented to the generation.
    let gens = events_of_type(&fake, sess, "generation-create");
    let gen_id = gens[0]["body"]["id"].as_str().unwrap();
    assert_eq!(spans[0]["body"]["parentObservationId"], gen_id);
}

#[test]
fn opencode_tool_span_has_output() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-tool-out";

    let msgs = vec![
        oc_user("List files"),
        oc_assistant_with_tool("msg_1", "Let me check.", "tu_1", "Bash", json!({"command": "ls"})),
        oc_tool_result("msg_2", "tu_1", "README.md"),
    ];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let spans = events_of_type(&fake, sess, "span-create");
    assert_eq!(spans.len(), 1);
    let output = spans[0]["body"]["output"].as_str().unwrap_or_else(|| {
        panic!(
            "span output must be a non-null string; got: {}",
            spans[0]["body"]["output"]
        )
    });
    assert!(
        output.contains("README.md"),
        "span output should contain the tool result: {output}"
    );
}

#[test]
fn opencode_pause_resumes_correctly() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-pause";

    // First invocation: emit normally (establishes cursor).
    let msgs1 = vec![oc_user("q1"), oc_assistant("msg_1", "a1")];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs1)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1]);

    // Pause.
    let (code, _, _) = env.run(&["pause", "--session", sess], None);
    assert_eq!(code, 0);

    // Second invocation: new messages → zero ingestion posts, cursor advances.
    let posts_before = fake.ingestion_posts();
    let msgs2 = vec![oc_user("q2"), oc_assistant("msg_2", "a2")];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs2)));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), posts_before, "paused session must send nothing");
    assert_eq!(turn_numbers(&fake, sess), vec![1], "no new trace while paused");

    let state = env.read_state();
    let cursor = &state.cursors[&state.sessions[sess].cursor_key];
    assert_eq!(cursor.turn_count, 2, "cursor must advance past suppressed turn");

    // Resume.
    let (code, _, _) = env.run(&["resume", "--session", sess], None);
    assert_eq!(code, 0);

    // Third invocation: more new messages → events delivered, turn numbering continues.
    let msgs3 = vec![oc_user("q3"), oc_assistant("msg_3", "a3")];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs3)));
    assert_eq!(code, 0);
    // Turn 2 was consumed while paused; turn 3 is emitted after resume.
    assert_eq!(turn_numbers(&fake, sess), vec![1, 3]);
}

#[test]
fn opencode_first_contact_skips_history() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-fc";

    // First invocation: 3 turns → only turn 3 emitted (first-contact skip).
    let msgs = vec![
        oc_user("q1"),
        oc_assistant("msg_1", "a1"),
        oc_user("q2"),
        oc_assistant("msg_2", "a2"),
        oc_user("q3"),
        oc_assistant("msg_3", "a3"),
    ];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs)));
    assert_eq!(code, 0);
    assert_eq!(
        turn_numbers(&fake, sess),
        vec![3],
        "only the latest turn may be emitted on first contact"
    );

    // Skipped turns are counted, not renumbered.
    let state = env.read_state();
    let cursor = &state.cursors[&state.sessions[sess].cursor_key];
    assert_eq!(cursor.turn_count, 3, "skipped turns are counted");

    // Second invocation: 1 new turn → turn 4 emitted.
    let msgs2 = vec![oc_user("q4"), oc_assistant("msg_4", "a4")];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs2)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![3, 4]);
}

#[test]
fn opencode_masks_secrets_end_to_end() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "oc-mask";

    let token = format!("ghp_{}", "a".repeat(36));
    let msgs = vec![
        oc_user(&format!("my token is {token}")),
        oc_assistant("msg_1", "I see it."),
    ];
    let (code, _, _) = env.run(&[], Some(&opencode_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let traces = events_of_type(&fake, sess, "trace-create");
    assert_eq!(traces.len(), 1);
    let input_content = traces[0]["body"]["input"]["content"]
        .as_str()
        .expect("trace input content must be a string");
    assert!(
        input_content.contains("[REDACTED:github-token]"),
        "token should be redacted: {input_content}"
    );
    assert!(
        !input_content.contains("ghp_"),
        "raw token prefix must not appear: {input_content}"
    );
    assert_eq!(
        traces[0]["body"]["metadata"]["user_text"]["redacted"], 1,
        "exactly one redaction should be recorded"
    );
}

// ===========================================================================
// PiAgent system tests
// ===========================================================================

#[test]
fn pi_agent_basic_emit_produces_trace_and_generation() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-basic";

    let msgs = vec![pi_user("What is 2+2?"), pi_assistant("msg_1", "The answer is 4.")];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let traces = events_of_type(&fake, sess, "trace-create");
    assert_eq!(traces.len(), 1, "expected exactly one trace-create");
    let trace = &traces[0];
    assert_eq!(trace["body"]["metadata"]["source"], "pi-agent");
    assert_eq!(trace["body"]["sessionId"], sess);
    assert!(
        trace["body"]["name"].as_str().unwrap().starts_with("Pi Agent"),
        "trace name should start with Pi Agent: {}",
        trace["body"]["name"]
    );

    let gens = events_of_type(&fake, sess, "generation-create");
    assert_eq!(gens.len(), 1, "expected exactly one generation-create");
    assert_eq!(gens[0]["body"]["model"], MODEL);

    let trace_id = trace["body"]["id"].as_str().unwrap();
    assert_eq!(gens[0]["body"]["traceId"], trace_id);

    let tags = trace["body"]["tags"].as_array().unwrap();
    assert!(
        tags.iter().any(|t| t == "pi-version:1.0.0"),
        "tags must contain pi-version:1.0.0: {tags:?}"
    );
}

#[test]
fn pi_agent_tool_calls_produce_spans() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-tool";

    let msgs = vec![
        pi_user("List files"),
        pi_assistant_with_tool("msg_1", "Let me check.", "tc_1", "bash", json!({"command": "ls"})),
    ];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let spans = events_of_type(&fake, sess, "span-create");
    assert_eq!(spans.len(), 1, "expected one span-create");
    assert_eq!(spans[0]["body"]["name"], "Tool: bash");
    assert_eq!(spans[0]["body"]["input"]["command"], "ls");

    let gens = events_of_type(&fake, sess, "generation-create");
    let gen_id = gens[0]["body"]["id"].as_str().unwrap();
    assert_eq!(spans[0]["body"]["parentObservationId"], gen_id);
}

#[test]
fn pi_agent_tool_span_has_output() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-tool-out";

    let msgs = vec![
        pi_user("List files"),
        pi_assistant_with_tool("msg_1", "Let me check.", "tc_1", "bash", json!({"command": "ls"})),
        pi_tool_result("tc_1", "README.md"),
    ];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let spans = events_of_type(&fake, sess, "span-create");
    assert_eq!(spans.len(), 1);
    // PiAgent tool result content is a JSON array; emit.rs serializes non-string
    // values to JSON, so the output is a JSON string containing the text.
    let output = spans[0]["body"]["output"].as_str().unwrap_or_else(|| {
        panic!(
            "span output must be a non-null string; got: {}",
            spans[0]["body"]["output"]
        )
    });
    assert!(
        output.contains("README.md"),
        "span output should contain the tool result: {output}"
    );
}

#[test]
fn pi_agent_pause_resumes_correctly() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-pause";

    let msgs1 = vec![pi_user("q1"), pi_assistant("msg_1", "a1")];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs1)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1]);

    let (code, _, _) = env.run(&["pause", "--session", sess], None);
    assert_eq!(code, 0);

    let posts_before = fake.ingestion_posts();
    let msgs2 = vec![pi_user("q2"), pi_assistant("msg_2", "a2")];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs2)));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), posts_before, "paused session must send nothing");
    assert_eq!(turn_numbers(&fake, sess), vec![1]);

    let state = env.read_state();
    let cursor = &state.cursors[&state.sessions[sess].cursor_key];
    assert_eq!(cursor.turn_count, 2, "cursor must advance past suppressed turn");

    let (code, _, _) = env.run(&["resume", "--session", sess], None);
    assert_eq!(code, 0);

    let msgs3 = vec![pi_user("q3"), pi_assistant("msg_3", "a3")];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs3)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1, 3]);
}

#[test]
fn pi_agent_first_contact_skips_history() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-fc";

    let msgs = vec![
        pi_user("q1"),
        pi_assistant("msg_1", "a1"),
        pi_user("q2"),
        pi_assistant("msg_2", "a2"),
        pi_user("q3"),
        pi_assistant("msg_3", "a3"),
    ];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![3]);

    let state = env.read_state();
    let cursor = &state.cursors[&state.sessions[sess].cursor_key];
    assert_eq!(cursor.turn_count, 3, "skipped turns are counted");

    let msgs2 = vec![pi_user("q4"), pi_assistant("msg_4", "a4")];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs2)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![3, 4]);
}

#[test]
fn pi_agent_masks_secrets_end_to_end() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-mask";

    let token = format!("ghp_{}", "a".repeat(36));
    let msgs = vec![
        pi_user(&format!("my token is {token}")),
        pi_assistant("msg_1", "I see it."),
    ];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let traces = events_of_type(&fake, sess, "trace-create");
    assert_eq!(traces.len(), 1);
    let input_content = traces[0]["body"]["input"]["content"]
        .as_str()
        .expect("trace input content must be a string");
    assert!(
        input_content.contains("[REDACTED:github-token]"),
        "token should be redacted: {input_content}"
    );
    assert!(
        !input_content.contains("ghp_"),
        "raw token prefix must not appear: {input_content}"
    );
    assert_eq!(
        traces[0]["body"]["metadata"]["user_text"]["redacted"], 1,
        "exactly one redaction should be recorded"
    );
}

#[test]
fn pi_agent_skips_thinking_blocks_and_unknown_roles() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "pi-think";

    let msgs = vec![
        json!({"role": "session", "data": {}}),
        json!({"role": "compaction", "data": {}}),
        pi_user("What is 2+2?"),
        pi_assistant_with_thinking("msg_1", "The answer is 4."),
    ];
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload(sess, &msgs)));
    assert_eq!(code, 0);

    let traces = events_of_type(&fake, sess, "trace-create");
    assert_eq!(traces.len(), 1, "unknown roles must not suppress the turn");

    // Only text content appears; thinking is excluded.
    let output_content = traces[0]["body"]["output"]["content"]
        .as_str()
        .expect("trace output content must be a string");
    assert!(
        output_content.contains("The answer is 4."),
        "text content must appear: {output_content}"
    );
    assert!(
        !output_content.contains("secret internal reasoning"),
        "thinking blocks must be excluded: {output_content}"
    );
}

// ===========================================================================
// Error resilience tests (both sources)
// ===========================================================================

#[test]
fn empty_messages_exits_cleanly() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();

    // OpenCode
    let (code, _, _) = env.run(&[], Some(&opencode_payload("oc-empty", &[])));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), 0, "empty messages must send nothing");

    // PiAgent
    let (code, _, _) = env.run(&[], Some(&pi_agent_payload("pi-empty", &[])));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), 0, "empty messages must send nothing");
}

#[test]
fn missing_session_id_exits_cleanly() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();

    // OpenCode
    let payload = json!({
        "source": "opencode",
        "cwd": "/tmp",
        "messages": [oc_user("hello")]
    })
    .to_string();
    let (code, _, _) = env.run(&[], Some(&payload));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), 0, "missing session_id must send nothing");

    // PiAgent (the early return at main.rs:52-58 is source-agnostic)
    let payload = json!({
        "source": "pi-agent",
        "cwd": "/tmp",
        "messages": [pi_user("hello")]
    })
    .to_string();
    let (code, _, _) = env.run(&[], Some(&payload));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), 0, "missing session_id must send nothing");
}

#[test]
fn malformed_json_exits_cleanly() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();

    let (code, _, _) = env.run(&[], Some("not json at all"));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), 0, "malformed JSON must send nothing");
}
