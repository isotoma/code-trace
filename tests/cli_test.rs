//! End-to-end tests of the binary's CLI surface and privacy behaviour.
//! Every invocation gets an isolated HOME/XDG_* pointing at a temp dir so the
//! developer's real code-trace state is never read or written.

use code_trace::state::{state_key, SessionRecord, SessionState, State};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_code-trace");

#[derive(Debug, Clone)]
struct Request {
    method: String,
    path: String,
    body: String,
}

/// Minimal HTTP server: records every request, answers by route.
struct MockServer {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl MockServer {
    fn start(trace_ids: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let reqs = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut content_length = 0usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = header.to_lowercase().strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_length];
                if content_length > 0 {
                    let _ = reader.read_exact(&mut body);
                }
                let body = String::from_utf8_lossy(&body).to_string();

                let response_body = if method == "GET" && path.starts_with("/api/public/traces") {
                    let data: Vec<_> = trace_ids
                        .iter()
                        .map(|id| serde_json::json!({"id": id}))
                        .collect();
                    serde_json::json!({
                        "data": data,
                        "meta": {"page": 1, "limit": 100, "totalItems": data.len(), "totalPages": 1}
                    })
                    .to_string()
                } else {
                    "{}".to_string()
                };
                reqs.lock().unwrap().push(Request { method, path, body });
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
            }
        });
        MockServer { url, requests }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn wait_for_request(&self, timeout_ms: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            if !self.requests.lock().unwrap().is_empty() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }
}

struct TestEnv {
    home: TempDir,
    langfuse_url: Option<String>,
}

impl TestEnv {
    fn new() -> Self {
        TestEnv {
            home: TempDir::new().unwrap(),
            langfuse_url: None,
        }
    }

    fn with_langfuse(url: &str) -> Self {
        TestEnv {
            home: TempDir::new().unwrap(),
            langfuse_url: Some(url.to_string()),
        }
    }

    fn state_file(&self) -> std::path::PathBuf {
        self.home.path().join("data").join("code-trace").join("state.json")
    }

    fn write_state(&self, state: &State) {
        let dir = self.state_file().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(self.state_file(), serde_json::to_string(state).unwrap()).unwrap();
    }

    fn read_state(&self) -> State {
        let buf = std::fs::read_to_string(self.state_file()).unwrap_or_default();
        code_trace::state::parse_state_json(&buf)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            // Payload cwds here are "/tmp" (non-git); the git-repo gate is on
            // by default, so disable it for these CLI-behavior tests.
            .env("CODE_TRACE_REQUIRE_GIT_REPO", "false");
        if let Some(url) = &self.langfuse_url {
            cmd.env("TRACE_TO_LANGFUSE", "true")
                .env("LANGFUSE_PUBLIC_KEY", "pk-test")
                .env("LANGFUSE_SECRET_KEY", "sk-test")
                .env("LANGFUSE_BASE_URL", url);
        }
        cmd
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
        let mut cmd = self.command(args);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.unwrap_or("").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    }
}

fn make_record(session_id: &str, transcript: Option<&str>, suppressed: bool, last_seen: u64) -> SessionRecord {
    let handle = transcript.unwrap_or(session_id);
    SessionRecord {
        session_id: session_id.to_string(),
        source: "claude-code".to_string(),
        transcript_path: transcript.map(String::from),
        cwd: None,
        suppressed,
        last_seen_epoch: last_seen,
        cursor_key: state_key("claude-code", session_id, handle),
    }
}

fn write_transcript(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("transcript.jsonl");
    let content = concat!(
        r#"{"type":"user","message":{"role":"user","content":"Hello"}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi!"}]}}"#,
        "\n"
    );
    std::fs::write(&path, content).unwrap();
    path
}

fn stop_payload(session_id: &str, transcript: &std::path::Path) -> String {
    serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript.to_string_lossy(),
        "cwd": "/tmp"
    })
    .to_string()
}

#[test]
fn version_prints_crate_version() {
    let env = TestEnv::new();
    let (code, out, _) = env.run(&["--version"], None);
    assert_eq!(code, 0);
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "got: {out}");
}

#[test]
fn help_lists_all_subcommands() {
    let env = TestEnv::new();
    let (code, out, _) = env.run(&["--help"], None);
    assert_eq!(code, 0);
    for cmd in ["--on-start", "status", "sessions", "pause", "resume", "purge", "--version", "--help"] {
        assert!(out.contains(cmd), "help missing {cmd}: {out}");
    }
}

#[test]
fn on_start_records_session_and_prints_enabled_reminder() {
    let env = TestEnv::with_langfuse("http://127.0.0.1:9");
    let payload = r#"{"hook_event_name":"SessionStart","source":"startup","session_id":"sess-on-start","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#;
    let (code, out, _) = env.run(&["--on-start"], Some(payload));
    assert_eq!(code, 0);
    // Emitted as a SessionStart `systemMessage` (JSON) so the USER sees a
    // terminal banner; plain stdout would only reach the model's context.
    let v: serde_json::Value = serde_json::from_str(&out).expect("on-start emits JSON");
    assert!(
        v["systemMessage"].as_str().unwrap_or("").contains("ENABLED"),
        "got: {out}"
    );
    assert!(
        v.get("hookSpecificOutput").is_none(),
        "reminder is user-facing only; no agent-context line: {out}"
    );
    let state = env.read_state();
    let record = &state.sessions["sess-on-start"];
    assert_eq!(record.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    assert_eq!(record.source, "claude-code");
    assert!(!record.suppressed);
}

#[test]
fn on_start_prints_nothing_when_unconfigured() {
    let env = TestEnv::new(); // no TRACE_TO_LANGFUSE, no keys
    let payload = r#"{"hook_event_name":"SessionStart","source":"startup","session_id":"sess-quiet","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#;
    let (code, out, _) = env.run(&["--on-start"], Some(payload));
    assert_eq!(code, 0);
    assert!(out.is_empty(), "expected silence, got: {out}");
    // Session is still recorded for later pause/purge targeting.
    assert!(env.read_state().sessions.contains_key("sess-quiet"));
}

#[test]
fn on_start_reports_paused_for_suppressed_session() {
    let env = TestEnv::with_langfuse("http://127.0.0.1:9");
    let mut state = State::default();
    state.sessions.insert(
        "sess-private".into(),
        make_record("sess-private", Some("/tmp/t.jsonl"), true, 1),
    );
    env.write_state(&state);
    let payload = r#"{"hook_event_name":"SessionStart","source":"resume","session_id":"sess-private","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#;
    let (code, out, _) = env.run(&["--on-start"], Some(payload));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).expect("on-start emits JSON");
    assert!(
        v["systemMessage"].as_str().unwrap_or("").contains("PAUSED"),
        "got: {out}"
    );
    assert!(env.read_state().sessions["sess-private"].suppressed);
}

#[test]
fn pause_targets_most_recent_and_resume_clears() {
    let env = TestEnv::new();
    let mut state = State::default();
    state.sessions.insert("older".into(), make_record("older", None, false, 100));
    state.sessions.insert("newer".into(), make_record("newer", None, false, 200));
    env.write_state(&state);

    let (code, out, _) = env.run(&["pause"], None);
    assert_eq!(code, 0);
    assert!(out.contains("newer"), "pause must name its target: {out}");
    let state = env.read_state();
    assert!(state.sessions["newer"].suppressed);
    assert!(!state.sessions["older"].suppressed);

    let (code, out, _) = env.run(&["resume", "--session", "newer"], None);
    assert_eq!(code, 0);
    assert!(out.contains("newer"));
    assert!(!env.read_state().sessions["newer"].suppressed);
}

#[test]
fn pause_fails_cleanly_on_empty_registry_and_unknown_id() {
    let env = TestEnv::new();
    let (code, _, err) = env.run(&["pause"], None);
    assert_ne!(code, 0);
    assert!(!err.is_empty());

    let mut state = State::default();
    state.sessions.insert("known".into(), make_record("known", None, false, 1));
    env.write_state(&state);
    let (code, _, err) = env.run(&["pause", "--session", "mystery"], None);
    assert_ne!(code, 0);
    assert!(err.contains("mystery"), "got: {err}");
}

#[test]
fn suppressed_session_emits_nothing_but_consumes_turns() {
    let server = MockServer::start(vec![]);
    let env = TestEnv::with_langfuse(&server.url);
    let transcript = write_transcript(env.home.path());
    let mut state = State::default();
    state.sessions.insert(
        "sess-supp".into(),
        make_record("sess-supp", Some(&transcript.to_string_lossy()), true, 1),
    );
    env.write_state(&state);

    let (code, _, _) = env.run(&[], Some(&stop_payload("sess-supp", &transcript)));
    assert_eq!(code, 0);

    // The send fork must never happen. Give a forked child a moment to
    // (wrongly) send.
    std::thread::sleep(std::time::Duration::from_millis(700));
    assert!(server.requests().is_empty(), "suppressed session sent data");
    // But the cursor advances past the paused turns, so they can never be
    // emitted later — pause means never traced, not deferred.
    let state = env.read_state();
    let cursor_key = &state.sessions["sess-supp"].cursor_key;
    let cursor = state.cursors.get(cursor_key).expect("cursor must exist");
    assert!(cursor.offset > 0, "cursor must advance past suppressed turns");
    assert_eq!(cursor.turn_count, 1, "consumed turns are counted");
    assert!(state.sessions["sess-supp"].suppressed, "suppression must survive the hook");
}

#[test]
fn bare_invocation_emits_and_advances_cursor() {
    let server = MockServer::start(vec![]);
    let env = TestEnv::with_langfuse(&server.url);
    let transcript = write_transcript(env.home.path());

    let (code, _, _) = env.run(&[], Some(&stop_payload("sess-open", &transcript)));
    assert_eq!(code, 0);

    assert!(server.wait_for_request(5000), "no ingestion request arrived");
    let reqs = server.requests();
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].path, "/api/public/ingestion");
    assert!(reqs[0].body.contains("sess-open"));

    let state = env.read_state();
    let record = &state.sessions["sess-open"];
    assert!(!record.suppressed);
    let cursor = &state.cursors[&record.cursor_key];
    assert!(cursor.offset > 0, "cursor did not advance");
    assert_eq!(cursor.turn_count, 1);
}

#[test]
fn purge_deletes_traces_transcript_and_state() {
    let server = MockServer::start(vec!["trace-1".into(), "trace-2".into()]);
    let env = TestEnv::with_langfuse(&server.url);
    let transcript = write_transcript(env.home.path());
    let mut state = State::default();
    let record = make_record("sess-purge", Some(&transcript.to_string_lossy()), false, 1);
    state.cursors.insert(
        record.cursor_key.clone(),
        SessionState { offset: 42, ..Default::default() },
    );
    state.cursors.insert("unrelated".into(), SessionState::default());
    state.sessions.insert("sess-purge".into(), record);
    env.write_state(&state);

    let (code, out, err) = env.run(&["purge", "--session", "sess-purge", "--yes"], None);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("deleted 2 Langfuse traces"), "got: {out}");

    let reqs = server.requests();
    let get = reqs.iter().find(|r| r.method == "GET").expect("no traces GET");
    assert!(get.path.contains("sessionId=sess-purge"));
    let del = reqs.iter().find(|r| r.method == "DELETE").expect("no DELETE");
    assert!(del.body.contains("trace-1") && del.body.contains("trace-2"));

    assert!(!transcript.exists(), "transcript not deleted");
    let state = env.read_state();
    assert!(!state.sessions.contains_key("sess-purge"));
    assert_eq!(state.cursors.len(), 1, "linked cursor not removed");
    assert!(state.cursors.contains_key("unrelated"));
}

#[test]
fn purge_local_only_skips_langfuse() {
    let server = MockServer::start(vec!["trace-1".into()]);
    let env = TestEnv::with_langfuse(&server.url);
    let transcript = write_transcript(env.home.path());
    let mut state = State::default();
    state.sessions.insert(
        "sess-local".into(),
        make_record("sess-local", Some(&transcript.to_string_lossy()), false, 1),
    );
    env.write_state(&state);

    let (code, _, _) = env.run(&["purge", "--session", "sess-local", "--local-only", "--yes"], None);
    assert_eq!(code, 0);
    assert!(server.requests().is_empty(), "local-only purge hit Langfuse");
    assert!(!transcript.exists());
    assert!(!env.read_state().sessions.contains_key("sess-local"));
}

#[test]
fn legacy_state_migrates_preserving_cursors() {
    let env = TestEnv::with_langfuse("http://127.0.0.1:9");
    let dir = env.state_file().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    let legacy = serde_json::json!({
        "somehash": {"offset": 1234, "buffer": "tail", "turn_count": 9, "updated_epoch": 4102444800u64}
    });
    std::fs::write(env.state_file(), legacy.to_string()).unwrap();

    // Any state-writing invocation triggers the migration.
    let payload = r#"{"hook_event_name":"SessionStart","source":"startup","session_id":"post-migration","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#;
    let (code, _, _) = env.run(&["--on-start"], Some(payload));
    assert_eq!(code, 0);

    let state = env.read_state();
    let cursor = &state.cursors["somehash"];
    assert_eq!(cursor.offset, 1234);
    assert_eq!(cursor.buffer, "tail");
    assert_eq!(cursor.turn_count, 9);
    assert!(state.sessions.contains_key("post-migration"));
}

#[test]
fn status_reports_counts() {
    let env = TestEnv::new();
    let mut state = State::default();
    state.sessions.insert("a".into(), make_record("a", None, false, 1));
    state.sessions.insert("b".into(), make_record("b", None, true, 2));
    env.write_state(&state);
    let (code, out, _) = env.run(&["status"], None);
    assert_eq!(code, 0);
    assert!(out.contains("1 active"), "got: {out}");
    assert!(out.contains("1 suppressed"), "got: {out}");
}

#[test]
fn sessions_lists_most_recent_first() {
    let env = TestEnv::new();
    let mut state = State::default();
    state.sessions.insert("first-session".into(), make_record("first-session", None, false, 100));
    state.sessions.insert("second-session".into(), make_record("second-session", Some("/tmp/x.jsonl"), true, 200));
    env.write_state(&state);
    let (code, out, _) = env.run(&["sessions"], None);
    assert_eq!(code, 0);
    // Ids are truncated to 12 chars in the listing.
    let newer = out.find("second-sessi").expect("newer session missing");
    let older = out.find("first-sessio").expect("older session missing");
    assert!(newer < older, "not sorted most-recent first: {out}");
    assert!(out.contains("/tmp/x.jsonl"));
}
