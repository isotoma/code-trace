use code_trace::{emit, log, opencode, payload, pi_agent, state, tags, transcript, turns};
use std::time::Instant;

fn run() -> i32 {
    let start = Instant::now();
    log::debug("code-trace started");

    if std::env::var("TRACE_TO_LANGFUSE")
        .unwrap_or_default()
        .to_lowercase()
        != "true"
    {
        return 0;
    }

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


    let _lock = state::FileLock::acquire();
    let mut global_state = state::load_state();
    state::prune_old_sessions(&mut global_state);

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
            let mut ss = global_state.get(&key).cloned().unwrap_or_default();

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

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in &built_turns {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    &transcript_path,
                    &env_tags,
                    source,
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.insert(key, ss);
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
            let mut ss = global_state.get(&key).cloned().unwrap_or_default();

            let normalized = opencode::normalize_opencode_messages(messages);
            let built_turns = turns::build_turns(normalized);
            if built_turns.is_empty() {
                global_state.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in &built_turns {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    std::path::Path::new("opencode"),
                    &env_tags,
                    source,
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.insert(key, ss);
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
            let mut ss = global_state.get(&key).cloned().unwrap_or_default();

            let normalized = pi_agent::normalize_pi_agent_messages(messages);
            let built_turns = turns::build_turns(normalized);
            if built_turns.is_empty() {
                global_state.insert(key, ss);
                state::save_state(&global_state);
                return 0;
            }

            let mut all_events = Vec::new();
            let mut emitted = 0u32;
            for t in &built_turns {
                emitted += 1;
                let turn_num = ss.turn_count + emitted;
                let events = emit::build_ingestion_batch(
                    &session_id,
                    turn_num,
                    t,
                    std::path::Path::new("pi-agent"),
                    &env_tags,
                    source,
                );
                all_events.extend(events);
            }

            ss.turn_count += emitted;
            state::touch(&mut ss);
            global_state.insert(key, ss);
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
    std::process::exit(run());
}
