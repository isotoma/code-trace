//! sync-send-mode spec: CODE_TRACE_SYNC_SEND=1 makes delivery synchronous
//! with process exit; the default fork path is untouched.

mod support;

use support::{stop_payload, write_transcript, FakeLangfuse, TestEnv};

#[test]
fn sync_send_delivers_before_exit() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()).sync_send();
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);

    let (code, _, _) = env.run(&[], Some(&stop_payload("sess-sync", &transcript)));
    assert_eq!(code, 0);

    // No waiting: the process has exited, so the batch must already be here.
    assert_eq!(fake.ingestion_posts(), 1, "exit must imply delivery");
    assert_eq!(fake.trace_ids_for_session("sess-sync").len(), 1);
}

#[test]
fn default_path_still_delivers_via_fork() {
    let fake = FakeLangfuse::start();
    let env = TestEnv::with_langfuse(fake.url()); // no sync_send
    let transcript = write_transcript(env.home.path(), "t.jsonl", 1);

    let (code, _, _) = env.run(&[], Some(&stop_payload("sess-fork", &transcript)));
    assert_eq!(code, 0);

    // Forked child delivers after the parent exits; poll for arrival.
    assert!(
        fake.wait_for_ingestion_posts(1, 5000),
        "forked send never arrived"
    );
    assert_eq!(fake.trace_ids_for_session("sess-fork").len(), 1);
}
