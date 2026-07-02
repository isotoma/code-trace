//! Track 2: concurrency and privacy behaviour of the binary under direct
//! control — no agent involved. Payloads are crafted, environments are
//! tmpdir-isolated, and sequencing uses the binary's own state lock file.
//!
//! The lock tests (mutual exclusion, pause-vs-emit, stress) landed red under
//! the original non-blocking flock and are the acceptance criteria for the
//! blocking-lock fix.

mod support;

use serde_json::Value;
use std::io::Write;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use support::{append_turn, stop_payload, write_transcript, FakeLangfuse, TestEnv};

/// Spawn a bare emit invocation feeding the Stop payload, without waiting.
fn spawn_emit(env: &TestEnv, payload: &str) -> Child {
    let mut cmd = env.command(&[]);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    let _ = child.stdin.take().unwrap().write_all(payload.as_bytes());
    child
}

/// Wait for a child with a deadline; None means it is still running.
fn wait_timeout(child: &mut Child, ms: u64) -> Option<i32> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status.code().unwrap_or(-1));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Turn numbers of all trace-create events recorded for a session, in order.
fn turn_numbers(fake: &FakeLangfuse, session_id: &str) -> Vec<u64> {
    fake.events()
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("trace-create"))
        .filter(|e| e.pointer("/body/sessionId").and_then(Value::as_str) == Some(session_id))
        .filter_map(|e| e.pointer("/body/metadata/turn_number").and_then(Value::as_u64))
        .collect()
}

/// Exclusive flock held by the test itself to stall binary invocations.
struct TestLock {
    file: std::fs::File,
}

impl TestLock {
    fn acquire(env: &TestEnv) -> Self {
        use std::os::unix::io::AsRawFd;
        let path = env.lock_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        unsafe {
            assert_eq!(libc::flock(file.as_raw_fd(), libc::LOCK_EX), 0);
        }
        TestLock { file }
    }

    fn release(self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

// --- 3.1 behaviour round-trip -----------------------------------------------

#[test]
fn pause_and_resume_round_trip() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);
    let sess = "sess-roundtrip";

    // New session traces by default.
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1]);

    // Paused: further payloads send exactly nothing (sync send => exact).
    let (code, out, _) = env.run(&["pause", "--session", sess], None);
    assert_eq!(code, 0, "pause failed: {out}");
    let posts_before = fake.ingestion_posts();
    append_turn(&transcript, 2);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), posts_before, "paused session must send nothing");
    assert_eq!(turn_numbers(&fake, sess), vec![1]);

    // Resumed: tracing flows again.
    let (code, _, _) = env.run(&["resume", "--session", sess], None);
    assert_eq!(code, 0);
    append_turn(&transcript, 3);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);

    // Pinned current behaviour: the cursor does not advance while suppressed,
    // so the paused-period turn (2) is emitted after resume alongside turn 3.
    // Whether that replay is desirable is an open design question for the
    // privacy feature; this assertion documents what the binary does today.
    assert_eq!(turn_numbers(&fake, sess), vec![1, 2, 3]);
}

// --- 3.2 lock mutual exclusion (red) -----------------------------------------

#[test]
fn pause_blocks_while_test_holds_the_state_lock() {
    let env = TestEnv::new();
    let mut state = code_trace::state::State::default();
    state.record_session("claude-code", "s1", None, None);
    env.write_state(&state);

    let lock = TestLock::acquire(&env);
    let mut child = env.command(&["pause", "--session", "s1"]);
    child.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = child.spawn().unwrap();

    // While the test holds the lock the invocation must neither finish nor
    // have touched state.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "pause must block while another process holds the state lock"
    );
    assert!(
        !env.read_state().is_suppressed("s1"),
        "state must not change while the lock is held"
    );

    lock.release();
    let code = wait_timeout(&mut child, 5000).expect("pause must complete after lock release");
    assert_eq!(code, 0);
    assert!(env.read_state().is_suppressed("s1"));
}

// --- 3.3 pause survives a concurrent emit (red) -------------------------------

#[test]
fn pause_survives_concurrent_emit() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url());

    // Session B exists and is about to be paused.
    let mut state = code_trace::state::State::default();
    state.record_session("claude-code", "sess-b", None, None);
    env.write_state(&state);

    // A large transcript gives session A's emit a long load->save window.
    let transcript_a = write_transcript(env.home.path(), "a.jsonl", 2000);
    let mut emit_a = spawn_emit(&env, &stop_payload("sess-a", &transcript_a));

    // Pause B while A is (very likely) between load_state and save_state.
    // Once the lock blocks, this serializes instead and must never be lost.
    std::thread::sleep(Duration::from_millis(30));
    let (code, out, _) = env.run(&["pause", "--session", "sess-b"], None);
    assert_eq!(code, 0, "pause failed: {out}");

    let code = wait_timeout(&mut emit_a, 30000).expect("emit A must finish");
    assert_eq!(code, 0);

    assert!(
        env.read_state().is_suppressed("sess-b"),
        "B's pause was lost to A's concurrent state save"
    );

    // And the suppression is effective: B's next emit sends nothing.
    let env_sync = TestEnv { sync_send: true, ..env };
    let transcript_b = write_transcript(env_sync.home.path(), "b.jsonl", 1);
    let posts_before = fake.ingestion_posts();
    let (code, _, _) = env_sync.run(&[], Some(&stop_payload("sess-b", &transcript_b)));
    assert_eq!(code, 0);
    assert_eq!(fake.ingestion_posts(), posts_before);
}

// --- 3.4 purge semantics (green pins) -----------------------------------------

#[test]
fn langfuse_only_purge_preserves_state_and_does_not_resurrect() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "sess-lfonly";
    let transcript = write_transcript(env.home.path(), "t.jsonl", 2);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1, 2]);

    let (code, out, err) = env.run(&["purge", "--session", sess, "--langfuse-only", "--yes"], None);
    assert_eq!(code, 0, "purge failed: {out} {err}");
    assert!(fake.trace_ids_for_session(sess).is_empty(), "traces must be deleted");

    // Registry entry, cursor, and transcript all survive a langfuse-only purge.
    let state = env.read_state();
    let record = state.sessions.get(sess).expect("registry entry must survive");
    let cursor = state.cursors.get(&record.cursor_key).expect("cursor must survive");
    assert_eq!(cursor.turn_count, 2);
    assert!(transcript.exists());

    // The next turn emits only itself — purged history is not re-POSTed.
    // The event log keeps the pre-purge events (turns 1, 2); a resurrection
    // would append duplicates of them. Only turn 3's trace is live.
    append_turn(&transcript, 3);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);
    assert_eq!(turn_numbers(&fake, sess), vec![1, 2, 3]);
    assert_eq!(fake.trace_ids_for_session(sess).len(), 1);
}

#[test]
fn full_purge_deletes_traces_transcript_and_state_with_pagination() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let sess = "sess-full";
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);

    // Seed 250 more traces so the purge's listing must paginate (limit 100).
    fake.seed_events(
        (0..250)
            .map(|i| {
                serde_json::json!({
                    "id": format!("seed-event-{i}"),
                    "type": "trace-create",
                    "body": {"id": format!("seed-trace-{i}"), "sessionId": sess}
                })
            })
            .collect(),
    );
    assert_eq!(fake.trace_ids_for_session(sess).len(), 251);

    let (code, out, err) = env.run(&["purge", "--session", sess, "--yes"], None);
    assert_eq!(code, 0, "purge failed: {out} {err}");
    assert!(out.contains("deleted 251 Langfuse traces"), "got: {out}");
    assert!(fake.trace_ids_for_session(sess).is_empty());
    assert!(!transcript.exists(), "transcript must be deleted");
    let state = env.read_state();
    assert!(state.sessions.is_empty());
    assert!(state.cursors.is_empty());
}

// --- 3.5 purge vs in-flight send: the accepted window --------------------------

/// A forked send that is already in flight cannot be recalled by purge — this
/// is the documented, accepted limitation (pause-early is the defence, purge
/// is remediation). This test pins the window so any change is deliberate.
#[test]
fn held_send_lands_after_purge_and_trace_reappears() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()); // fork mode: send outlives process
    let sess = "sess-window";
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);
    let (code, _, _) = env.run(&[], Some(&stop_payload(sess, &transcript)));
    assert_eq!(code, 0);
    assert!(fake.wait_for_ingestion_posts(1, 5000));
    assert_eq!(fake.trace_ids_for_session(sess).len(), 1);

    // Hold the next send in flight, emit turn 2, purge inside the window.
    // spawn_emit (null stdio) is essential: with piped stdio, waiting for the
    // parent would silently also wait for the forked child holding the pipes.
    fake.hold(1500);
    append_turn(&transcript, 2);
    let mut emit = spawn_emit(&env, &stop_payload(sess, &transcript));
    let code = wait_timeout(&mut emit, 5000).expect("emit parent must exit while send is held");
    assert_eq!(code, 0); // parent exited; child's POST is being held

    let (code, out, err) = env.run(&["purge", "--session", sess, "--langfuse-only", "--yes"], None);
    assert_eq!(code, 0, "purge failed: {out} {err}");
    assert!(
        fake.trace_ids_for_session(sess).is_empty(),
        "purge deleted everything visible at purge time"
    );

    // The held send lands after the purge: the trace reappears.
    fake.hold(0);
    assert!(fake.wait_for_ingestion_posts(2, 10000), "held send never landed");
    assert_eq!(turn_numbers(&fake, sess), vec![1, 2]); // turn 2 landed post-purge
    assert_eq!(
        fake.trace_ids_for_session(sess).len(),
        1,
        "the in-flight turn's trace survives the purge — accepted window"
    );
}

// --- 3.6 stress: invariants under contention (red) ------------------------------

#[test]
fn stress_concurrent_emits_uphold_invariants() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    const SESSIONS: usize = 4;
    const ROUNDS: u32 = 5;

    let transcripts: Vec<_> = (0..SESSIONS)
        .map(|s| (format!("sess-{s}"), env.home.path().join(format!("t{s}.jsonl"))))
        .collect();

    let dump = |msg: &str| {
        format!(
            "{msg}\nfake langfuse event log:\n{}",
            serde_json::to_string_pretty(&fake.events()).unwrap()
        )
    };

    for round in 1..=ROUNDS {
        // Every session gains a turn; all emits for the round run concurrently.
        for (sess, path) in &transcripts {
            if round == 1 {
                write_transcript(env.home.path(), path.file_name().unwrap().to_str().unwrap(), 1);
            } else {
                append_turn(path, round);
            }
            let _ = sess;
        }
        let mut children: Vec<Child> = transcripts
            .iter()
            .map(|(sess, path)| spawn_emit(&env, &stop_payload(sess, path)))
            .collect();
        for child in &mut children {
            let code = wait_timeout(child, 30000).expect("emit must finish");
            assert_eq!(code, 0);
        }

        // Interleaved privacy ops between rounds:
        // sess-0 pauses after round 1 and never resumes;
        // sess-1 pauses after round 2 and resumes after round 3.
        if round == 1 {
            let (code, _, _) = env.run(&["pause", "--session", "sess-0"], None);
            assert_eq!(code, 0);
        }
        if round == 2 {
            let (code, _, _) = env.run(&["pause", "--session", "sess-1"], None);
            assert_eq!(code, 0);
        }
        if round == 3 {
            let (code, _, _) = env.run(&["resume", "--session", "sess-1"], None);
            assert_eq!(code, 0);
        }
    }

    let state = env.read_state();

    // Invariant 1: a pause with no subsequent resume is still in effect.
    assert!(state.is_suppressed("sess-0"), "{}", dump("sess-0's pause was lost"));

    // Invariant 2: sess-0 emitted nothing after its pause.
    assert_eq!(
        turn_numbers(&fake, "sess-0"),
        vec![1],
        "{}",
        dump("suppressed session emitted after pause")
    );

    // Invariant 3: no duplicate (session, turn) pairs anywhere.
    for s in 0..SESSIONS {
        let sess = format!("sess-{s}");
        let mut turns = turn_numbers(&fake, &sess);
        let before = turns.len();
        turns.sort_unstable();
        turns.dedup();
        assert_eq!(
            turns.len(),
            before,
            "{}",
            dump(&format!("duplicate turns emitted for {sess}"))
        );
    }
}
