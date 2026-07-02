use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;

use crate::log;

fn state_dir() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("code-trace");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn old_state_file() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("state").join("code_trace_state.json"))
}

fn state_file() -> Option<PathBuf> {
    Some(state_dir()?.join("state.json"))
}

fn lock_file() -> Option<PathBuf> {
    Some(state_dir()?.join("state.lock"))
}

/// RAII file lock using a blocking exclusive flock. Every state
/// read-modify-write cycle must run under this lock: without it, concurrent
/// invocations (parallel agents' hooks, pause/purge commands) are
/// last-writer-wins on state.json and can silently drop a pause.
///
/// Invariant: acquire at most ONCE per process, at the entry point, before
/// the first state access — a second blocking acquisition on a new file
/// descriptor would deadlock against our own lock. Helpers like `load_state`
/// and `save_state` never acquire it themselves.
///
/// `purge` holds the lock across its Langfuse HTTP calls, so concurrent
/// hooks queue behind it — accepted: purge is rare and bounded by HTTP
/// timeouts, and releasing mid-purge would reopen the read-modify-write gap.
///
/// On genuine flock failure (not contention) this logs and proceeds
/// unlocked — degraded but loud, and no worse than not having a lock.
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
            Err(e) => {
                log::error(&format!("state lock: cannot open {}: {e}", path.display()));
                return FileLock { _file: None };
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            loop {
                let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
                if rc == 0 {
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                log::error(&format!("state lock: flock failed: {err}; proceeding unlocked"));
                break;
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
    #[serde(default)]
    pub updated_epoch: u64,
}

pub type GlobalState = HashMap<String, SessionState>;

/// Per-session metadata registry entry. Unlike cursors, this keeps the raw
/// session id and transcript path so sessions can be listed, paused, and purged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub source: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub suppressed: bool,
    #[serde(default)]
    pub last_seen_epoch: u64,
    pub cursor_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub cursors: GlobalState,
    #[serde(default)]
    pub sessions: HashMap<String, SessionRecord>,
}

impl State {
    /// Insert or refresh a session record. An existing `suppressed` flag is
    /// preserved — a paused session stays paused across hook invocations.
    pub fn record_session(
        &mut self,
        source: &str,
        session_id: &str,
        transcript_path: Option<&str>,
        cwd: Option<&str>,
    ) {
        // handle matches the emit path: transcript path for Claude Code,
        // session id for sources without a local transcript.
        let handle = transcript_path.unwrap_or(session_id);
        let cursor_key = state_key(source, session_id, handle);
        let record = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionRecord {
                session_id: session_id.to_string(),
                source: source.to_string(),
                transcript_path: None,
                cwd: None,
                suppressed: false,
                last_seen_epoch: 0,
                cursor_key: cursor_key.clone(),
            });
        record.source = source.to_string();
        if let Some(t) = transcript_path {
            record.transcript_path = Some(t.to_string());
        }
        if let Some(c) = cwd {
            record.cwd = Some(c.to_string());
        }
        record.cursor_key = cursor_key;
        record.last_seen_epoch = now_epoch();
    }

    pub fn is_suppressed(&self, session_id: &str) -> bool {
        self.sessions
            .get(session_id)
            .map(|r| r.suppressed)
            .unwrap_or(false)
    }

    /// Returns false if the session is not in the registry.
    pub fn set_suppressed(&mut self, session_id: &str, suppressed: bool) -> bool {
        match self.sessions.get_mut(session_id) {
            Some(r) => {
                r.suppressed = suppressed;
                true
            }
            None => false,
        }
    }

    /// Drop a session's registry entry and its linked cursor.
    pub fn remove_session(&mut self, session_id: &str) -> Option<SessionRecord> {
        let record = self.sessions.remove(session_id)?;
        self.cursors.remove(&record.cursor_key);
        Some(record)
    }

    pub fn most_recent_session(&self) -> Option<&SessionRecord> {
        self.sessions.values().max_by_key(|r| r.last_seen_epoch)
    }

    /// Age-prune cursors and active registry entries. Suppressed entries are
    /// never pruned: a private session must stay private if resumed later.
    pub fn prune(&mut self) {
        prune_old_sessions(&mut self.cursors);
        let now = now_epoch();
        self.sessions.retain(|_, r| {
            r.suppressed
                || r.last_seen_epoch == 0
                || now.saturating_sub(r.last_seen_epoch) < MAX_AGE_SECS
        });
    }
}

const MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60; // 7 days

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Remove sessions older than 7 days.
pub fn prune_old_sessions(state: &mut GlobalState) {
    let now = now_epoch();
    state.retain(|_, ss| {
        ss.updated_epoch == 0 || now.saturating_sub(ss.updated_epoch) < MAX_AGE_SECS
    });
}

pub fn touch(ss: &mut SessionState) {
    ss.updated_epoch = now_epoch();
}

pub fn state_key(source: &str, session_id: &str, handle: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{source}:{session_id}:{handle}"));
    format!("{:x}", hasher.finalize())
}

/// Parse persisted state, migrating the legacy shape (a flat map of
/// hash → cursor state) by wrapping it. Cursor entries must survive
/// unchanged: losing an offset re-emits every prior turn as a duplicate.
pub fn parse_state_json(buf: &str) -> State {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(buf) else {
        return State::default();
    };
    if value.get("cursors").is_some() {
        return serde_json::from_value(value).unwrap_or_default();
    }
    let cursors: GlobalState = serde_json::from_value(value).unwrap_or_default();
    State {
        cursors,
        sessions: HashMap::new(),
    }
}

pub fn load_state() -> State {
    let Some(path) = state_file() else {
        return State::default();
    };
    if path.exists() {
        let Ok(mut file) = File::open(&path) else {
            return State::default();
        };
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            return State::default();
        }
        return parse_state_json(&buf);
    }
    if let Some(old_path) = old_state_file() {
        if old_path.exists() {
            if let Ok(mut file) = File::open(&old_path) {
                let mut buf = String::new();
                if file.read_to_string(&mut buf).is_ok() {
                    let state = parse_state_json(&buf);
                    if !state.cursors.is_empty() {
                        save_state(&state);
                        return state;
                    }
                }
            }
        }
    }
    State::default()
}

pub fn save_state(state: &State) {
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
        let k1 = state_key("claude-code", "sess1", "/tmp/t.jsonl");
        let k2 = state_key("claude-code", "sess1", "/tmp/t.jsonl");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 64); // sha256 hex
    }

    #[test]
    fn state_key_differs_for_different_inputs() {
        let k1 = state_key("claude-code", "sess1", "/tmp/a.jsonl");
        let k2 = state_key("claude-code", "sess1", "/tmp/b.jsonl");
        assert_ne!(k1, k2);
    }

    #[test]
    fn state_key_differs_for_different_sources() {
        let k1 = state_key("claude-code", "sess1", "/tmp/t.jsonl");
        let k2 = state_key("opencode", "sess1", "/tmp/t.jsonl");
        assert_ne!(k1, k2);
    }

    #[test]
    fn session_state_defaults() {
        let ss = SessionState::default();
        assert_eq!(ss.offset, 0);
        assert_eq!(ss.buffer, "");
        assert_eq!(ss.turn_count, 0);
    }

    #[test]
    fn legacy_flat_map_migrates_with_cursors_intact() {
        let legacy = r#"{
            "abc123": {"offset": 4242, "buffer": "partial", "turn_count": 7, "updated_epoch": 1751400000},
            "def456": {"offset": 100}
        }"#;
        let state = parse_state_json(legacy);
        assert!(state.sessions.is_empty());
        assert_eq!(state.cursors.len(), 2);
        let c = &state.cursors["abc123"];
        assert_eq!(c.offset, 4242);
        assert_eq!(c.buffer, "partial");
        assert_eq!(c.turn_count, 7);
        assert_eq!(c.updated_epoch, 1751400000);
        assert_eq!(state.cursors["def456"].offset, 100);
    }

    #[test]
    fn new_shape_loads_directly() {
        let json = r#"{
            "cursors": {"abc": {"offset": 5}},
            "sessions": {"s1": {"session_id": "s1", "source": "claude-code", "suppressed": true, "last_seen_epoch": 1, "cursor_key": "abc"}}
        }"#;
        let state = parse_state_json(json);
        assert_eq!(state.cursors["abc"].offset, 5);
        assert!(state.sessions["s1"].suppressed);
    }

    #[test]
    fn garbage_and_empty_input_yield_default_state() {
        assert!(parse_state_json("not json").cursors.is_empty());
        let state = parse_state_json("{}");
        assert!(state.cursors.is_empty());
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn roundtrip_preserves_both_maps() {
        let mut state = State::default();
        state.cursors.insert("k".into(), SessionState { offset: 9, ..Default::default() });
        state.record_session("claude-code", "s1", Some("/tmp/t.jsonl"), Some("/tmp"));
        let json = serde_json::to_string(&state).unwrap();
        let back = parse_state_json(&json);
        assert_eq!(back.cursors["k"].offset, 9);
        assert_eq!(back.sessions["s1"].transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    }

    #[test]
    fn record_session_sets_cursor_key_matching_emit_path() {
        let mut state = State::default();
        state.record_session("claude-code", "s1", Some("/tmp/t.jsonl"), None);
        assert_eq!(
            state.sessions["s1"].cursor_key,
            state_key("claude-code", "s1", "/tmp/t.jsonl")
        );
        // Sources without a transcript use the session id as handle.
        state.record_session("opencode", "s2", None, None);
        assert_eq!(state.sessions["s2"].cursor_key, state_key("opencode", "s2", "s2"));
    }

    #[test]
    fn record_session_refresh_preserves_suppressed() {
        let mut state = State::default();
        state.record_session("claude-code", "s1", Some("/tmp/t.jsonl"), None);
        assert!(state.set_suppressed("s1", true));
        state.sessions.get_mut("s1").unwrap().last_seen_epoch = 0;
        state.record_session("claude-code", "s1", Some("/tmp/t.jsonl"), Some("/home"));
        let r = &state.sessions["s1"];
        assert!(r.suppressed);
        assert!(r.last_seen_epoch > 0);
        assert_eq!(r.cwd.as_deref(), Some("/home"));
    }

    #[test]
    fn set_suppressed_unknown_session_returns_false() {
        let mut state = State::default();
        assert!(!state.set_suppressed("nope", true));
        assert!(!state.is_suppressed("nope"));
    }

    #[test]
    fn remove_session_drops_registry_and_cursor() {
        let mut state = State::default();
        state.record_session("claude-code", "s1", Some("/tmp/t.jsonl"), None);
        let key = state.sessions["s1"].cursor_key.clone();
        state.cursors.insert(key.clone(), SessionState { offset: 10, ..Default::default() });
        state.cursors.insert("other".into(), SessionState::default());
        let removed = state.remove_session("s1");
        assert!(removed.is_some());
        assert!(!state.sessions.contains_key("s1"));
        assert!(!state.cursors.contains_key(&key));
        assert!(state.cursors.contains_key("other"));
    }

    #[test]
    fn most_recent_session_picks_highest_last_seen() {
        let mut state = State::default();
        state.record_session("claude-code", "old", None, None);
        state.record_session("claude-code", "new", None, None);
        state.sessions.get_mut("old").unwrap().last_seen_epoch = 100;
        state.sessions.get_mut("new").unwrap().last_seen_epoch = 200;
        assert_eq!(state.most_recent_session().unwrap().session_id, "new");
    }

    #[test]
    fn prune_removes_old_active_but_keeps_suppressed() {
        let mut state = State::default();
        let stale = now_epoch() - MAX_AGE_SECS - 1;
        for (id, suppressed) in [("active-old", false), ("private-old", true), ("active-new", false)] {
            state.record_session("claude-code", id, None, None);
            state.sessions.get_mut(id).unwrap().suppressed = suppressed;
        }
        state.sessions.get_mut("active-old").unwrap().last_seen_epoch = stale;
        state.sessions.get_mut("private-old").unwrap().last_seen_epoch = stale;
        state.prune();
        assert!(!state.sessions.contains_key("active-old"));
        assert!(state.sessions.contains_key("private-old"));
        assert!(state.sessions.contains_key("active-new"));
    }
}
