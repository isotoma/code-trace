use code_trace::{cli, config, emit, langfuse, log, opencode, payload, pi_agent, state, tags, transcript, turns};
use std::time::Instant;

/// Turns to skip when a session is seen for the first time (no cursor yet):
/// everything before the turn that fired this hook predates code-trace
/// seeing the session — pre-install/pre-enable history is never emitted.
fn first_contact_skip(first_contact: bool, turns: usize, session_id: &str) -> usize {
    if !first_contact || turns <= 1 {
        return 0;
    }
    let skipped = turns - 1;
    log::info(&format!(
        "first contact with session {session_id}: skipped {skipped} pre-existing turns"
    ));
    skipped
}

/// Advance a suppressed session's cursor past turns it will never emit, so
/// they cannot replay after `resume`. Paused means never traced, not deferred.
fn consume_suppressed(
    global_state: &mut state::State,
    key: String,
    mut ss: state::SessionState,
    turns: usize,
    session_id: &str,
) {
    ss.turn_count += turns as u32;
    state::touch(&mut ss);
    global_state.cursors.insert(key, ss);
    state::save_state(global_state);
    log::debug(&format!(
        "session {session_id} suppressed; consumed {turns} turns without emitting"
    ));
}

fn run() -> i32 {
    let start = Instant::now();
    log::debug("code-trace started");

    if !langfuse::tracing_enabled() {
        return 0;
    }

    let Some(config) = langfuse::config_from_env() else {
        return 0;
    };

    let raw = payload::read_stdin();
    let input = payload::parse_payload(&raw);

    let source = input.source();
    let session_id = match input.session_id() {
        Some(s) => s.to_string(),
        None => {
            log::debug("Missing session_id; exiting");
            return 0;
        }
    };

    let cwd = input.cwd().map(String::from);
    let env_tags = tags::gather_env_tags(source, cwd.as_deref(), input.agent_version());
    let user_id = langfuse::user_id_from_env();

    let _lock = state::FileLock::acquire();
    let mut global_state = state::load_state();
    global_state.prune();

    // Privacy guarantee: a suppressed session's input is still consumed (the
    // cursor advances past its turns, in each source arm below) but no event
    // is ever built and no send is ever forked — turns that occur while
    // paused are never traced, including after a later resume.
    let transcript_str = input
        .transcript_path()
        .map(|p| p.to_string_lossy().to_string());
    global_state.record_session(
        source.as_str(),
        &session_id,
        transcript_str.as_deref(),
        cwd.as_deref(),
    );
    let suppressed = global_state.is_suppressed(&session_id);
    state::save_state(&global_state);

    match input {
        payload::Input::ClaudeCode {
            session_id: _,
            transcript_path,
            cwd: _,
        } => {
            let Some(transcript_path) = transcript_path else {
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

            let key = state::state_key(
                source.as_str(),
                &session_id,
                &transcript_path.to_string_lossy(),
            );
            let first_contact = !global_state.cursors.contains_key(&key);
            let mut ss = global_state.cursors.get(&key).cloned().unwrap_or_default();

            let msgs = transcript::read_new_jsonl(&transcript_path, &mut ss);
            if msgs.is_empty() {
                global_state.cursors.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            let built_turns = turns::build_turns(msgs);
            if built_turns.is_empty() {
                global_state.cursors.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            if suppressed {
                consume_suppressed(&mut global_state, key, ss, built_turns.len(), &session_id);
                return 0;
            }

            let skipped = first_contact_skip(first_contact, built_turns.len(), &session_id);
            ss.turn_count += skipped as u32;

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in built_turns.iter().skip(skipped) {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    &transcript_path,
                    &env_tags,
                    source,
                    user_id.as_deref(),
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.cursors.insert(key, ss);
            state::save_state(&global_state);

            emit::send_batch_fire_and_forget(&config, all_events);

            let dur = start.elapsed();
            log::info(&format!(
                "Processed {emitted} turns in {:.2}s (session={session_id})",
                dur.as_secs_f64()
            ));
        }

        payload::Input::Opencode {
            session_id: _,
            cwd: _,
            messages,
            agent_version: _,
        } => {
            if messages.is_empty() {
                return 0;
            }

            let key = state::state_key(source.as_str(), &session_id, &session_id);
            let first_contact = !global_state.cursors.contains_key(&key);
            let mut ss = global_state.cursors.get(&key).cloned().unwrap_or_default();

            let normalized = opencode::normalize_opencode_messages(messages);
            let built_turns = turns::build_turns(normalized);
            if built_turns.is_empty() {
                global_state.cursors.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            if suppressed {
                consume_suppressed(&mut global_state, key, ss, built_turns.len(), &session_id);
                return 0;
            }

            let skipped = first_contact_skip(first_contact, built_turns.len(), &session_id);
            ss.turn_count += skipped as u32;

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in built_turns.iter().skip(skipped) {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    std::path::Path::new("opencode"),
                    &env_tags,
                    source,
                    user_id.as_deref(),
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.cursors.insert(key, ss);
            state::save_state(&global_state);

            emit::send_batch_fire_and_forget(&config, all_events);

            let dur = start.elapsed();
            log::info(&format!(
                "Processed {emitted} turns in {:.2}s (session={session_id})",
                dur.as_secs_f64()
            ));
        }

        payload::Input::PiAgent {
            session_id: _,
            cwd: _,
            messages,
            agent_version: _,
        } => {
            if messages.is_empty() {
                return 0;
            }

            let key = state::state_key(source.as_str(), &session_id, &session_id);
            let first_contact = !global_state.cursors.contains_key(&key);
            let mut ss = global_state.cursors.get(&key).cloned().unwrap_or_default();

            let normalized = pi_agent::normalize_pi_agent_messages(messages);
            let built_turns = turns::build_turns(normalized);
            if built_turns.is_empty() {
                global_state.cursors.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            if suppressed {
                consume_suppressed(&mut global_state, key, ss, built_turns.len(), &session_id);
                return 0;
            }

            let skipped = first_contact_skip(first_contact, built_turns.len(), &session_id);
            ss.turn_count += skipped as u32;

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in built_turns.iter().skip(skipped) {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    std::path::Path::new("pi-agent"),
                    &env_tags,
                    source,
                    user_id.as_deref(),
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.cursors.insert(key, ss);
            state::save_state(&global_state);

            emit::send_batch_fire_and_forget(&config, all_events);

            let dur = start.elapsed();
            log::info(&format!(
                "Processed {emitted} turns in {:.2}s (session={session_id})",
                dur.as_secs_f64()
            ));
        }
    }

    0
}

fn main() {
    let file_config = config::load_config();
    for (k, v) in &file_config {
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }

    let args: Vec<String> = std::env::args().collect();
    // Known subcommands/flags dispatch; anything else falls through to the
    // stdin/emit path so the installed Stop hook keeps working unchanged.
    let code = match args.get(1).map(String::as_str) {
        Some("--on-start") => cli::on_start(),
        Some("status") => cli::status(),
        Some("sessions") => cli::sessions(),
        Some("pause") => cli::pause(&args[2..]),
        Some("resume") => cli::resume(&args[2..]),
        Some("purge") => cli::purge(&args[2..]),
        Some("--version") | Some("-V") => cli::version(),
        Some("--help") | Some("-h") => cli::help(),
        _ => run(),
    };
    std::process::exit(code);
}
