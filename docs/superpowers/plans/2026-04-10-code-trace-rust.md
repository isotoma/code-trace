# code-trace Rust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single static Rust binary that replaces the Python langfuse_hook.py — reads Claude Code hook payloads from stdin, incrementally parses transcript JSONL, and sends traces to Langfuse via the REST ingestion API. Fire-and-forget HTTP so the hook returns instantly.

**Architecture:** Single binary, no SDK dependency. Reads JSON from stdin, reads incremental transcript state from a JSON file under `~/.claude/state/`, assembles turns, builds a batch of `trace-create` / `generation-create` / `span-create` events, fires them to `/api/public/ingestion` with basic auth, and exits without waiting for the response. File locking via `flock` for state safety.

**Tech Stack:** Rust, serde/serde_json (JSON), reqwest (HTTP, non-blocking fire-and-forget), sha2 (hashing), dirs (home dir), chrono (timestamps), uuid (event IDs)

---

## File Structure

```
Cargo.toml                  — workspace root, single binary crate
src/
  main.rs                   — entrypoint, config loading, orchestration
  payload.rs                — stdin JSON parsing, session/transcript extraction
  state.rs                  — state file load/save/lock, session state
  transcript.rs             — JSONL incremental reader, message parsing helpers
  turns.rs                  — turn assembly from messages
  emit.rs                   — build Langfuse ingestion batch, fire HTTP request
  tags.rs                   — gather environment tags (git, user, host, os, cc-version)
  log.rs                    — simple file logger (append to ~/.claude/state/code_trace.log)
tests/
  integration_test.rs       — end-to-end: feed stdin JSON + transcript file, assert HTTP request body
  fixtures/
    transcript_simple.jsonl — minimal 1-turn transcript
    transcript_tools.jsonl  — transcript with tool use + tool results
    payload.json            — sample hook payload
```

---

### Task 1: Project scaffold and config

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/log.rs`

- [ ] **Step 1: Initialize cargo project**

```bash
cd /home/doug/projects/code-trace
cargo init --name code-trace
```

- [ ] **Step 2: Set up Cargo.toml with dependencies**

Replace the generated `Cargo.toml` with:

```toml
[package]
name = "code-trace"
version = "0.1.0"
edition = "2021"
description = "Claude Code -> Langfuse trace hook"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json", "blocking"] }
sha2 = "0.10"
dirs = "6"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }

[profile.release]
opt-level = "s"
lto = true
strip = true
```

Note: We use `reqwest` with the `blocking` feature. The fire-and-forget pattern will spawn a detached child process (see Task 7) rather than using async, keeping the binary simple and dependency-light.

- [ ] **Step 3: Write the logger module**

`src/log.rs`:

```rust
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn log_dir() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".claude").join("state");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn log_file() -> Option<PathBuf> {
    Some(log_dir()?.join("code_trace.log"))
}

fn write_log(level: &str, msg: &str) {
    let Some(path) = log_file() else { return };
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(f, "{ts} [{level}] {msg}");
}

pub fn info(msg: &str) {
    write_log("INFO", msg);
}

pub fn warn(msg: &str) {
    write_log("WARN", msg);
}

pub fn error(msg: &str) {
    write_log("ERROR", msg);
}

pub fn debug(msg: &str) {
    if std::env::var("CC_TRACE_DEBUG").unwrap_or_default().to_lowercase() == "true" {
        write_log("DEBUG", msg);
    }
}
```

- [ ] **Step 4: Write minimal main.rs**

`src/main.rs`:

```rust
mod log;

fn main() {
    log::info("code-trace started");
}
```

- [ ] **Step 5: Verify it compiles and runs**

Run: `cd /home/doug/projects/code-trace && cargo build`
Expected: Compiles with no errors.

Run: `cargo run`
Expected: Exits 0, line appears in `~/.claude/state/code_trace.log`.

- [ ] **Step 6: Commit**

```bash
git init
echo '/target' > .gitignore
git add Cargo.toml Cargo.lock src/ .gitignore CLAUDE.md docs/
git commit -m "feat: project scaffold with logger"
```

---

### Task 2: Payload parsing (stdin JSON)

**Files:**
- Create: `src/payload.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing test**

Add to `src/payload.rs`:

```rust
use serde_json::Value;
use std::path::PathBuf;

pub struct HookPayload {
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub cwd: Option<String>,
}

/// Parse hook payload from a JSON value. Tolerates multiple field name conventions.
pub fn parse_payload(value: &Value) -> HookPayload {
    let session_id = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let transcript = value
        .get("transcriptPath")
        .or_else(|| value.get("transcript_path"))
        .and_then(|v| v.as_str())
        .map(|s| {
            let p = PathBuf::from(s);
            if s.starts_with('~') {
                if let Some(home) = dirs::home_dir() {
                    return home.join(s.strip_prefix("~/").unwrap_or(s));
                }
            }
            p
        });

    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(String::from);

    HookPayload {
        session_id,
        transcript_path: transcript,
        cwd,
    }
}

/// Read stdin fully and parse as JSON. Returns Value::Null on any failure.
pub fn read_stdin() -> Value {
    let mut buf = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).is_err() {
        return Value::Null;
    }
    serde_json::from_str(&buf).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_camel_case_payload() {
        let v = json!({
            "sessionId": "abc-123",
            "transcriptPath": "/tmp/test.jsonl",
            "cwd": "/home/user/project"
        });
        let p = parse_payload(&v);
        assert_eq!(p.session_id.as_deref(), Some("abc-123"));
        assert_eq!(p.transcript_path.as_deref(), Some(std::path::Path::new("/tmp/test.jsonl")));
        assert_eq!(p.cwd.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn parses_snake_case_payload() {
        let v = json!({
            "session_id": "abc-123",
            "transcript_path": "/tmp/test.jsonl"
        });
        let p = parse_payload(&v);
        assert_eq!(p.session_id.as_deref(), Some("abc-123"));
        assert_eq!(p.transcript_path.as_deref(), Some(std::path::Path::new("/tmp/test.jsonl")));
    }

    #[test]
    fn handles_empty_payload() {
        let p = parse_payload(&Value::Null);
        assert!(p.session_id.is_none());
        assert!(p.transcript_path.is_none());
    }
}
```

- [ ] **Step 2: Register module and run tests**

Add `mod payload;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/payload.rs src/main.rs
git commit -m "feat: hook payload parsing from stdin"
```

---

### Task 3: State management (file lock, load/save)

**Files:**
- Create: `src/state.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write state module with tests**

`src/state.rs`:

```rust
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;

use crate::log;

fn state_dir() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".claude").join("state");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn state_file() -> Option<PathBuf> {
    Some(state_dir()?.join("code_trace_state.json"))
}

fn lock_file() -> Option<PathBuf> {
    Some(state_dir()?.join("code_trace_state.lock"))
}

/// RAII file lock using flock. Best-effort: proceeds without lock on failure.
pub struct FileLock {
    _file: Option<File>,
}

impl FileLock {
    pub fn acquire() -> Self {
        let Some(path) = lock_file() else {
            return FileLock { _file: None };
        };
        let file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(_) => return FileLock { _file: None },
        };
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                // Non-blocking try, fall through on failure
                libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB);
            }
        }
        FileLock { _file: Some(file) }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(ref file) = self._file {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub offset: u64,
    #[serde(default)]
    pub buffer: String,
    #[serde(default)]
    pub turn_count: u32,
}

pub type GlobalState = HashMap<String, SessionState>;

pub fn state_key(session_id: &str, transcript_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{session_id}::{transcript_path}"));
    format!("{:x}", hasher.finalize())
}

pub fn load_state() -> GlobalState {
    let Some(path) = state_file() else {
        return GlobalState::new();
    };
    let Ok(mut file) = File::open(&path) else {
        return GlobalState::new();
    };
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return GlobalState::new();
    }
    serde_json::from_str(&buf).unwrap_or_default()
}

pub fn save_state(state: &GlobalState) {
    let Some(path) = state_file() else { return };
    let tmp = path.with_extension("tmp");
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if fs::write(&tmp, &json).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
        Err(e) => log::error(&format!("save_state failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_key_is_deterministic() {
        let k1 = state_key("sess1", "/tmp/t.jsonl");
        let k2 = state_key("sess1", "/tmp/t.jsonl");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // sha256 hex
    }

    #[test]
    fn state_key_differs_for_different_inputs() {
        let k1 = state_key("sess1", "/tmp/a.jsonl");
        let k2 = state_key("sess1", "/tmp/b.jsonl");
        assert_ne!(k1, k2);
    }

    #[test]
    fn session_state_defaults() {
        let ss = SessionState::default();
        assert_eq!(ss.offset, 0);
        assert_eq!(ss.buffer, "");
        assert_eq!(ss.turn_count, 0);
    }
}
```

- [ ] **Step 2: Add libc dependency to Cargo.toml**

Add under `[dependencies]`:

```toml
libc = "0.2"
```

- [ ] **Step 3: Register module and run tests**

Add `mod state;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All tests pass (payload + state tests).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/state.rs src/main.rs
git commit -m "feat: state management with file locking"
```

---

### Task 4: Transcript reader (incremental JSONL)

**Files:**
- Create: `src/transcript.rs`
- Create: `tests/fixtures/transcript_simple.jsonl`
- Modify: `src/main.rs`

- [ ] **Step 1: Create test fixture**

`tests/fixtures/transcript_simple.jsonl` — each line is one JSON object:

```jsonl
{"type":"user","message":{"role":"user","content":"Hello"}}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi there! How can I help?"}]}}
```

- [ ] **Step 2: Write transcript module with tests**

`src/transcript.rs`:

```rust
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::log;
use crate::state::SessionState;

/// Read new bytes from transcript since last offset. Handles partial lines via buffer.
pub fn read_new_jsonl(path: &Path, ss: &mut SessionState) -> Vec<Value> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            log::debug(&format!("read_new_jsonl open failed: {e}"));
            return vec![];
        }
    };

    if file.seek(SeekFrom::Start(ss.offset)).is_err() {
        return vec![];
    }

    let mut chunk = Vec::new();
    if file.read_to_end(&mut chunk).is_err() {
        return vec![];
    }

    let new_offset = ss.offset + chunk.len() as u64;

    if chunk.is_empty() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&chunk);
    let combined = format!("{}{}", ss.buffer, text);
    let lines: Vec<&str> = combined.split('\n').collect();

    // Last element may be incomplete
    ss.buffer = lines.last().unwrap_or(&"").to_string();
    ss.offset = new_offset;

    let mut msgs = Vec::new();
    for line in &lines[..lines.len().saturating_sub(1)] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(v) => msgs.push(v),
            Err(_) => continue,
        }
    }

    msgs
}

/// Extract role from a transcript message. Returns "user" or "assistant" or None.
pub fn get_role(msg: &Value) -> Option<&str> {
    // type=user/assistant at top level
    if let Some(t) = msg.get("type").and_then(|v| v.as_str()) {
        if t == "user" || t == "assistant" {
            return Some(t);
        }
    }
    // message.role
    msg.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .filter(|r| *r == "user" || *r == "assistant")
}

/// Get content from a message (message.content or top-level content).
pub fn get_content(msg: &Value) -> Option<&Value> {
    msg.get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| msg.get("content"))
}

/// Get message.id
pub fn get_message_id(msg: &Value) -> Option<&str> {
    msg.get("message")
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
}

/// Get message.model
pub fn get_model(msg: &Value) -> &str {
    msg.get("message")
        .and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("claude")
}

/// Extract plain text from content (string or array of text blocks).
pub fn extract_text(content: Option<&Value>) -> String {
    let Some(c) = content else {
        return String::new();
    };
    if let Some(s) = c.as_str() {
        return s.to_string();
    }
    if let Some(arr) = c.as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|x| {
                if x.get("type").and_then(|t| t.as_str()) == Some("text") {
                    x.get("text").and_then(|t| t.as_str())
                } else if let Some(s) = x.as_str() {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        return parts.join("\n");
    }
    String::new()
}

/// Check if a message is a tool_result row (user message containing tool_result content blocks).
pub fn is_tool_result(msg: &Value) -> bool {
    if get_role(msg) != Some("user") {
        return false;
    }
    let Some(content) = get_content(msg) else {
        return false;
    };
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .any(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
    }
    false
}

/// Extract tool_result blocks from content array.
pub fn iter_tool_results(content: Option<&Value>) -> Vec<&Value> {
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
        .collect()
}

/// Extract tool_use blocks from content array.
pub fn iter_tool_uses(content: Option<&Value>) -> Vec<&Value> {
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return vec![];
    };
    arr.iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SessionState;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn reads_jsonl_incrementally() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, r#"{{"type":"user","message":{{"role":"user","content":"Hello"}}}}"#).unwrap();
        writeln!(tmp, r#"{{"type":"assistant","message":{{"id":"m1","role":"assistant","model":"claude","content":[{{"type":"text","text":"Hi"}}]}}}}"#).unwrap();

        let mut ss = SessionState::default();
        let msgs = read_new_jsonl(tmp.path(), &mut ss);
        assert_eq!(msgs.len(), 2);
        assert!(ss.offset > 0);

        // Second read with no new data returns empty
        let msgs2 = read_new_jsonl(tmp.path(), &mut ss);
        assert_eq!(msgs2.len(), 0);
    }

    #[test]
    fn get_role_top_level_type() {
        let v: Value = serde_json::from_str(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#).unwrap();
        assert_eq!(get_role(&v), Some("user"));
    }

    #[test]
    fn get_role_message_role() {
        let v: Value = serde_json::from_str(r#"{"message":{"role":"assistant","content":"hi"}}"#).unwrap();
        assert_eq!(get_role(&v), Some("assistant"));
    }

    #[test]
    fn extract_text_from_string() {
        let v: Value = serde_json::json!("hello");
        assert_eq!(extract_text(Some(&v)), "hello");
    }

    #[test]
    fn extract_text_from_content_blocks() {
        let v: Value = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "text", "text": "World"}
        ]);
        assert_eq!(extract_text(Some(&v)), "Hello\nWorld");
    }

    #[test]
    fn is_tool_result_detects_correctly() {
        let v: Value = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "output"}
            ]}
        });
        assert!(is_tool_result(&v));
    }
}
```

- [ ] **Step 3: Add tempfile dev-dependency to Cargo.toml**

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Register module and run tests**

Add `mod transcript;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/transcript.rs src/main.rs tests/
git commit -m "feat: incremental JSONL transcript reader"
```

---

### Task 5: Turn assembly

**Files:**
- Create: `src/turns.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write turns module with tests**

`src/turns.rs`:

```rust
use serde_json::Value;
use std::collections::HashMap;

use crate::transcript;

pub struct Turn {
    pub user_msg: Value,
    pub assistant_msgs: Vec<Value>,
    pub tool_results_by_id: HashMap<String, Value>,
}

pub fn build_turns(messages: Vec<Value>) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut current_user: Option<Value> = None;
    let mut assistant_order: Vec<String> = Vec::new();
    let mut assistant_latest: HashMap<String, Value> = HashMap::new();
    let mut tool_results_by_id: HashMap<String, Value> = HashMap::new();

    let flush = |current_user: &mut Option<Value>,
                 assistant_order: &mut Vec<String>,
                 assistant_latest: &mut HashMap<String, Value>,
                 tool_results_by_id: &mut HashMap<String, Value>,
                 turns: &mut Vec<Turn>| {
        if current_user.is_none() || assistant_latest.is_empty() {
            return;
        }
        let assistants: Vec<Value> = assistant_order
            .iter()
            .filter_map(|mid| assistant_latest.remove(mid))
            .collect();
        turns.push(Turn {
            user_msg: current_user.take().unwrap(),
            assistant_msgs: assistants,
            tool_results_by_id: std::mem::take(tool_results_by_id),
        });
        assistant_order.clear();
    };

    for msg in messages {
        // tool_result rows
        if transcript::is_tool_result(&msg) {
            for tr in transcript::iter_tool_results(transcript::get_content(&msg)) {
                if let Some(tid) = tr.get("tool_use_id").and_then(|v| v.as_str()) {
                    let content = tr.get("content").cloned().unwrap_or(Value::Null);
                    tool_results_by_id.insert(tid.to_string(), content);
                }
            }
            continue;
        }

        let role = transcript::get_role(&msg);

        if role == Some("user") {
            flush(
                &mut current_user,
                &mut assistant_order,
                &mut assistant_latest,
                &mut tool_results_by_id,
                &mut turns,
            );
            current_user = Some(msg);
            assistant_order.clear();
            assistant_latest.clear();
            tool_results_by_id.clear();
            continue;
        }

        if role == Some("assistant") {
            if current_user.is_none() {
                continue;
            }
            let mid = transcript::get_message_id(&msg)
                .map(String::from)
                .unwrap_or_else(|| format!("noid:{}", assistant_order.len()));
            if !assistant_latest.contains_key(&mid) {
                assistant_order.push(mid.clone());
            }
            assistant_latest.insert(mid, msg);
            continue;
        }
    }

    // flush last turn
    flush(
        &mut current_user,
        &mut assistant_order,
        &mut assistant_latest,
        &mut tool_results_by_id,
        &mut turns,
    );

    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_single_turn() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"Hi"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_msgs.len(), 1);
    }

    #[test]
    fn builds_multiple_turns() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"First"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"Reply 1"}]}}),
            json!({"type":"user","message":{"role":"user","content":"Second"}}),
            json!({"type":"assistant","message":{"id":"m2","role":"assistant","model":"claude","content":[{"type":"text","text":"Reply 2"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 2);
    }

    #[test]
    fn collects_tool_results() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Do something"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[
                {"type":"text","text":"Let me check"},
                {"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}
            ]}}),
            json!({"type":"user","message":{"role":"user","content":[
                {"type":"tool_result","tool_use_id":"tu_1","content":"file1.txt\nfile2.txt"}
            ]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_results_by_id.contains_key("tu_1"));
    }

    #[test]
    fn dedupes_assistant_messages_by_id() {
        let msgs = vec![
            json!({"type":"user","message":{"role":"user","content":"Hello"}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"partial"}]}}),
            json!({"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude","content":[{"type":"text","text":"full response"}]}}),
        ];
        let turns = build_turns(msgs);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].assistant_msgs.len(), 1);
        // Latest version wins
        let text = transcript::extract_text(transcript::get_content(&turns[0].assistant_msgs[0]));
        assert_eq!(text, "full response");
    }
}
```

- [ ] **Step 2: Register module and run tests**

Add `mod turns;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/turns.rs src/main.rs
git commit -m "feat: turn assembly from transcript messages"
```

---

### Task 6: Environment tags

**Files:**
- Create: `src/tags.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write tags module with tests**

`src/tags.rs`:

```rust
use std::process::Command;

fn git_cmd(args: &[&str], cwd: Option<&str>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

pub fn gather_env_tags(cwd: Option<&str>) -> Vec<String> {
    let mut tags = vec!["claude-code".to_string()];

    // Git repo name
    if let Some(toplevel) = git_cmd(&["rev-parse", "--show-toplevel"], cwd) {
        if let Some(name) = std::path::Path::new(&toplevel).file_name() {
            tags.push(format!("repo:{}", name.to_string_lossy()));
        }
    }

    // Git branch
    if let Some(branch) = git_cmd(&["rev-parse", "--abbrev-ref", "HEAD"], cwd) {
        tags.push(format!("branch:{branch}"));
    }

    // Username
    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        tags.push(format!("user:{user}"));
    }

    // Hostname
    if let Ok(host) = hostname::get() {
        tags.push(format!("host:{}", host.to_string_lossy()));
    }

    // OS
    tags.push(format!("os:{}", std::env::consts::OS));

    // Claude Code version
    if let Ok(output) = Command::new("claude").arg("--version").output() {
        if output.status.success() {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !ver.is_empty() {
                tags.push(format!("cc-version:{ver}"));
            }
        }
    }

    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_claude_code_tag() {
        let tags = gather_env_tags(None);
        assert!(tags.contains(&"claude-code".to_string()));
    }

    #[test]
    fn includes_os_tag() {
        let tags = gather_env_tags(None);
        assert!(tags.iter().any(|t| t.starts_with("os:")));
    }

    #[test]
    fn includes_user_tag() {
        // Should work on any system with USER or USERNAME set
        let tags = gather_env_tags(None);
        assert!(tags.iter().any(|t| t.starts_with("user:")));
    }
}
```

- [ ] **Step 2: Add hostname dependency to Cargo.toml**

```toml
hostname = "0.4"
```

- [ ] **Step 3: Register module and run tests**

Add `mod tags;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/tags.rs src/main.rs
git commit -m "feat: environment tag collection"
```

---

### Task 7: Langfuse emit (build batch + fire-and-forget HTTP)

**Files:**
- Create: `src/emit.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write emit module**

`src/emit.rs`:

```rust
use crate::log;
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
) -> Vec<Value> {
    let user_text_raw = transcript::extract_text(transcript::get_content(&turn.user_msg));
    let (user_text, user_text_meta) = truncate(&user_text_raw);

    let last_assistant = turn.assistant_msgs.last().unwrap();
    let assistant_text_raw = transcript::extract_text(transcript::get_content(last_assistant));
    let (assistant_text, assistant_text_meta) = truncate(&assistant_text_raw);

    let model = transcript::get_model(&turn.assistant_msgs[0]);
    let tool_calls = tool_calls_from_assistants(&turn.assistant_msgs);

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
            "name": format!("Claude Code - Turn {turn_num}"),
            "sessionId": session_id,
            "input": json!({"role": "user", "content": user_text}),
            "output": json!({"role": "assistant", "content": assistant_text}),
            "tags": tags,
            "metadata": {
                "source": "claude-code",
                "session_id": session_id,
                "turn_number": turn_num,
                "transcript_path": transcript_path.to_string_lossy(),
                "user_text": user_text_meta,
            },
        }
    }));

    // 2. generation-create for the LLM response
    let gen_id = uuid::Uuid::new_v4().to_string();
    events.push(json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "generation-create",
        "timestamp": now,
        "body": {
            "id": gen_id,
            "traceId": trace_id,
            "name": "Claude Response",
            "startTime": now,
            "endTime": now,
            "model": model,
            "input": json!({"role": "user", "content": user_text}),
            "output": json!({"role": "assistant", "content": assistant_text}),
            "metadata": {
                "assistant_text": assistant_text_meta,
                "tool_count": tool_calls.len(),
            },
        }
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

        // Look up tool output from tool_results_by_id
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

/// Fire-and-forget: spawn a detached child process that sends the HTTP request.
/// The parent returns immediately.
pub fn send_batch_fire_and_forget(config: &LangfuseConfig, events: Vec<Value>) {
    let url = format!("{}/api/public/ingestion", config.host);
    let body = json!({
        "batch": events,
        "metadata": {}
    });
    let body_str = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => {
            log::error(&format!("Failed to serialize batch: {e}"));
            return;
        }
    };

    // Use curl as a detached subprocess for true fire-and-forget.
    // This avoids pulling in a tokio runtime and returns instantly.
    let auth = format!("{}:{}", config.public_key, config.secret_key);
    match std::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-u",
            &auth,
            "-d",
            &body_str,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_child) => {
            // Don't wait. The child process will send the request and exit.
            log::debug("Spawned curl for fire-and-forget send");
        }
        Err(e) => {
            log::error(&format!("Failed to spawn curl: {e}"));
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
        let turn = make_simple_turn();
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()]);
        assert_eq!(events.len(), 2); // trace + generation, no tools
        assert_eq!(events[0]["type"], "trace-create");
        assert_eq!(events[1]["type"], "generation-create");
    }

    #[test]
    fn trace_event_has_tags() {
        let turn = make_simple_turn();
        let tags = vec!["claude-code".to_string(), "repo:myrepo".to_string()];
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &tags);
        let trace_tags = events[0]["body"]["tags"].as_array().unwrap();
        assert_eq!(trace_tags.len(), 2);
        assert_eq!(trace_tags[0], "claude-code");
        assert_eq!(trace_tags[1], "repo:myrepo");
    }

    #[test]
    fn builds_tool_spans() {
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
        let events = build_ingestion_batch("sess1", 1, &turn, Path::new("/tmp/t.jsonl"), &["claude-code".to_string()]);
        assert_eq!(events.len(), 3); // trace + generation + 1 tool span
        assert_eq!(events[2]["type"], "span-create");
        assert_eq!(events[2]["body"]["name"], "Tool: Bash");
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
```

- [ ] **Step 2: Register module and run tests**

Add `mod emit;` to `src/main.rs`.

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/emit.rs src/main.rs
git commit -m "feat: langfuse ingestion batch builder and fire-and-forget sender"
```

---

### Task 8: Wire up main

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write the full main.rs orchestration**

Replace `src/main.rs` with:

```rust
mod emit;
mod log;
mod payload;
mod state;
mod tags;
mod transcript;
mod turns;

use std::time::Instant;

fn run() -> i32 {
    let start = Instant::now();
    log::debug("code-trace started");

    // Check enable flag
    if std::env::var("TRACE_TO_LANGFUSE")
        .unwrap_or_default()
        .to_lowercase()
        != "true"
    {
        return 0;
    }

    // Read config from env
    let public_key = std::env::var("CC_LANGFUSE_PUBLIC_KEY")
        .or_else(|_| std::env::var("LANGFUSE_PUBLIC_KEY"))
        .unwrap_or_default();
    let secret_key = std::env::var("CC_LANGFUSE_SECRET_KEY")
        .or_else(|_| std::env::var("LANGFUSE_SECRET_KEY"))
        .unwrap_or_default();
    let host = std::env::var("CC_LANGFUSE_BASE_URL")
        .or_else(|_| std::env::var("LANGFUSE_BASE_URL"))
        .unwrap_or_else(|_| "https://cloud.langfuse.com".to_string());

    if public_key.is_empty() || secret_key.is_empty() {
        return 0;
    }

    let config = emit::LangfuseConfig {
        host,
        public_key,
        secret_key,
    };

    // Read hook payload from stdin
    let raw = payload::read_stdin();
    let hook = payload::parse_payload(&raw);

    let Some(session_id) = hook.session_id else {
        log::debug("Missing session_id; exiting");
        return 0;
    };
    let Some(transcript_path) = hook.transcript_path else {
        log::debug("Missing transcript_path; exiting");
        return 0;
    };

    if !transcript_path.exists() {
        log::debug(&format!(
            "Transcript does not exist: {}",
            transcript_path.display()
        ));
        return 0;
    }

    // Gather tags
    let env_tags = tags::gather_env_tags(hook.cwd.as_deref());

    // Lock, load state, read new messages, build turns, emit
    let _lock = state::FileLock::acquire();
    let mut global_state = state::load_state();
    let key = state::state_key(&session_id, &transcript_path.to_string_lossy());
    let mut ss = global_state
        .get(&key)
        .cloned()
        .unwrap_or_default();

    let msgs = transcript::read_new_jsonl(&transcript_path, &mut ss);
    if msgs.is_empty() {
        global_state.insert(key, ss);
        state::save_state(&global_state);
        return 0;
    }

    let built_turns = turns::build_turns(msgs);
    if built_turns.is_empty() {
        global_state.insert(key, ss);
        state::save_state(&global_state);
        return 0;
    }

    // Collect all events across turns into one batch
    let mut all_events = Vec::new();
    let mut emitted = 0u32;
    for t in &built_turns {
        emitted += 1;
        let turn_num = ss.turn_count + emitted;
        let events =
            emit::build_ingestion_batch(&session_id, turn_num, t, &transcript_path, &env_tags);
        all_events.extend(events);
    }

    ss.turn_count += emitted;
    global_state.insert(key, ss);
    state::save_state(&global_state);

    // Fire and forget
    emit::send_batch_fire_and_forget(&config, all_events);

    let dur = start.elapsed();
    log::info(&format!(
        "Processed {emitted} turns in {:.2}s (session={session_id})",
        dur.as_secs_f64()
    ));

    0
}

fn main() {
    std::process::exit(run());
}
```

- [ ] **Step 2: Build and verify**

Run: `cd /home/doug/projects/code-trace && cargo build`
Expected: Compiles with no errors.

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 3: Manual smoke test**

```bash
cd /home/doug/projects/code-trace

# Create a test transcript
cat > /tmp/test_transcript.jsonl << 'EOF'
{"type":"user","message":{"role":"user","content":"Hello"}}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi! How can I help you today?"}]}}
EOF

# Run with a test payload (won't actually send — no TRACE_TO_LANGFUSE)
echo '{"sessionId":"test-123","transcriptPath":"/tmp/test_transcript.jsonl","cwd":"/tmp"}' | cargo run

# Run with env to see it try to send (will fail auth but proves the flow)
echo '{"sessionId":"test-123","transcriptPath":"/tmp/test_transcript.jsonl","cwd":"/tmp"}' | TRACE_TO_LANGFUSE=true LANGFUSE_PUBLIC_KEY=pk-test LANGFUSE_SECRET_KEY=sk-test cargo run
```

Expected: Second run exits quickly (fire-and-forget), log file at `~/.claude/state/code_trace.log` shows "Processed 1 turns".

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire up main orchestration"
```

---

### Task 9: Release build and install script

**Files:**
- Modify: `Cargo.toml` (already has release profile)
- Create: `install.sh`

- [ ] **Step 1: Build release binary and measure size/speed**

```bash
cd /home/doug/projects/code-trace
cargo build --release
ls -lh target/release/code-trace
```

Expected: Binary under 5MB (stripped + LTO).

- [ ] **Step 2: Time a run to verify it's fast**

```bash
echo '{"sessionId":"bench-1","transcriptPath":"/tmp/test_transcript.jsonl","cwd":"/tmp"}' | \
  TRACE_TO_LANGFUSE=true LANGFUSE_PUBLIC_KEY=pk-test LANGFUSE_SECRET_KEY=sk-test \
  time target/release/code-trace
```

Expected: Under 50ms total wall time.

- [ ] **Step 3: Write install script**

`install.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

REPO="OWNER/code-trace"  # TODO: update with actual GitHub owner
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"

# Detect platform
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${ARCH}" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "Unsupported architecture: ${ARCH}" >&2
    exit 1
    ;;
esac

case "${OS}" in
  linux)  TARGET="${ARCH}-unknown-linux-gnu" ;;
  darwin) TARGET="${ARCH}-apple-darwin" ;;
  *)
    echo "Unsupported OS: ${OS}" >&2
    exit 1
    ;;
esac

ASSET="${BINARY}-${TARGET}"

# Get latest release URL
DOWNLOAD_URL="$(curl -sfL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep "browser_download_url.*${ASSET}" \
  | head -1 \
  | cut -d '"' -f 4)"

if [ -z "${DOWNLOAD_URL}" ]; then
  echo "Could not find release asset for ${ASSET}" >&2
  exit 1
fi

echo "Downloading ${BINARY} for ${TARGET}..."
mkdir -p "${INSTALL_DIR}"
curl -sfL "${DOWNLOAD_URL}" -o "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
echo ""
echo "Make sure ${INSTALL_DIR} is in your PATH."
echo ""
echo "Add to your Claude Code hooks (~/.claude/settings.json):"
echo ""
cat << 'HOOKEOF'
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "command",
        "command": "code-trace"
      }]
    }]
  }
}
HOOKEOF
```

- [ ] **Step 4: Commit**

```bash
git add install.sh
chmod +x install.sh
git commit -m "feat: install script for curl|bash installation"
```

---

### Task 10: Integration test

**Files:**
- Create: `tests/integration_test.rs`
- Create: `tests/fixtures/payload.json`
- Create: `tests/fixtures/transcript_simple.jsonl`
- Create: `tests/fixtures/transcript_tools.jsonl`

- [ ] **Step 1: Create test fixtures**

`tests/fixtures/payload.json`:

```json
{
  "sessionId": "test-session-abc",
  "transcriptPath": "PLACEHOLDER",
  "cwd": "/tmp"
}
```

`tests/fixtures/transcript_simple.jsonl`:

```jsonl
{"type":"user","message":{"role":"user","content":"Hello"}}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Hi there! How can I help?"}]}}
```

`tests/fixtures/transcript_tools.jsonl`:

```jsonl
{"type":"user","message":{"role":"user","content":"List the files"}}
{"type":"assistant","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Let me check."},{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"README.md\nsrc/"}]}}
{"type":"assistant","message":{"id":"msg_2","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"Here are the files:\n- README.md\n- src/"}]}}
```

- [ ] **Step 2: Write integration test**

`tests/integration_test.rs`:

```rust
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use tempfile::NamedTempFile;

// We test the public API of the library modules by importing them.
// This requires making the modules public or using a lib.rs. For simplicity,
// we test via the binary's internal logic by re-testing the core pipeline.

#[test]
fn end_to_end_simple_transcript() {
    // Write transcript to temp file
    let mut transcript = NamedTempFile::new().unwrap();
    writeln!(transcript, r#"{{"type":"user","message":{{"role":"user","content":"Hello"}}}}"#).unwrap();
    writeln!(transcript, r#"{{"type":"assistant","message":{{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-20250514","content":[{{"type":"text","text":"Hi there!"}}]}}}}"#).unwrap();

    // Simulate the pipeline
    let payload = json!({
        "sessionId": "test-session",
        "transcriptPath": transcript.path().to_string_lossy().to_string(),
        "cwd": "/tmp"
    });

    // Parse payload
    let session_id = payload["sessionId"].as_str().unwrap();
    let transcript_path = std::path::Path::new(payload["transcriptPath"].as_str().unwrap());

    // Read transcript
    let mut ss = code_trace::state::SessionState::default();
    let msgs = code_trace::transcript::read_new_jsonl(transcript_path, &mut ss);
    assert_eq!(msgs.len(), 2);

    // Build turns
    let turns = code_trace::turns::build_turns(msgs);
    assert_eq!(turns.len(), 1);

    // Build batch
    let tags = vec!["claude-code".to_string(), "test".to_string()];
    let events = code_trace::emit::build_ingestion_batch(
        session_id,
        1,
        &turns[0],
        transcript_path,
        &tags,
    );

    // Verify: 1 trace + 1 generation = 2 events
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

    // trace + generation + 1 tool span
    assert_eq!(events.len(), 3);
    assert_eq!(events[2]["type"], "span-create");
    assert_eq!(events[2]["body"]["name"], "Tool: Bash");
    // Tool output should be present
    assert_eq!(events[2]["body"]["output"], json!("README.md"));
}
```

- [ ] **Step 3: Add lib.rs for integration test access**

Create `src/lib.rs`:

```rust
pub mod emit;
pub mod log;
pub mod payload;
pub mod state;
pub mod tags;
pub mod transcript;
pub mod turns;
```

- [ ] **Step 4: Run all tests**

Run: `cd /home/doug/projects/code-trace && cargo test`
Expected: All unit tests and integration tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs tests/ 
git commit -m "feat: integration tests with fixtures"
```

---

### Task 11: GitHub Actions CI and release workflow

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create CI workflow**

`.github/workflows/ci.yml`:

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test
      - run: cargo clippy -- -D warnings
```

- [ ] **Step 2: Create release workflow**

`.github/workflows/release.yml`:

```yaml
name: Release
on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  build:
    strategy:
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-latest
          - target: x86_64-apple-darwin
            os: macos-latest
          - target: aarch64-apple-darwin
            os: macos-latest

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install cross-compilation tools
        if: matrix.target == 'aarch64-unknown-linux-gnu'
        run: |
          sudo apt-get update
          sudo apt-get install -y gcc-aarch64-linux-gnu
          echo "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc" >> $GITHUB_ENV

      - run: cargo build --release --target ${{ matrix.target }}

      - name: Package binary
        run: |
          cp target/${{ matrix.target }}/release/code-trace code-trace-${{ matrix.target }}

      - uses: softprops/action-gh-release@v2
        with:
          files: code-trace-${{ matrix.target }}
```

- [ ] **Step 3: Commit**

```bash
git add .github/
git commit -m "ci: add CI and release workflows"
```

---

### Task 12: Final polish and push

**Files:**
- Modify: `CLAUDE.md`
- Modify: `install.sh` (update REPO owner)

- [ ] **Step 1: Update CLAUDE.md**

Replace `CLAUDE.md` with:

```markdown
# code-trace

Claude Code -> Langfuse trace hook, written in Rust.

## Build

```bash
cargo build --release
```

## Test

```bash
cargo test
```

## Install

```bash
curl -sfL https://raw.githubusercontent.com/OWNER/code-trace/main/install.sh | bash
```

## Configuration

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "command",
        "command": "code-trace"
      }]
    }]
  }
}
```

Set in `.claude/settings.local.json` per project:

```json
{
  "env": {
    "TRACE_TO_LANGFUSE": "true",
    "LANGFUSE_PUBLIC_KEY": "pk-lf-...",
    "LANGFUSE_SECRET_KEY": "sk-lf-...",
    "LANGFUSE_BASE_URL": "https://cloud.langfuse.com"
  }
}
```

## Env vars

| Variable | Description |
|---|---|
| `TRACE_TO_LANGFUSE` | Set to `true` to enable |
| `LANGFUSE_PUBLIC_KEY` / `CC_LANGFUSE_PUBLIC_KEY` | Langfuse public key |
| `LANGFUSE_SECRET_KEY` / `CC_LANGFUSE_SECRET_KEY` | Langfuse secret key |
| `LANGFUSE_BASE_URL` / `CC_LANGFUSE_BASE_URL` | Langfuse host (default: cloud) |
| `CC_TRACE_DEBUG` | Set to `true` for debug logging |
| `CC_TRACE_MAX_CHARS` | Max chars before truncation (default: 20000) |
```

- [ ] **Step 2: Update install.sh REPO variable**

Update the `REPO=` line with the actual GitHub owner/repo once known.

- [ ] **Step 3: Create GitHub repo and push**

```bash
cd /home/doug/projects/code-trace
gh repo create code-trace --public --source=. --push
```

- [ ] **Step 4: Tag and release**

```bash
git tag v0.1.0
git push origin v0.1.0
```

Expected: GitHub Actions builds binaries for all 4 targets and creates a release.
