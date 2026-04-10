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
