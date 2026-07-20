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

// --- setup --write-config ---

/// Run `setup --write-config` against `config_file` with extra args. Every call
/// passes a deterministic email source (`--user-email` or `--no-prompt`) so the
/// binary never blocks on the interactive terminal prompt.
fn write_config(config_file: &Path, extra: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(BIN);
    cmd.args(["setup", "--write-config", "--config-file"]);
    cmd.arg(config_file);
    cmd.args(extra);
    cmd.output().expect("failed to run code-trace setup --write-config")
}

fn config_user_id(config_file: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(config_file).ok()?;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == "LANGFUSE_USER_ID" {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

#[test]
fn write_config_fresh_with_email_sets_active_user_id() {
    let file = scratch("cfg-fresh-email").join("config");
    assert!(write_config(&file, &["--user-email", "me@example.com"]).status.success());
    assert_eq!(config_user_id(&file).as_deref(), Some("me@example.com"));
}

#[test]
fn write_config_fresh_without_email_leaves_placeholder_commented() {
    let file = scratch("cfg-fresh-noprompt").join("config");
    assert!(write_config(&file, &["--no-prompt"]).status.success());
    let contents = std::fs::read_to_string(&file).unwrap();
    assert!(contents.contains("# LANGFUSE_USER_ID=you@example.com"));
    assert_eq!(config_user_id(&file), None, "placeholder must stay commented");
}

#[test]
fn write_config_appends_email_to_existing_file() {
    let file = scratch("cfg-append").join("config");
    std::fs::write(&file, "TRACE_TO_LANGFUSE=true\nLANGFUSE_PUBLIC_KEY=pk\n").unwrap();
    assert!(write_config(&file, &["--user-email", "doug@example.com"]).status.success());
    assert_eq!(config_user_id(&file).as_deref(), Some("doug@example.com"));
    // Existing keys preserved.
    assert!(std::fs::read_to_string(&file).unwrap().contains("LANGFUSE_PUBLIC_KEY=pk"));
}

#[test]
fn write_config_leaves_existing_active_user_id_untouched() {
    let file = scratch("cfg-already").join("config");
    let original = "LANGFUSE_USER_ID=old@example.com\n";
    std::fs::write(&file, original).unwrap();
    assert!(write_config(&file, &["--user-email", "new@example.com"]).status.success());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        original,
        "an already-configured user id must not be overwritten"
    );
}

#[test]
fn write_config_existing_file_without_email_is_noop() {
    let file = scratch("cfg-noop").join("config");
    let original = "TRACE_TO_LANGFUSE=true\n";
    std::fs::write(&file, original).unwrap();
    assert!(write_config(&file, &["--no-prompt"]).status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}

// --- setup plugin install ---

/// Run `setup` with `$HOME` pointed at an isolated directory, so plugin paths
/// and agent detection resolve under the scratch dir.
fn setup_with_home(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .arg("setup")
        .args(args)
        .env("HOME", home)
        .output()
        .expect("failed to run code-trace setup")
}

fn repo_file(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel))
        .expect("read repo plugin source")
}

#[test]
fn install_opencode_writes_embedded_plugin() {
    let home = scratch("oc-install");
    assert!(setup_with_home(&home, &["--install-opencode"]).status.success());
    let installed = home.join(".config/opencode/plugins/code-trace.ts");
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        repo_file("plugin/opencode/code-trace.ts"),
        "installed plugin must match the repo source embedded in the binary"
    );
}

#[test]
fn install_pi_writes_embedded_extension() {
    let home = scratch("pi-install");
    assert!(setup_with_home(&home, &["--install-pi"]).status.success());
    let installed = home.join(".pi/agent/extensions/code-trace.ts");
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        repo_file("plugin/pi-agent/code-trace.ts")
    );
}

#[test]
fn offer_opencode_skips_when_not_detected() {
    let home = scratch("oc-offer-absent");
    assert!(setup_with_home(&home, &["--offer-opencode"]).status.success());
    assert!(
        !home.join(".config/opencode/plugins/code-trace.ts").exists(),
        "must not install when OpenCode is not detected (no prompt)"
    );
}

#[test]
fn offer_pi_skips_when_not_detected() {
    let home = scratch("pi-offer-absent");
    assert!(setup_with_home(&home, &["--offer-pi"]).status.success());
    assert!(!home.join(".pi/agent/extensions/code-trace.ts").exists());
}
