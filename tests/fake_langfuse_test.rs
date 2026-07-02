//! Tests of the fake Langfuse server itself (task 1.5): auth rejection,
//! purge round-trip, hold semantics, parallel-instance isolation.

mod support;

use base64::Engine;
use serde_json::{json, Value};
use support::FakeLangfuse;

fn auth_header() -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            support::fake_langfuse::PUBLIC_KEY,
            support::fake_langfuse::SECRET_KEY
        ))
    )
}

fn post_batch(url: &str, auth: Option<&str>, events: Vec<Value>) -> u16 {
    let body = json!({"batch": events, "metadata": {}});
    let mut req = ureq::post(format!("{url}/api/public/ingestion"));
    if let Some(a) = auth {
        req = req.header("Authorization", a);
    }
    match req.send_json(&body) {
        Ok(resp) => resp.status().as_u16(),
        Err(ureq::Error::StatusCode(code)) => code,
        Err(e) => panic!("unexpected transport error: {e}"),
    }
}

fn trace_event(session_id: &str, trace_id: &str) -> Value {
    json!({
        "id": format!("event-{trace_id}"),
        "type": "trace-create",
        "body": {"id": trace_id, "sessionId": session_id, "name": "turn"}
    })
}

fn get_traces(url: &str, session_id: &str, page: usize, limit: usize) -> Value {
    let resp = ureq::get(format!(
        "{url}/api/public/traces?sessionId={session_id}&page={page}&limit={limit}"
    ))
    .header("Authorization", &auth_header())
    .call()
    .unwrap();
    serde_json::from_str(&resp.into_body().read_to_string().unwrap()).unwrap()
}

#[test]
fn accepts_authorized_batch_and_records_events_in_order() {
    let fake = FakeLangfuse::start();
    let status = post_batch(
        fake.url(),
        Some(&auth_header()),
        vec![trace_event("s1", "t1"), trace_event("s1", "t2")],
    );
    assert_eq!(status, 207);
    let events = fake.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].pointer("/body/id").unwrap(), "t1");
    assert_eq!(events[1].pointer("/body/id").unwrap(), "t2");
    assert_eq!(fake.ingestion_posts(), 1);
}

#[test]
fn rejects_missing_and_wrong_credentials() {
    let fake = FakeLangfuse::start();
    assert_eq!(post_batch(fake.url(), None, vec![trace_event("s1", "t1")]), 401);
    assert_eq!(
        post_batch(fake.url(), Some("Basic d3Jvbmc6Y3JlZHM="), vec![trace_event("s1", "t1")]),
        401
    );
    // Rejected attempts are recorded, but no events land and none count as accepted.
    assert!(fake.events().is_empty());
    assert_eq!(fake.ingestion_posts(), 0);
    let reqs = fake.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs.iter().all(|r| !r.authorized));
}

#[test]
fn purge_round_trip_lists_pages_and_deletes() {
    let fake = FakeLangfuse::start();
    // 5 traces for s1, 1 for another session that must survive s1's purge.
    let events: Vec<Value> = (1..=5).map(|i| trace_event("s1", &format!("t{i}"))).collect();
    assert_eq!(post_batch(fake.url(), Some(&auth_header()), events), 207);
    assert_eq!(post_batch(fake.url(), Some(&auth_header()), vec![trace_event("s2", "other")]), 207);

    // Pagination: limit 2 -> 3 pages, ids partition cleanly.
    let page1 = get_traces(fake.url(), "s1", 1, 2);
    assert_eq!(page1.pointer("/meta/totalItems").unwrap(), 5);
    assert_eq!(page1.pointer("/meta/totalPages").unwrap(), 3);
    assert_eq!(page1["data"].as_array().unwrap().len(), 2);
    let page3 = get_traces(fake.url(), "s1", 3, 2);
    assert_eq!(page3["data"].as_array().unwrap().len(), 1);

    // Delete s1's traces; listing empties, s2 untouched.
    let ids = fake.trace_ids_for_session("s1");
    assert_eq!(ids.len(), 5);
    let resp = ureq::delete(format!("{}/api/public/traces", fake.url()))
        .header("Authorization", &auth_header())
        .force_send_body()
        .send_json(json!({"traceIds": ids}))
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(fake.trace_ids_for_session("s1").is_empty());
    let after = get_traces(fake.url(), "s1", 1, 100);
    assert_eq!(after.pointer("/meta/totalItems").unwrap(), 0);
    assert_eq!(fake.trace_ids_for_session("s2"), vec!["other"]);
}

#[test]
fn hold_delays_ingestion_but_not_other_routes() {
    let fake = FakeLangfuse::start();
    fake.hold(300);

    // A held ingestion POST takes at least the hold duration...
    let start = std::time::Instant::now();
    let ingest_url = fake.url().to_string();
    let auth = auth_header();
    let handle = std::thread::spawn(move || post_batch(&ingest_url, Some(&auth), vec![trace_event("s1", "t1")]));

    // ...while a concurrent GET on another connection answers immediately.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let get_start = std::time::Instant::now();
    let _ = get_traces(fake.url(), "s1", 1, 100);
    assert!(
        get_start.elapsed() < std::time::Duration::from_millis(200),
        "GET must not queue behind a held ingestion"
    );

    assert_eq!(handle.join().unwrap(), 207);
    assert!(start.elapsed() >= std::time::Duration::from_millis(300));

    // Clearing the hold restores fast ingestion.
    fake.hold(0);
    let start = std::time::Instant::now();
    assert_eq!(post_batch(fake.url(), Some(&auth_header()), vec![trace_event("s1", "t2")]), 207);
    assert!(start.elapsed() < std::time::Duration::from_millis(200));
}

#[test]
fn control_plane_reports_and_resets() {
    let fake = FakeLangfuse::start();
    post_batch(fake.url(), Some(&auth_header()), vec![trace_event("s1", "t1")]);

    let resp = ureq::get(format!("{}/_test/events", fake.url())).call().unwrap();
    let dump: Value = serde_json::from_str(&resp.into_body().read_to_string().unwrap()).unwrap();
    assert_eq!(dump["events"].as_array().unwrap().len(), 1);
    assert_eq!(dump["requests"].as_array().unwrap().len(), 1);

    let resp = ureq::post(format!("{}/_test/reset", fake.url()))
        .send_empty()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert!(fake.events().is_empty());
    assert!(fake.requests().is_empty());
}

#[test]
fn hold_settable_over_http() {
    let fake = FakeLangfuse::start();
    ureq::post(format!("{}/_test/hold?ms=250", fake.url())).send_empty().unwrap();
    let start = std::time::Instant::now();
    assert_eq!(post_batch(fake.url(), Some(&auth_header()), vec![trace_event("s1", "t1")]), 207);
    assert!(start.elapsed() >= std::time::Duration::from_millis(250));
}

#[test]
fn parallel_instances_are_isolated() {
    let a = FakeLangfuse::start();
    let b = FakeLangfuse::start();
    assert_ne!(a.url(), b.url());
    post_batch(a.url(), Some(&auth_header()), vec![trace_event("s1", "t1")]);
    assert_eq!(a.events().len(), 1);
    assert!(b.events().is_empty());
}
