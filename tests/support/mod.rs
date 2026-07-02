//! Shared support for integration tests: isolated environments, crafted
//! payloads, and the fake Langfuse server. Test crates pull this in with
//! `mod support;`. Not every helper is used by every test crate.
#![allow(dead_code)]

pub mod fake_langfuse;

pub use fake_langfuse::FakeLangfuse;

use code_trace::state::State;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

pub const BIN: &str = env!("CARGO_BIN_EXE_code-trace");

/// Isolated environment for one binary invocation set: HOME/XDG_* point at a
/// temp dir so the developer's real code-trace state is never touched.
pub struct TestEnv {
    pub home: TempDir,
    pub langfuse_url: Option<String>,
    pub sync_send: bool,
}

impl TestEnv {
    pub fn new() -> Self {
        TestEnv {
            home: TempDir::new().unwrap(),
            langfuse_url: None,
            sync_send: false,
        }
    }

    /// Environment wired to a Langfuse endpoint with the fake's credentials.
    pub fn with_langfuse(url: &str) -> Self {
        TestEnv {
            home: TempDir::new().unwrap(),
            langfuse_url: Some(url.to_string()),
            sync_send: false,
        }
    }

    /// Enable CODE_TRACE_SYNC_SEND=1: process exit implies all sends done,
    /// making "nothing was sent" assertions exact.
    pub fn sync_send(mut self) -> Self {
        self.sync_send = true;
        self
    }

    pub fn data_dir(&self) -> PathBuf {
        self.home.path().join("data").join("code-trace")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data_dir().join("state.json")
    }

    /// The lock file the binary flocks around state access. Tests acquire it
    /// themselves to sequence concurrent invocations.
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir().join("state.lock")
    }

    pub fn write_state(&self, state: &State) {
        std::fs::create_dir_all(self.data_dir()).unwrap();
        std::fs::write(self.state_file(), serde_json::to_string(state).unwrap()).unwrap();
    }

    pub fn read_state(&self) -> State {
        let buf = std::fs::read_to_string(self.state_file()).unwrap_or_default();
        code_trace::state::parse_state_json(&buf)
    }

    /// A Command with the isolated environment applied, not yet spawned —
    /// callers needing concurrency spawn and wait themselves.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env_clear()
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"));
        if let Some(url) = &self.langfuse_url {
            cmd.env("TRACE_TO_LANGFUSE", "true")
                .env("LANGFUSE_PUBLIC_KEY", fake_langfuse::PUBLIC_KEY)
                .env("LANGFUSE_SECRET_KEY", fake_langfuse::SECRET_KEY)
                .env("LANGFUSE_BASE_URL", url);
        }
        if self.sync_send {
            cmd.env("CODE_TRACE_SYNC_SEND", "1");
        }
        cmd
    }

    /// Run to completion with optional stdin; returns (exit code, stdout, stderr).
    pub fn run(&self, args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
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

/// Claude Code Stop-hook payload.
pub fn stop_payload(session_id: &str, transcript: &Path) -> String {
    serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript.to_string_lossy(),
        "cwd": "/tmp"
    })
    .to_string()
}

/// Claude Code SessionStart payload; `source` is "startup"|"resume"|"clear"|"compact".
pub fn session_start_payload(session_id: &str, transcript: &Path, source: &str) -> String {
    serde_json::json!({
        "hook_event_name": "SessionStart",
        "source": source,
        "session_id": session_id,
        "transcript_path": transcript.to_string_lossy(),
        "cwd": "/tmp"
    })
    .to_string()
}

/// One transcript turn: a user line and an assistant line with distinct text.
fn turn_lines(turn: u32) -> String {
    format!(
        "{}\n{}\n",
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": format!("question {turn}")}
        }),
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": format!("msg_{turn}"),
                "role": "assistant",
                "model": "claude-sonnet-4-20250514",
                "content": [{"type": "text", "text": format!("answer {turn}")}]
            }
        }),
    )
}

/// Write a fresh transcript containing turns 1..=n; returns its path.
pub fn write_transcript(dir: &Path, name: &str, turns: u32) -> PathBuf {
    let path = dir.join(name);
    let content: String = (1..=turns).map(turn_lines).collect();
    std::fs::write(&path, content).unwrap();
    path
}

/// Append one more turn (numbered `turn`) to an existing transcript.
pub fn append_turn(path: &Path, turn: u32) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(turn_lines(turn).as_bytes()).unwrap();
}
