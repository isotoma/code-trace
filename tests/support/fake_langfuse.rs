//! In-memory fake Langfuse: the three endpoints code-trace uses plus a test
//! control plane. Serves both test tracks — spawned in-process here, run as a
//! standalone container service for the Track 1 harness.
//!
//! Endpoints:
//! - `POST /api/public/ingestion` — record batch events (Basic auth, 207)
//! - `GET /api/public/traces?sessionId=&page=&limit=` — paginated trace list
//!   derived from recorded `trace-create` events, minus deletions
//! - `DELETE /api/public/traces` with `{"traceIds": [...]}`
//! - `GET /_test/events`, `POST /_test/reset`, `POST /_test/hold?ms=N`
//!
//! Each connection is handled on its own thread so a held (latency-injected)
//! ingestion response never blocks a concurrent purge's GET/DELETE.

use base64::Engine;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PUBLIC_KEY: &str = "pk-test";
pub const SECRET_KEY: &str = "sk-test";

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub body: String,
    pub authorized: bool,
}

#[derive(Default)]
struct Inner {
    /// Every request in arrival order, accepted or rejected.
    requests: Mutex<Vec<RecordedRequest>>,
    /// Accepted ingestion events, in arrival order.
    events: Mutex<Vec<Value>>,
    /// Trace ids removed via DELETE.
    deleted: Mutex<HashSet<String>>,
    /// Delay applied to ingestion POST handling, 0 = none.
    hold_ms: AtomicU64,
}

pub struct FakeLangfuse {
    url: String,
    inner: Arc<Inner>,
}

impl FakeLangfuse {
    /// Bind 127.0.0.1 on an ephemeral port and serve until dropped.
    pub fn start() -> Self {
        Self::start_on("127.0.0.1:0")
    }

    /// Bind an explicit address — used by the standalone `fake-langfuse`
    /// binary the Track 1 container runs.
    pub fn start_on(addr: &str) -> Self {
        let listener = TcpListener::bind(addr).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let inner = Arc::new(Inner::default());
        let server_inner = Arc::clone(&inner);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let conn_inner = Arc::clone(&server_inner);
                std::thread::spawn(move || handle_connection(stream, &conn_inner));
            }
        });
        FakeLangfuse { url, inner }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.inner.requests.lock().unwrap().clone()
    }

    /// Accepted ingestion events in arrival order.
    pub fn events(&self) -> Vec<Value> {
        self.inner.events.lock().unwrap().clone()
    }

    /// Accepted ingestion events belonging to the given session: trace-create
    /// events carrying its sessionId, plus child events referencing those traces.
    pub fn events_for_session(&self, session_id: &str) -> Vec<Value> {
        let events = self.events();
        let trace_ids: HashSet<String> = events
            .iter()
            .filter(|e| e.pointer("/body/sessionId").and_then(Value::as_str) == Some(session_id))
            .filter_map(|e| e.pointer("/body/id").and_then(Value::as_str).map(String::from))
            .collect();
        events
            .into_iter()
            .filter(|e| {
                e.pointer("/body/sessionId").and_then(Value::as_str) == Some(session_id)
                    || e.pointer("/body/traceId")
                        .and_then(Value::as_str)
                        .is_some_and(|t| trace_ids.contains(t))
            })
            .collect()
    }

    /// Live (non-deleted) trace ids for a session, from trace-create events.
    pub fn trace_ids_for_session(&self, session_id: &str) -> Vec<String> {
        let deleted = self.inner.deleted.lock().unwrap();
        live_trace_ids(&self.events(), session_id, &deleted)
    }

    /// Count of accepted ingestion POSTs.
    pub fn ingestion_posts(&self) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == "POST" && r.path.starts_with("/api/public/ingestion") && r.authorized)
            .count()
    }

    /// Poll until at least `n` accepted ingestion POSTs arrived. Needed when
    /// the binary's fire-and-forget fork delivers after process exit.
    pub fn wait_for_ingestion_posts(&self, n: usize, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if self.ingestion_posts() >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Inject events directly, bypassing HTTP — for tests that need a large
    /// corpus (e.g. purge pagination) without hundreds of real emits.
    pub fn seed_events(&self, events: Vec<Value>) {
        self.inner.events.lock().unwrap().extend(events);
    }

    /// Delay handling of subsequent ingestion POSTs; 0 clears the hold.
    pub fn hold(&self, ms: u64) {
        self.inner.hold_ms.store(ms, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.inner.requests.lock().unwrap().clear();
        self.inner.events.lock().unwrap().clear();
        self.inner.deleted.lock().unwrap().clear();
        self.inner.hold_ms.store(0, Ordering::SeqCst);
    }
}

fn live_trace_ids(events: &[Value], session_id: &str, deleted: &HashSet<String>) -> Vec<String> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("trace-create"))
        .filter(|e| e.pointer("/body/sessionId").and_then(Value::as_str) == Some(session_id))
        .filter_map(|e| e.pointer("/body/id").and_then(Value::as_str).map(String::from))
        .filter(|id| !deleted.contains(id))
        .collect()
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok()?;
    }
    Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn respond(mut stream: &TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
}

fn is_authorized(req: &Request) -> bool {
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{PUBLIC_KEY}:{SECRET_KEY}"))
    );
    req.headers.get("authorization") == Some(&expected)
}

/// Query string values, e.g. sessionId / page / limit / ms.
fn query_param(path: &str, name: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn handle_connection(stream: TcpStream, inner: &Inner) {
    let Some(req) = read_request(&stream) else { return };
    let route = req.path.split('?').next().unwrap_or("");

    // Control plane: unauthenticated by design.
    match (req.method.as_str(), route) {
        ("GET", "/_test/events") => {
            let body = json!({
                "requests": inner.requests.lock().unwrap().iter().map(|r| json!({
                    "method": r.method, "path": r.path, "authorized": r.authorized,
                })).collect::<Vec<_>>(),
                "events": *inner.events.lock().unwrap(),
            });
            respond(&stream, "200 OK", &body.to_string());
            return;
        }
        ("POST", "/_test/reset") => {
            inner.requests.lock().unwrap().clear();
            inner.events.lock().unwrap().clear();
            inner.deleted.lock().unwrap().clear();
            inner.hold_ms.store(0, Ordering::SeqCst);
            respond(&stream, "200 OK", "{}");
            return;
        }
        ("POST", "/_test/hold") => {
            let ms: u64 = query_param(&req.path, "ms").and_then(|v| v.parse().ok()).unwrap_or(0);
            inner.hold_ms.store(ms, Ordering::SeqCst);
            respond(&stream, "200 OK", "{}");
            return;
        }
        _ => {}
    }

    // Latency injection: a held ingestion POST sleeps before it is recorded,
    // so it stays invisible to the log and counters until it completes.
    if req.method == "POST" && route == "/api/public/ingestion" {
        let hold = inner.hold_ms.load(Ordering::SeqCst);
        if hold > 0 {
            std::thread::sleep(Duration::from_millis(hold));
        }
    }

    let authorized = is_authorized(&req);
    // For accepted ingestion, events are stored BEFORE the request is
    // recorded: pollers gate on the request count, so a counted POST must
    // already have its events visible.
    if authorized && req.method == "POST" && route == "/api/public/ingestion" {
        let batch = serde_json::from_str::<Value>(&req.body)
            .ok()
            .and_then(|v| v.get("batch").cloned());
        if let Some(Value::Array(events)) = batch {
            inner.events.lock().unwrap().extend(events);
        }
    }
    inner.requests.lock().unwrap().push(RecordedRequest {
        method: req.method.clone(),
        path: req.path.clone(),
        body: req.body.clone(),
        authorized,
    });
    if !authorized {
        respond(&stream, "401 Unauthorized", r#"{"message":"invalid credentials"}"#);
        return;
    }

    match (req.method.as_str(), route) {
        ("POST", "/api/public/ingestion") => {
            respond(&stream, "207 Multi-Status", r#"{"successes":[],"errors":[]}"#);
        }
        ("GET", "/api/public/traces") => {
            let session_id = query_param(&req.path, "sessionId").unwrap_or_default();
            let page: usize = query_param(&req.path, "page").and_then(|v| v.parse().ok()).unwrap_or(1);
            let limit: usize = query_param(&req.path, "limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50)
                .max(1); // limit=0 would divide by zero below
            let ids = {
                let events = inner.events.lock().unwrap();
                let deleted = inner.deleted.lock().unwrap();
                live_trace_ids(&events, &session_id, &deleted)
            };
            let total_items = ids.len();
            let total_pages = total_items.div_ceil(limit);
            let data: Vec<Value> = ids
                .iter()
                .skip(page.saturating_sub(1) * limit)
                .take(limit)
                .map(|id| json!({"id": id}))
                .collect();
            let body = json!({
                "data": data,
                "meta": {"page": page, "limit": limit, "totalItems": total_items, "totalPages": total_pages}
            });
            respond(&stream, "200 OK", &body.to_string());
        }
        ("DELETE", "/api/public/traces") => {
            let ids: Vec<String> = serde_json::from_str::<Value>(&req.body)
                .ok()
                .and_then(|v| v.get("traceIds").cloned())
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            inner.deleted.lock().unwrap().extend(ids);
            respond(&stream, "200 OK", "{}");
        }
        _ => respond(&stream, "404 Not Found", r#"{"message":"unknown route"}"#),
    }
}
