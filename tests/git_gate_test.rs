//! git-repo gate spec: with CODE_TRACE_REQUIRE_GIT_REPO on (the default),
//! only sessions whose cwd is inside a git repository are traced.

mod support;

use std::process::Command;
use support::{stop_payload_cwd, write_transcript, FakeLangfuse, TestEnv};
use tempfile::TempDir;

fn git_init(dir: &std::path::Path) {
    let ok = Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git must be installed for this test")
        .status
        .success();
    assert!(ok, "git init failed");
}

#[test]
fn git_cwd_delivers_when_gate_enabled() {
    let fake = FakeLangfuse::start();
    let repo = TempDir::new().unwrap();
    git_init(repo.path());

    let env = TestEnv::with_langfuse(fake.url())
        .sync_send()
        .with_env("CODE_TRACE_REQUIRE_GIT_REPO", "true");
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);

    let payload = stop_payload_cwd("sess-git", &transcript, &repo.path().to_string_lossy());
    let (code, _, _) = env.run(&[], Some(&payload));
    assert_eq!(code, 0);

    assert_eq!(fake.ingestion_posts(), 1, "git cwd should be traced");
    assert_eq!(fake.trace_ids_for_session("sess-git").len(), 1);
}

#[test]
fn non_git_cwd_suppressed_when_gate_enabled() {
    let fake = FakeLangfuse::start();
    let plain = TempDir::new().unwrap(); // not a git repo

    let env = TestEnv::with_langfuse(fake.url())
        .sync_send()
        .with_env("CODE_TRACE_REQUIRE_GIT_REPO", "true");
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);

    let payload = stop_payload_cwd("sess-nogit", &transcript, &plain.path().to_string_lossy());
    let (code, _, _) = env.run(&[], Some(&payload));
    assert_eq!(code, 0);

    // sync_send makes exit imply delivery, so zero posts is exact.
    assert_eq!(fake.ingestion_posts(), 0, "non-git cwd must not be traced");
    // No session recorded — the gate returns before touching state.
    let state = env.read_state();
    assert!(state.cursors.is_empty(), "gate must not record the session");
}

#[test]
fn non_git_cwd_delivers_when_gate_disabled() {
    let fake = FakeLangfuse::start();
    let plain = TempDir::new().unwrap();

    // Gate off (this is also the TestEnv default, but be explicit).
    let env = TestEnv::with_langfuse(fake.url())
        .sync_send()
        .with_env("CODE_TRACE_REQUIRE_GIT_REPO", "false");
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);

    let payload = stop_payload_cwd("sess-off", &transcript, &plain.path().to_string_lossy());
    let (code, _, _) = env.run(&[], Some(&payload));
    assert_eq!(code, 0);

    assert_eq!(fake.ingestion_posts(), 1, "gate disabled: non-git cwd traced");
    assert_eq!(fake.trace_ids_for_session("sess-off").len(), 1);
}
