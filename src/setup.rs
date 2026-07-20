//! `code-trace setup` — configures the host agent so it invokes the binary.
//!
//! Logic that used to live in `install.sh` as an embedded `python3` program
//! moves here, so it is written in Rust, unit-tested, and needs no interpreter
//! on the target machine. The shell installer shrinks to a bootstrap that
//! downloads the binary and calls these subcommands.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// True if a hook command invokes the code-trace binary in any form: the bare
/// `code-trace`, an absolute/relative path such as `~/.claude/hooks/code-trace`
/// or `/usr/local/bin/code-trace`, and any of these with trailing arguments.
pub fn is_code_trace_command(cmd: &str) -> bool {
    match cmd.split_whitespace().next() {
        Some(first) => {
            Path::new(first).file_name().and_then(|s| s.to_str()) == Some("code-trace")
        }
        None => false,
    }
}

/// Register (or migrate) the canonical code-trace Stop hook in a Claude Code
/// settings document.
///
/// Idempotent and self-healing: any pre-existing Stop hook that invokes
/// code-trace — bare, by absolute path, or the legacy `~/.claude/hooks`
/// layout — is stripped and replaced with the canonical PATH-based
/// `code-trace` command, collapsing duplicates to a single entry. Unrelated
/// settings and unrelated Stop hooks are preserved. A non-object document (or
/// non-object `hooks` / non-array `Stop`) is reset to a well-formed shape.
pub fn register_stop_hook(mut settings: Value) -> Value {
    // A non-object document (e.g. a JSON array or scalar) is discarded.
    if !settings.is_object() {
        settings = json!({});
    }
    let obj = settings.as_object_mut().expect("settings is an object");

    // `hooks` must be an object; reset it otherwise.
    if !obj.get("hooks").is_some_and(Value::is_object) {
        obj.insert("hooks".to_string(), json!({}));
    }
    let hooks = obj["hooks"].as_object_mut().expect("hooks is an object");

    // `hooks.Stop` must be an array; reset it otherwise.
    if !hooks.get("Stop").is_some_and(Value::is_array) {
        hooks.insert("Stop".to_string(), json!([]));
    }
    let stop = hooks["Stop"].as_array_mut().expect("Stop is an array");

    // Strip every existing code-trace hook from all matcher entries (migration
    // + dedup). A non-string or missing command is left alone.
    for entry in stop.iter_mut() {
        if let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
            inner.retain(|h| match h.get("command").and_then(Value::as_str) {
                Some(cmd) => !is_code_trace_command(cmd),
                None => true,
            });
        }
    }

    // Add the canonical hook to the first matcher-style entry, or create one.
    let canonical = json!({"type": "command", "command": "code-trace"});
    let appended = stop.iter_mut().any(|entry| {
        match entry.get_mut("hooks").and_then(Value::as_array_mut) {
            Some(inner) => {
                inner.push(canonical.clone());
                true
            }
            None => false,
        }
    });
    if !appended {
        stop.push(json!({"hooks": [canonical]}));
    }

    settings
}

fn default_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    PathBuf::from(home).join(".claude").join("settings.json")
}

/// Read the settings file (treating a missing file as empty), register the
/// canonical hook, and write it back with 2-space indentation. Refuses to
/// overwrite a file whose existing contents are not valid JSON.
pub fn register_hook(settings_path: &Path) -> Result<(), String> {
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }

    let settings = match std::fs::read_to_string(settings_path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|_| "Existing settings file is not valid JSON; refusing to overwrite.".to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("could not read {}: {e}", settings_path.display())),
    };

    let updated = register_stop_hook(settings);
    let mut out = serde_json::to_string_pretty(&updated)
        .map_err(|e| format!("could not serialize settings: {e}"))?;
    out.push('\n');
    std::fs::write(settings_path, out)
        .map_err(|e| format!("could not write {}: {e}", settings_path.display()))?;
    Ok(())
}

/// `code-trace setup` entry point. `args` is everything after `setup`.
pub fn run(args: &[String]) -> i32 {
    let mut register = false;
    let mut settings_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--register-hook" => register = true,
            "--settings-file" => {
                i += 1;
                match args.get(i) {
                    Some(p) => settings_path = Some(PathBuf::from(p)),
                    None => {
                        eprintln!("setup: --settings-file requires a path");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("setup: unknown argument '{other}'");
                return 2;
            }
        }
        i += 1;
    }

    if !register {
        eprintln!("setup: nothing to do (expected --register-hook)");
        return 2;
    }

    let path = settings_path.unwrap_or_else(default_settings_path);
    match register_hook(&path) {
        Ok(()) => {
            println!("Registered code-trace Stop hook in {}", path.display());
            0
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Commands of every code-trace Stop hook in the document, in order.
    fn code_trace_commands(settings: &Value) -> Vec<String> {
        let mut cmds = Vec::new();
        if let Some(stop) = settings.pointer("/hooks/Stop").and_then(|v| v.as_array()) {
            for entry in stop {
                if let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) {
                    for h in hooks {
                        if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                            if is_code_trace_command(cmd) {
                                cmds.push(cmd.to_string());
                            }
                        }
                    }
                }
            }
        }
        cmds
    }

    #[test]
    fn recognises_bare_command() {
        assert!(is_code_trace_command("code-trace"));
    }

    #[test]
    fn recognises_absolute_path_with_args() {
        assert!(is_code_trace_command("/opt/bin/code-trace --verbose"));
    }

    #[test]
    fn recognises_legacy_home_hooks_path() {
        assert!(is_code_trace_command("~/.claude/hooks/code-trace"));
    }

    #[test]
    fn rejects_unrelated_command() {
        assert!(!is_code_trace_command("some-other-tool"));
    }

    #[test]
    fn rejects_empty_and_similar_names() {
        assert!(!is_code_trace_command(""));
        assert!(!is_code_trace_command("   "));
        assert!(!is_code_trace_command("code-tracer"));
    }

    #[test]
    fn fresh_document_gets_one_canonical_hook() {
        let out = register_stop_hook(json!({}));
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }

    #[test]
    fn legacy_absolute_path_hook_is_migrated_and_settings_preserved() {
        let input = json!({
            "model": "opus",
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "~/.claude/hooks/code-trace"}]}]}
        });
        let out = register_stop_hook(input);
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
        assert_eq!(out["model"], "opus");
    }

    #[test]
    fn existing_canonical_hook_stays_single() {
        let input = json!({
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "code-trace"}]}]}
        });
        let out = register_stop_hook(input);
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }

    #[test]
    fn canonical_added_alongside_unrelated_stop_hook() {
        let input = json!({
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "some-other-tool"}]}]}
        });
        let out = register_stop_hook(input);
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
        // Unrelated hook preserved.
        let stop = out.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        let has_other = stop.iter().any(|e| {
            e.get("hooks").and_then(|v| v.as_array()).is_some_and(|hs| {
                hs.iter()
                    .any(|h| h.get("command").and_then(|c| c.as_str()) == Some("some-other-tool"))
            })
        });
        assert!(has_other, "unrelated Stop hook must be preserved");
    }

    #[test]
    fn running_twice_is_idempotent() {
        let once = register_stop_hook(json!({}));
        let twice = register_stop_hook(once);
        assert_eq!(code_trace_commands(&twice), vec!["code-trace"]);
    }

    #[test]
    fn legacy_hook_with_args_is_migrated_not_duplicated() {
        let input = json!({
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/opt/bin/code-trace --verbose"}]}]}
        });
        let out = register_stop_hook(input);
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }

    #[test]
    fn non_object_document_is_reset() {
        let out = register_stop_hook(json!([1, 2, 3]));
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }

    #[test]
    fn non_object_hooks_is_reset() {
        let out = register_stop_hook(json!({"hooks": "nonsense"}));
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }

    #[test]
    fn non_array_stop_is_reset() {
        let out = register_stop_hook(json!({"hooks": {"Stop": "nonsense"}}));
        assert_eq!(code_trace_commands(&out), vec!["code-trace"]);
    }
}
