use crate::{langfuse, log, payload, state};
use std::io::BufRead;

/// SessionStart handler. Records the session, prints a one-line tracing
/// status reminder (SessionStart stdout is injected as agent context), and
/// never reads the transcript or emits.
pub fn on_start() -> i32 {
    let raw = payload::read_stdin();
    let input = payload::parse_payload(&raw);
    let Some(session_id) = input.session_id().map(String::from) else {
        log::debug("--on-start: missing session_id");
        return 0;
    };
    let transcript_str = input
        .transcript_path()
        .map(|p| p.to_string_lossy().to_string());
    let cwd = input.cwd().map(String::from);

    let _lock = state::FileLock::acquire();
    let mut st = state::load_state();
    st.prune();
    st.record_session(
        input.source().as_str(),
        &session_id,
        transcript_str.as_deref(),
        cwd.as_deref(),
    );
    let suppressed = st.is_suppressed(&session_id);
    state::save_state(&st);

    if !langfuse::tracing_enabled() {
        return 0;
    }
    let Some(config) = langfuse::config_from_env() else {
        return 0;
    };
    if suppressed {
        println!("code-trace: tracing PAUSED for this session (private mode).");
    } else {
        println!(
            "⚠️ code-trace: tracing ENABLED → {}. Use the pause command to make this session private.",
            config.host
        );
    }
    0
}

pub fn status() -> i32 {
    match (langfuse::tracing_enabled(), langfuse::config_from_env()) {
        (true, Some(config)) => println!("tracing: ENABLED → {}", config.host),
        (true, None) => println!("tracing: not configured (TRACE_TO_LANGFUSE set but keys missing)"),
        (false, Some(_)) => println!("tracing: disabled (keys configured, TRACE_TO_LANGFUSE not true)"),
        (false, None) => println!("tracing: not configured"),
    }
    let st = state::load_state();
    let suppressed = st.sessions.values().filter(|r| r.suppressed).count();
    let active = st.sessions.len() - suppressed;
    println!("sessions: {active} active, {suppressed} suppressed");
    0
}

pub fn sessions() -> i32 {
    let st = state::load_state();
    if st.sessions.is_empty() {
        println!("no sessions recorded");
        return 0;
    }
    let mut records: Vec<_> = st.sessions.values().collect();
    records.sort_by(|a, b| b.last_seen_epoch.cmp(&a.last_seen_epoch));
    println!(
        "{:<14} {:<12} {:<10} {:<17} TRANSCRIPT",
        "SESSION", "SOURCE", "PAUSED", "LAST SEEN"
    );
    for r in records {
        let id_short: String = r.session_id.chars().take(12).collect();
        let last_seen = chrono::DateTime::from_timestamp(r.last_seen_epoch as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<14} {:<12} {:<10} {:<17} {}",
            id_short,
            r.source,
            if r.suppressed { "yes" } else { "no" },
            last_seen,
            r.transcript_path.as_deref().unwrap_or("-"),
        );
    }
    0
}

/// Resolve the target session for pause/resume: explicit --session <id>,
/// otherwise the most-recently-seen entry.
fn target_session(st: &state::State, args: &[String]) -> Result<String, String> {
    if let Some(pos) = args.iter().position(|a| a == "--session") {
        let id = args
            .get(pos + 1)
            .ok_or_else(|| "--session requires a session id".to_string())?;
        if !st.sessions.contains_key(id) {
            return Err(format!(
                "unknown session '{id}' — run 'code-trace sessions' to list known sessions"
            ));
        }
        return Ok(id.clone());
    }
    st.most_recent_session()
        .map(|r| r.session_id.clone())
        .ok_or_else(|| "no sessions recorded yet — nothing to target".to_string())
}

fn set_suppression(args: &[String], suppressed: bool) -> i32 {
    let verb = if suppressed { "paused" } else { "resumed" };
    let _lock = state::FileLock::acquire();
    let mut st = state::load_state();
    let session_id = match target_session(&st, args) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    st.set_suppressed(&session_id, suppressed);
    let source = st.sessions[&session_id].source.clone();
    state::save_state(&st);
    // Always name the target: with parallel sessions the caller must be able
    // to confirm the right one was hit.
    println!("{verb} tracing for session {session_id} ({source})");
    0
}

pub fn pause(args: &[String]) -> i32 {
    set_suppression(args, true)
}

pub fn resume(args: &[String]) -> i32 {
    set_suppression(args, false)
}

struct PurgeArgs {
    session_id: String,
    langfuse_only: bool,
    local_only: bool,
    yes: bool,
    transcript_path: Option<String>,
}

fn parse_purge_args(args: &[String]) -> Result<PurgeArgs, String> {
    let mut session_id = None;
    let mut langfuse_only = false;
    let mut local_only = false;
    let mut yes = false;
    let mut transcript_path = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session" => {
                session_id = Some(
                    args.get(i + 1)
                        .ok_or("--session requires a session id")?
                        .clone(),
                );
                i += 2;
            }
            "--transcript-path" => {
                transcript_path = Some(
                    args.get(i + 1)
                        .ok_or("--transcript-path requires a path")?
                        .clone(),
                );
                i += 2;
            }
            "--langfuse-only" => {
                langfuse_only = true;
                i += 1;
            }
            "--local-only" => {
                local_only = true;
                i += 1;
            }
            "--yes" | "-y" => {
                yes = true;
                i += 1;
            }
            other => return Err(format!("unknown purge option '{other}'")),
        }
    }
    let session_id = session_id.ok_or("usage: code-trace purge --session <id> [--langfuse-only] [--local-only] [--yes] [--transcript-path <p>]")?;
    if langfuse_only && local_only {
        return Err("--langfuse-only and --local-only are mutually exclusive".to_string());
    }
    Ok(PurgeArgs {
        session_id,
        langfuse_only,
        local_only,
        yes,
        transcript_path,
    })
}

fn confirm(prompt: &str) -> bool {
    println!("{prompt} [y/N]");
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

pub fn purge(args: &[String]) -> i32 {
    let parsed = match parse_purge_args(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let _lock = state::FileLock::acquire();
    let mut st = state::load_state();
    let record = st.sessions.get(&parsed.session_id).cloned();
    let transcript_path = parsed
        .transcript_path
        .clone()
        .or_else(|| record.as_ref().and_then(|r| r.transcript_path.clone()));

    let mut plan = Vec::new();
    if !parsed.local_only {
        plan.push(format!("Langfuse traces for session {}", parsed.session_id));
    }
    if !parsed.langfuse_only {
        if let Some(t) = &transcript_path {
            plan.push(format!("local transcript {t}"));
        }
        plan.push("code-trace state for the session".to_string());
    }
    if !parsed.yes && !confirm(&format!("Delete: {}?", plan.join(", "))) {
        println!("aborted, nothing deleted");
        return 0;
    }

    let mut code = 0;

    if !parsed.local_only {
        match langfuse::config_from_env() {
            Some(config) => {
                match langfuse::list_trace_ids(&config, &parsed.session_id)
                    .and_then(|ids| langfuse::bulk_delete_traces(&config, &ids))
                {
                    Ok(count) => println!("deleted {count} Langfuse traces"),
                    Err(e) => {
                        eprintln!("error: Langfuse purge failed: {e}");
                        code = 1;
                    }
                }
            }
            None => {
                eprintln!("error: Langfuse keys not configured; cannot purge traces");
                code = 1;
            }
        }
    }

    if !parsed.langfuse_only {
        if let Some(t) = &transcript_path {
            let path = std::path::Path::new(t);
            if path.exists() {
                match std::fs::remove_file(path) {
                    Ok(()) => println!("deleted transcript {t}"),
                    Err(e) => {
                        eprintln!("error: could not delete transcript {t}: {e}");
                        code = 1;
                    }
                }
            }
        }
        if st.remove_session(&parsed.session_id).is_some() {
            state::save_state(&st);
            println!("removed session from code-trace state");
        } else {
            println!("session not in code-trace state (nothing to remove)");
        }
    }

    code
}

pub fn version() -> i32 {
    println!("code-trace {}", env!("CARGO_PKG_VERSION"));
    0
}

pub fn help() -> i32 {
    println!(
        "code-trace {} — send agent session traces to Langfuse

USAGE:
    code-trace                          read a Stop-hook payload on stdin and emit (default)
    code-trace --on-start               SessionStart handler: record session, print tracing reminder
    code-trace status                   show tracing configuration and session counts
    code-trace sessions                 list known sessions (most recent first)
    code-trace pause [--session <id>]   pause tracing for a session (default: most recent)
    code-trace resume [--session <id>]  resume tracing for a session (default: most recent)
    code-trace purge --session <id>     delete a session's Langfuse traces, transcript, and state
        [--langfuse-only]               only delete Langfuse traces
        [--local-only]                  only delete transcript and state
        [--transcript-path <p>]         transcript path for sessions not in the registry
        [--yes]                         skip confirmation
    code-trace --version                print version
    code-trace --help                   this help",
        env!("CARGO_PKG_VERSION")
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn purge_args_require_session() {
        assert!(parse_purge_args(&[]).is_err());
        assert!(parse_purge_args(&s(&["--yes"])).is_err());
    }

    #[test]
    fn purge_args_parse_all_flags() {
        let p = parse_purge_args(&s(&[
            "--session",
            "abc",
            "--langfuse-only",
            "--yes",
            "--transcript-path",
            "/tmp/t.jsonl",
        ]))
        .unwrap();
        assert_eq!(p.session_id, "abc");
        assert!(p.langfuse_only);
        assert!(!p.local_only);
        assert!(p.yes);
        assert_eq!(p.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    }

    #[test]
    fn purge_args_reject_conflicting_scopes() {
        assert!(parse_purge_args(&s(&["--session", "a", "--langfuse-only", "--local-only"])).is_err());
    }

    #[test]
    fn target_session_prefers_explicit_id() {
        let mut st = state::State::default();
        st.record_session("claude-code", "s1", None, None);
        st.record_session("claude-code", "s2", None, None);
        st.sessions.get_mut("s1").unwrap().last_seen_epoch = 100;
        st.sessions.get_mut("s2").unwrap().last_seen_epoch = 200;
        assert_eq!(target_session(&st, &s(&["--session", "s1"])).unwrap(), "s1");
        assert_eq!(target_session(&st, &[]).unwrap(), "s2");
        assert!(target_session(&st, &s(&["--session", "nope"])).is_err());
        assert!(target_session(&state::State::default(), &[]).is_err());
    }
}
