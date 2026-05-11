# Unified Config File Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `~/.config/code-trace/config` as a unified config source so credentials work identically for Claude Code, OpenCode, and Pi without setting per-agent env vars.

**Architecture:** A new `src/config.rs` module parses `KEY=value` files. `src/main.rs` calls it at startup and sets missing env vars from the file before any other reads. The TypeScript plugins drop their `TRACE_TO_LANGFUSE` guard — the binary handles the enabled check using the merged config.

**Tech Stack:** Rust (std only, no new deps), TypeScript (plugins), bash (install script)

---

## File Map

| File | Change |
|------|--------|
| `src/config.rs` | **Create** — `load_config()` parses config file, returns `HashMap<String, String>` |
| `src/lib.rs` | **Modify** — add `pub mod config;` |
| `src/main.rs` | **Modify** — call `load_config()`, populate missing env vars before credentials read |
| `plugin/code-trace.ts` | **Modify** — remove `TRACE_TO_LANGFUSE` guard at line 77 |
| `plugin/pi-agent/code-trace.ts` | **Modify** — remove `TRACE_TO_LANGFUSE` guard at line 45 |
| `install.sh` | **Modify** — create `~/.config/code-trace/config` if absent |
| `README.md` | **Modify** — restructure configuration section |

---

### Task 1: `src/config.rs` — config file parser

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add a new file `src/config.rs` with tests only (no impl yet):

```rust
use std::collections::HashMap;

pub fn load_config() -> HashMap<String, String> {
    let path = config_path();
    parse_config_file(&path)
}

fn config_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            std::path::PathBuf::from(home).join(".config")
        });
    base.join("code-trace").join("config")
}

fn parse_config_file(path: &std::path::Path) -> HashMap<String, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_config_str(&content)
}

fn parse_config_str(content: &str) -> HashMap<String, String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_value_pairs() {
        let content = "TRACE_TO_LANGFUSE=true\nLANGFUSE_PUBLIC_KEY=pk-lf-abc\n";
        let map = parse_config_str(content);
        assert_eq!(map.get("TRACE_TO_LANGFUSE"), Some(&"true".to_string()));
        assert_eq!(map.get("LANGFUSE_PUBLIC_KEY"), Some(&"pk-lf-abc".to_string()));
    }

    #[test]
    fn skips_comment_lines() {
        let content = "# this is a comment\nTRACE_TO_LANGFUSE=true\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("TRACE_TO_LANGFUSE"), Some(&"true".to_string()));
    }

    #[test]
    fn skips_blank_lines() {
        let content = "\nTRACE_TO_LANGFUSE=true\n\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn skips_lines_without_equals() {
        let content = "INVALID_LINE\nTRACE_TO_LANGFUSE=true\n";
        let map = parse_config_str(content);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn value_can_contain_equals() {
        let content = "LANGFUSE_SECRET_KEY=sk-lf-a=b=c\n";
        let map = parse_config_str(content);
        assert_eq!(map.get("LANGFUSE_SECRET_KEY"), Some(&"sk-lf-a=b=c".to_string()));
    }

    #[test]
    fn missing_file_returns_empty_map() {
        let map = parse_config_file(std::path::Path::new("/tmp/code-trace-does-not-exist.cfg"));
        assert!(map.is_empty());
    }
}
```

- [ ] **Step 2: Register the module in `src/lib.rs`**

Add `pub mod config;` as the first line:

```rust
pub mod config;
pub mod emit;
pub mod log;
pub mod opencode;
pub mod payload;
pub mod pi_agent;
pub mod source;
pub mod state;
pub mod tags;
pub mod transcript;
pub mod turns;
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo test config
```

Expected: compile error or test failure — `parse_config_str` has `todo!()`.

- [ ] **Step 4: Implement `parse_config_str`**

Replace the `todo!()` body in `src/config.rs`:

```rust
fn parse_config_str(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test config
```

Expected: all 6 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat: add config file parser for ~/.config/code-trace/config"
```

---

### Task 2: Wire config into `src/main.rs`

**Files:**
- Modify: `src/main.rs`

The strategy: call `config::load_config()` once at the start of `run()`, then for each key in the map, call `std::env::set_var(key, value)` only if the env var is not already set. This means env vars always win over file values. All subsequent `std::env::var()` calls further down `run()` naturally see the merged result.

- [ ] **Step 1: Write a test for the env-merge behaviour**

Add a new integration test at the bottom of `tests/integration_test.rs`:

```rust
#[test]
fn config_env_var_takes_priority_over_file() {
    // env var already set → config file value must NOT override it
    std::env::set_var("_CT_TEST_KEY", "from_env");
    let mut file_values = std::collections::HashMap::new();
    file_values.insert("_CT_TEST_KEY".to_string(), "from_file".to_string());
    for (k, v) in &file_values {
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }
    assert_eq!(std::env::var("_CT_TEST_KEY").unwrap(), "from_env");
    std::env::remove_var("_CT_TEST_KEY");
}
```

- [ ] **Step 2: Run the test to verify it passes immediately**

```bash
cargo test config_env_var_takes_priority_over_file
```

Expected: PASS (this test documents the behaviour, not new code).

- [ ] **Step 3: Add config loading to `src/main.rs`**

First, add `config` to the existing `use` import at the top of `src/main.rs`:

```rust
use code_trace::{config, emit, log, opencode, payload, pi_agent, state, tags, transcript, turns};
```

Then insert the config loading block at the very start of `run()`, before the `Instant::now()` call:

```rust
fn run() -> i32 {
    let file_config = config::load_config();
    for (k, v) in &file_config {
        if std::env::var(k).is_err() {
            std::env::set_var(k, v);
        }
    }

    let start = Instant::now();
    log::debug("code-trace started");

    if std::env::var("TRACE_TO_LANGFUSE")
        .unwrap_or_default()
        .to_lowercase()
        != "true"
    {
        return 0;
    }
    // ... rest of run() unchanged
```

- [ ] **Step 4: Build to verify it compiles**

```bash
cargo build 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 5: Run full test suite**

```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/integration_test.rs
git commit -m "feat: load config file at startup, env vars take priority"
```

---

### Task 3: Remove `TRACE_TO_LANGFUSE` guard from TypeScript plugins

**Files:**
- Modify: `plugin/code-trace.ts` (OpenCode plugin)
- Modify: `plugin/pi-agent/code-trace.ts` (Pi extension)

The Rust binary now handles the enabled check. The plugins should always invoke the binary so the binary can decide.

- [ ] **Step 1: Remove the guard from `plugin/code-trace.ts`**

Find the block at lines 77–78:

```typescript
      if (process.env.TRACE_TO_LANGFUSE?.toLowerCase() !== "true") return;
```

Delete that line entirely. The `event` handler in `plugin/code-trace.ts` should now start:

```typescript
    event: async (event: { type: string; properties?: Record<string, unknown> }) => {
      if (event.type !== "session.idle") return;

      const sessionId = event.properties?.sessionID as string | undefined;
```

- [ ] **Step 2: Remove the guard from `plugin/pi-agent/code-trace.ts`**

Find the block at line 45:

```typescript
    if (process.env.TRACE_TO_LANGFUSE?.toLowerCase() !== "true") return;
```

Delete that line entirely. The `agent_end` handler should now start:

```typescript
  pi.on("agent_end", async (event: any, ctx: any) => {
    const allEntries: any[] = ctx.sessionManager.getEntries();
```

- [ ] **Step 3: Verify the files look right**

```bash
grep -n "TRACE_TO_LANGFUSE" plugin/code-trace.ts plugin/pi-agent/code-trace.ts
```

Expected: no output (both guards removed).

- [ ] **Step 4: Commit**

```bash
git add plugin/code-trace.ts plugin/pi-agent/code-trace.ts
git commit -m "feat: plugins always invoke binary, binary handles enabled check via config"
```

---

### Task 4: `install.sh` — create config file on install

**Files:**
- Modify: `install.sh`

- [ ] **Step 1: Add config file creation to `install.sh`**

Find the block near the end of the file that prints the done message (around line 209):

```bash
echo ""
echo "Done! To enable tracing, add to your project's .claude/settings.local.json (Claude Code)"
```

Insert a new block **before** that `echo ""` line:

```bash
# Create config file if it does not already exist
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/code-trace"
CONFIG_FILE="${CONFIG_DIR}/config"

if [ -f "${CONFIG_FILE}" ]; then
  echo "Config file already exists: ${CONFIG_FILE}"
else
  mkdir -p "${CONFIG_DIR}"
  cat > "${CONFIG_FILE}" << 'EOF'
# code-trace configuration
# Set TRACE_TO_LANGFUSE=true and add your Langfuse keys to enable tracing.
TRACE_TO_LANGFUSE=false
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# CODE_TRACE_DEBUG=false
EOF
  echo "Created config file: ${CONFIG_FILE}"
fi
```

- [ ] **Step 2: Update the closing usage hint**

Replace the existing closing block:

```bash
echo ""
echo "Done! To enable tracing, add to your project's .claude/settings.local.json (Claude Code)"
echo "or set environment variables for OpenCode and Pi Agent:"
echo ""
cat << 'EOF'
{
  "env": {
    "TRACE_TO_LANGFUSE": "true",
    "LANGFUSE_PUBLIC_KEY": "pk-lf-...",
    "LANGFUSE_SECRET_KEY": "sk-lf-..."
  }
}
EOF
echo ""
echo "For OpenCode and Pi Agent extensions, set these environment variables in your shell profile."
```

With:

```bash
echo ""
echo "Done! Edit ${CONFIG_FILE} to enable tracing:"
echo "  Set TRACE_TO_LANGFUSE=true and add your LANGFUSE_PUBLIC_KEY / LANGFUSE_SECRET_KEY."
echo ""
echo "Environment variables override the config file if you need per-project overrides."
```

- [ ] **Step 3: Verify the script is valid bash**

```bash
bash -n install.sh && echo "syntax ok"
```

Expected: `syntax ok`

- [ ] **Step 4: Commit**

```bash
git add install.sh
git commit -m "feat: install.sh creates ~/.config/code-trace/config on first install"
```

---

### Task 5: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Replace the Configuration section**

Replace the entire `## Configuration` section (from `## Configuration` down to the end of `## Environment variables`) with the following:

````markdown
## Configuration

### Config file (recommended)

Add your credentials to `~/.config/code-trace/config` (created by the install script):

```
TRACE_TO_LANGFUSE=true
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# CODE_TRACE_DEBUG=false
```

This file is read by the binary at startup and works the same regardless of which agent you're using.

Respects `$XDG_CONFIG_HOME`: if set, the file is read from `$XDG_CONFIG_HOME/code-trace/config`.

### Per-agent setup

Each agent also needs the hook/plugin installed so it invokes the binary.

#### Claude Code

The install script does this automatically. If not, add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "code-trace"
          }
        ]
      }
    ]
  }
}
```

#### OpenCode

Copy `plugin/opencode/code-trace.ts` to your OpenCode plugins directory:

```bash
mkdir -p ~/.config/opencode/plugins/
cp plugin/opencode/code-trace.ts ~/.config/opencode/plugins/code-trace.ts
```

Or use the install script with `--opencode` (see above).

#### Pi

Copy `plugin/pi-agent/code-trace.ts` to your Pi extensions directory:

```bash
mkdir -p ~/.pi/agent/extensions/
cp plugin/pi-agent/code-trace.ts ~/.pi/agent/extensions/code-trace.ts
```

Or use the install script with `--pi` (see above).

### Environment variable overrides

Environment variables take precedence over the config file. This is useful for per-project credentials or CI environments.

| Variable | Required | Description |
|----------|----------|-------------|
| `TRACE_TO_LANGFUSE` | Yes | Set to `true` to enable tracing |
| `LANGFUSE_PUBLIC_KEY` | Yes | Langfuse public key |
| `LANGFUSE_SECRET_KEY` | Yes | Langfuse secret key |
| `LANGFUSE_BASE_URL` | No | Langfuse host (default: `https://cloud.langfuse.com`) |
| `CODE_TRACE_DEBUG` | No | Set to `true` for debug logging (alias: `CC_TRACE_DEBUG`) |

The `CC_LANGFUSE_` prefix is also accepted for all Langfuse variables (e.g. `CC_LANGFUSE_PUBLIC_KEY`).
````

- [ ] **Step 2: Verify the README renders correctly**

```bash
grep -n "## Configuration" README.md
grep -n "## How it works" README.md
```

Expected: one line each; the section headings are present and in order.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: restructure configuration section around config file"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass, no regressions.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 3: Verify guard removal is complete**

```bash
grep -rn "TRACE_TO_LANGFUSE" plugin/
```

Expected: no results (both plugin guards removed).

- [ ] **Step 4: Verify config keys appear in binary**

```bash
grep -n "load_config\|config_path\|XDG_CONFIG_HOME" src/main.rs src/config.rs
```

Expected: `config_path` in `src/config.rs`, `load_config` in both files.

- [ ] **Step 5: Check for any `set_var` safety issues**

`std::env::set_var` is marked unsafe in Rust 2024 edition. Verify the Rust edition in `Cargo.toml`:

```bash
grep "edition" Cargo.toml
```

If edition is `2021` or earlier, `set_var` is safe to call. If `2024`, wrap the call:

```rust
// Safety: called single-threaded at binary startup before any threads spawn
unsafe { std::env::set_var(k, v); }
```

- [ ] **Step 6: Build release binary**

```bash
cargo build --release
```

Expected: builds successfully.
