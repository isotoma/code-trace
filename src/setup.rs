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

fn default_config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("code-trace").join("config")
}

/// True if the config text has an active (uncommented) `LANGFUSE_USER_ID`
/// setting. Uses the same parser the binary reads config with at runtime, so a
/// commented `# LANGFUSE_USER_ID=...` placeholder does not count.
pub fn config_has_user_id(contents: &str) -> bool {
    crate::config::parse_config_str(contents).contains_key("LANGFUSE_USER_ID")
}

/// What writing the config should do, given the current file contents (None if
/// the file is absent) and a resolved email (None to leave the id unset).
#[derive(Debug, PartialEq, Eq)]
enum WriteAction {
    /// An active LANGFUSE_USER_ID is already present; leave the file untouched.
    AlreadyConfigured,
    /// No file yet — create it from the template, with the email if given.
    Create(Option<String>),
    /// File exists without a user id and an email was given — append it.
    AppendEmail(String),
    /// File exists without a user id and no email was given — nothing to do.
    ExistsNoChange,
}

fn decide_write(existing: Option<&str>, email: Option<&str>) -> WriteAction {
    match existing {
        None => WriteAction::Create(email.map(str::to_string)),
        Some(contents) if config_has_user_id(contents) => WriteAction::AlreadyConfigured,
        Some(_) => match email {
            Some(e) => WriteAction::AppendEmail(e.to_string()),
            None => WriteAction::ExistsNoChange,
        },
    }
}

/// Full contents of a freshly created config file.
fn fresh_config(email: Option<&str>) -> String {
    let user_id_line = match email {
        Some(e) => format!("LANGFUSE_USER_ID={e}"),
        None => "# LANGFUSE_USER_ID=you@example.com".to_string(),
    };
    format!(
        "# code-trace configuration
# Set TRACE_TO_LANGFUSE=true and add your Langfuse keys to enable tracing.
TRACE_TO_LANGFUSE=false
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
{user_id_line}
# CODE_TRACE_DEBUG=false
"
    )
}

/// `existing` with an active LANGFUSE_USER_ID line appended, inserting a
/// separating newline first if the file did not already end with one.
fn with_appended_user_id(existing: &str, email: &str) -> String {
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("LANGFUSE_USER_ID={email}\n"));
    out
}

// The agent plugin/extension sources are baked into the binary, so plugin
// install works even under `curl | bash`, where there is no local checkout to
// copy from.
const OPENCODE_PLUGIN: &str = include_str!("../plugin/opencode/code-trace.ts");
const PI_EXTENSION: &str = include_str!("../plugin/pi-agent/code-trace.ts");

fn opencode_plugin_path(home: &Path) -> PathBuf {
    home.join(".config/opencode/plugins/code-trace.ts")
}

fn pi_extension_path(home: &Path) -> PathBuf {
    home.join(".pi/agent/extensions/code-trace.ts")
}

/// OpenCode is considered present if its config directory (or config file)
/// exists under `home`.
fn opencode_detected(home: &Path) -> bool {
    home.join(".config/opencode").is_dir() || home.join(".config/opencode/opencode.json").is_file()
}

/// Pi is considered present if its agent directory exists under `home`.
fn pi_detected(home: &Path) -> bool {
    home.join(".pi/agent").is_dir()
}

/// Write `contents` to `target`, creating parent directories as needed.
fn install_plugin(target: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    std::fs::write(target, contents)
        .map_err(|e| format!("could not write {}: {e}", target.display()))
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

/// Read a line from the controlling terminal: stdin when it is a TTY (e.g.
/// `bash install.sh`), else `/dev/tty` (so a piped `curl | bash` install can
/// still prompt), else None when no terminal exists. Because this runs in the
/// binary — a separate process from the installer — reading the terminal never
/// consumes the piped install script, unlike a shell `read` from stdin.
fn read_terminal_line() -> Option<String> {
    use std::io::{BufRead, IsTerminal};
    if std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().lock().read_line(&mut s).ok()?;
        Some(s)
    } else if let Ok(tty) = std::fs::File::open("/dev/tty") {
        let mut s = String::new();
        std::io::BufReader::new(tty).read_line(&mut s).ok()?;
        Some(s)
    } else {
        None
    }
}

/// Prompt for an email to use as the Langfuse user id. None if no terminal is
/// available or the answer is blank. Whitespace is stripped entirely, matching
/// the previous shell prompt's `tr -d`.
fn prompt_email() -> Option<String> {
    use std::io::Write;
    eprintln!();
    eprintln!("Optionally attach your email to traces as the Langfuse user id");
    eprintln!("(enables Langfuse's per-user views). Leave blank to skip.");
    eprint!("Email [skip]: ");
    let _ = std::io::stderr().flush();

    let line = read_terminal_line()?;
    let stripped: String = line.split_whitespace().collect();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
}

/// Create or update the config file, attaching `explicit_email` as the Langfuse
/// user id — or, when none is given and `allow_prompt` is set, whatever the
/// interactive prompt returns. An already-configured user id is left untouched.
pub fn write_config(
    config_path: &Path,
    explicit_email: Option<&str>,
    allow_prompt: bool,
) -> Result<(), String> {
    let existing = match std::fs::read_to_string(config_path) {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("could not read {}: {e}", config_path.display())),
    };

    let already_configured = existing.as_deref().is_some_and(config_has_user_id);

    // Resolve the email: an explicit value wins; otherwise prompt only when a
    // user id is not already set and prompting is allowed.
    let email: Option<String> = if already_configured {
        None
    } else if let Some(e) = explicit_email {
        let stripped: String = e.split_whitespace().collect();
        (!stripped.is_empty()).then_some(stripped)
    } else if allow_prompt {
        prompt_email()
    } else {
        None
    };

    let display = config_path.display();
    match decide_write(existing.as_deref(), email.as_deref()) {
        WriteAction::AlreadyConfigured => {
            println!("LANGFUSE_USER_ID already configured in {display} — leaving as-is");
        }
        WriteAction::Create(email) => {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
            }
            std::fs::write(config_path, fresh_config(email.as_deref()))
                .map_err(|e| format!("could not write {display}: {e}"))?;
            println!("Created config file: {display}");
            if let Some(e) = email {
                println!("Set LANGFUSE_USER_ID={e}");
            }
        }
        WriteAction::AppendEmail(e) => {
            let base = existing.unwrap_or_default();
            std::fs::write(config_path, with_appended_user_id(&base, &e))
                .map_err(|err| format!("could not write {display}: {err}"))?;
            println!("Set LANGFUSE_USER_ID={e} in {display}");
        }
        WriteAction::ExistsNoChange => {
            println!("Config file already exists: {display}");
        }
    }
    Ok(())
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "~".to_string()))
}

/// Ask a yes/no question on the terminal. None when no terminal is available;
/// otherwise true only for an exact `y`/`Y`, matching the previous shell prompt.
fn prompt_yes_no(question: &str) -> Option<bool> {
    use std::io::Write;
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let line = read_terminal_line()?;
    let answer = line.trim();
    Some(answer == "y" || answer == "Y")
}

/// Install an agent plugin/extension, either forced or offered (detected +
/// confirmed at the prompt). Returns a process exit code (0 = success/skipped).
fn offer_or_install(
    force: bool,
    detected: bool,
    target: &Path,
    contents: &str,
    detected_line: &str,
    question: &str,
    installed_line: &str,
) -> i32 {
    let should_install = if force {
        true
    } else if detected {
        println!();
        println!("{detected_line}");
        println!("  {}", target.display());
        matches!(prompt_yes_no(question), Some(true))
    } else {
        false
    };

    if !should_install {
        return 0;
    }

    match install_plugin(target, contents) {
        Ok(()) => {
            println!("{installed_line} {}", target.display());
            0
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

/// `code-trace setup` entry point. `args` is everything after `setup`.
pub fn run(args: &[String]) -> i32 {
    let mut register = false;
    let mut write = false;
    let mut no_prompt = false;
    let mut install_opencode = false;
    let mut offer_opencode = false;
    let mut install_pi = false;
    let mut offer_pi = false;
    let mut settings_path: Option<PathBuf> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut user_email: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--register-hook" => register = true,
            "--write-config" => write = true,
            "--no-prompt" => no_prompt = true,
            "--install-opencode" => install_opencode = true,
            "--offer-opencode" => offer_opencode = true,
            "--install-pi" => install_pi = true,
            "--offer-pi" => offer_pi = true,
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
            "--config-file" => {
                i += 1;
                match args.get(i) {
                    Some(p) => config_path = Some(PathBuf::from(p)),
                    None => {
                        eprintln!("setup: --config-file requires a path");
                        return 2;
                    }
                }
            }
            "--user-email" => {
                i += 1;
                match args.get(i) {
                    Some(e) => user_email = Some(e.clone()),
                    None => {
                        eprintln!("setup: --user-email requires a value");
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

    let any_action =
        register || write || install_opencode || offer_opencode || install_pi || offer_pi;
    if !any_action {
        eprintln!(
            "setup: nothing to do (expected one of --register-hook, --write-config, \
             --install-opencode, --offer-opencode, --install-pi, --offer-pi)"
        );
        return 2;
    }

    let mut code = 0;
    let home = home_dir();

    if register {
        let path = settings_path.unwrap_or_else(default_settings_path);
        match register_hook(&path) {
            Ok(()) => println!("Registered code-trace Stop hook in {}", path.display()),
            Err(msg) => {
                eprintln!("{msg}");
                code = 1;
            }
        }
    }

    if install_opencode || offer_opencode {
        code |= offer_or_install(
            install_opencode,
            opencode_detected(&home),
            &opencode_plugin_path(&home),
            OPENCODE_PLUGIN,
            "OpenCode detected. Install the code-trace plugin?",
            "Install OpenCode plugin? [y/N] ",
            "Installed OpenCode plugin to",
        );
    }

    if install_pi || offer_pi {
        code |= offer_or_install(
            install_pi,
            pi_detected(&home),
            &pi_extension_path(&home),
            PI_EXTENSION,
            "Pi Agent detected. Install the code-trace extension?",
            "Install Pi Agent extension? [y/N] ",
            "Installed Pi Agent extension to",
        );
    }

    if write {
        let path = config_path.unwrap_or_else(default_config_path);
        if let Err(msg) = write_config(&path, user_email.as_deref(), !no_prompt) {
            eprintln!("{msg}");
            code = 1;
        }
    }

    code
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

    // --- config writing ---

    #[test]
    fn active_user_id_is_detected() {
        assert!(config_has_user_id("LANGFUSE_USER_ID=me@example.com\n"));
        assert!(config_has_user_id("  LANGFUSE_USER_ID = me@example.com \n"));
    }

    #[test]
    fn commented_or_absent_user_id_is_not_detected() {
        assert!(!config_has_user_id("# LANGFUSE_USER_ID=you@example.com\n"));
        assert!(!config_has_user_id("TRACE_TO_LANGFUSE=true\nLANGFUSE_PUBLIC_KEY=pk\n"));
        assert!(!config_has_user_id(""));
    }

    #[test]
    fn decide_create_when_no_file() {
        assert_eq!(decide_write(None, None), WriteAction::Create(None));
        assert_eq!(
            decide_write(None, Some("me@example.com")),
            WriteAction::Create(Some("me@example.com".to_string()))
        );
    }

    #[test]
    fn decide_append_when_file_lacks_user_id() {
        assert_eq!(
            decide_write(Some("TRACE_TO_LANGFUSE=true\n"), Some("me@example.com")),
            WriteAction::AppendEmail("me@example.com".to_string())
        );
    }

    #[test]
    fn decide_noop_when_file_lacks_user_id_and_no_email() {
        assert_eq!(
            decide_write(Some("TRACE_TO_LANGFUSE=true\n"), None),
            WriteAction::ExistsNoChange
        );
    }

    #[test]
    fn decide_already_configured_wins_over_email() {
        assert_eq!(
            decide_write(Some("LANGFUSE_USER_ID=old@example.com\n"), Some("new@example.com")),
            WriteAction::AlreadyConfigured
        );
    }

    #[test]
    fn fresh_config_without_email_has_commented_placeholder() {
        let c = fresh_config(None);
        assert!(!config_has_user_id(&c), "placeholder must be commented");
        assert!(c.contains("# LANGFUSE_USER_ID=you@example.com"));
        assert!(c.contains("TRACE_TO_LANGFUSE=false"));
        assert!(c.contains("# CODE_TRACE_DEBUG=false"));
        assert!(c.ends_with('\n'));
    }

    #[test]
    fn fresh_config_with_email_sets_active_user_id() {
        let c = fresh_config(Some("me@example.com"));
        assert!(config_has_user_id(&c));
        assert_eq!(
            crate::config::parse_config_str(&c).get("LANGFUSE_USER_ID"),
            Some(&"me@example.com".to_string())
        );
    }

    #[test]
    fn append_user_id_adds_active_line() {
        let out = with_appended_user_id("TRACE_TO_LANGFUSE=true\n", "me@example.com");
        assert!(out.ends_with("LANGFUSE_USER_ID=me@example.com\n"));
        assert!(config_has_user_id(&out));
    }

    #[test]
    fn append_user_id_inserts_newline_when_file_has_no_trailing_newline() {
        // A hand-edited file without a trailing newline must not concatenate.
        let out = with_appended_user_id("TRACE_TO_LANGFUSE=true", "me@example.com");
        assert!(out.contains("TRACE_TO_LANGFUSE=true\nLANGFUSE_USER_ID=me@example.com\n"));
        assert!(config_has_user_id(&out));
    }

    // --- plugin install ---

    /// A unique empty temp directory for a test.
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-trace-ut-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn embedded_plugin_sources_are_present() {
        assert!(!OPENCODE_PLUGIN.is_empty());
        assert!(!PI_EXTENSION.is_empty());
        assert!(OPENCODE_PLUGIN.contains("code-trace"));
        assert!(PI_EXTENSION.contains("code-trace"));
    }

    #[test]
    fn plugin_paths_are_under_home() {
        assert_eq!(
            opencode_plugin_path(Path::new("/home/x")),
            Path::new("/home/x/.config/opencode/plugins/code-trace.ts")
        );
        assert_eq!(
            pi_extension_path(Path::new("/home/x")),
            Path::new("/home/x/.pi/agent/extensions/code-trace.ts")
        );
    }

    #[test]
    fn opencode_detected_only_when_config_dir_exists() {
        let home = temp_home("oc-detect");
        assert!(!opencode_detected(&home));
        std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
        assert!(opencode_detected(&home));
    }

    #[test]
    fn pi_detected_only_when_agent_dir_exists() {
        let home = temp_home("pi-detect");
        assert!(!pi_detected(&home));
        std::fs::create_dir_all(home.join(".pi/agent")).unwrap();
        assert!(pi_detected(&home));
    }

    #[test]
    fn install_plugin_creates_parents_and_writes_contents() {
        let home = temp_home("install");
        let target = home.join(".config/opencode/plugins/code-trace.ts");
        install_plugin(&target, "hello world").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello world");
    }
}
