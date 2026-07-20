//! Integration coverage for `code-trace setup --register-hook`, driving the
//! actual built binary against throwaway settings files.
//!
//! This is the Claude Code hook-registration logic that used to live in
//! install.sh as an embedded python3 program; it now ships in the binary, so
//! these tests exercise the real CLI end to end (parse → read → transform →
//! write). The pure transform is additionally unit-tested in `src/setup.rs`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Path to the binary under test (set by Cargo for integration tests).
const BIN: &str = env!("CARGO_BIN_EXE_code-trace");

/// A unique scratch directory under Cargo's per-suite temp dir.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn register(settings_file: &Path) -> std::process::Output {
    Command::new(BIN)
        .args(["setup", "--register-hook", "--settings-file"])
        .arg(settings_file)
        .output()
        .expect("failed to run code-trace setup")
}

/// Commands of every code-trace Stop hook in the written file, in order.
fn code_trace_commands(settings_file: &Path) -> Vec<String> {
    let contents = std::fs::read_to_string(settings_file).expect("read settings file");
    let settings: Value = serde_json::from_str(&contents).expect("settings file is valid JSON");
    let mut cmds = Vec::new();
    if let Some(stop) = settings.pointer("/hooks/Stop").and_then(|v| v.as_array()) {
        for entry in stop {
            if let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) {
                for h in hooks {
                    if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                        let base = Path::new(cmd.split_whitespace().next().unwrap_or(""))
                            .file_name()
                            .and_then(|s| s.to_str());
                        if base == Some("code-trace") {
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
fn fresh_install_registers_one_canonical_hook() {
    let file = scratch("fresh").join("settings.json");
    let out = register(&file);
    assert!(out.status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);
}

#[test]
fn legacy_absolute_path_hook_is_migrated_and_settings_preserved() {
    let file = scratch("legacy").join("settings.json");
    std::fs::write(
        &file,
        r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"~/.claude/hooks/code-trace"}]}]}}"#,
    )
    .unwrap();
    assert!(register(&file).status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);

    let settings: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(settings["model"], "opus", "unrelated settings preserved");
}

#[test]
fn existing_canonical_hook_stays_single() {
    let file = scratch("already").join("settings.json");
    std::fs::write(
        &file,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"code-trace"}]}]}}"#,
    )
    .unwrap();
    assert!(register(&file).status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);
}

#[test]
fn canonical_added_alongside_unrelated_hook() {
    let file = scratch("coexist").join("settings.json");
    std::fs::write(
        &file,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"some-other-tool"}]}]}}"#,
    )
    .unwrap();
    assert!(register(&file).status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);

    let contents = std::fs::read_to_string(&file).unwrap();
    assert!(
        contents.contains("some-other-tool"),
        "unrelated Stop hook must be preserved"
    );
}

#[test]
fn running_twice_is_idempotent() {
    let file = scratch("twice").join("settings.json");
    assert!(register(&file).status.success());
    assert!(register(&file).status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);
}

#[test]
fn legacy_hook_with_args_is_migrated_not_duplicated() {
    let file = scratch("args").join("settings.json");
    std::fs::write(
        &file,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/opt/bin/code-trace --verbose"}]}]}}"#,
    )
    .unwrap();
    assert!(register(&file).status.success());
    assert_eq!(code_trace_commands(&file), vec!["code-trace"]);
}

#[test]
fn invalid_json_is_refused_without_overwriting() {
    let file = scratch("invalid").join("settings.json");
    let original = "{ this is not valid json";
    std::fs::write(&file, original).unwrap();

    let out = register(&file);
    assert!(!out.status.success(), "must exit non-zero on invalid JSON");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        original,
        "existing file must be left untouched"
    );
}
