# Unified Config File Design

**Date:** 2026-05-11
**Status:** Approved

## Problem

code-trace currently reads configuration from environment variables, which must be set differently depending on which agent is running:

- **Claude Code** — set in `.claude/settings.local.json` under `"env":`
- **OpenCode** — exported in shell profile (`.bashrc` / `.zshrc`)
- **Pi** — exported in shell profile (`.bashrc` / `.zshrc`)

A user who switches between agents must keep multiple config locations in sync. The credential keys (`LANGFUSE_PUBLIC_KEY`, etc.) are the same across all three, but the mechanism for supplying them differs.

## Goal

A single config file that works uniformly across all three agents, so switching between Claude Code, OpenCode, and Pi requires no config change.

## Config File

**Location:** `~/.config/code-trace/config`

Respects `$XDG_CONFIG_HOME`: if set, uses `$XDG_CONFIG_HOME/code-trace/config` instead of `~/.config/code-trace/config`.

**Format:** `KEY=value` pairs

- Lines starting with `#` are comments and are ignored
- Blank lines are ignored
- Values are not quoted (no shell expansion)
- Unknown keys are ignored

**Example:**

```
# code-trace configuration
TRACE_TO_LANGFUSE=true
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# CODE_TRACE_DEBUG=true
```

**Supported keys:**

| Key | Description |
|-----|-------------|
| `TRACE_TO_LANGFUSE` | Set to `true` to enable tracing |
| `LANGFUSE_PUBLIC_KEY` | Langfuse public key |
| `LANGFUSE_SECRET_KEY` | Langfuse secret key |
| `LANGFUSE_BASE_URL` | Langfuse host (default: `https://cloud.langfuse.com`) |
| `CODE_TRACE_DEBUG` | Set to `true` for debug logging |

The existing `CC_LANGFUSE_` prefix aliases continue to be accepted for backwards compatibility (e.g. `CC_LANGFUSE_PUBLIC_KEY`).

## Precedence

Environment variables take precedence over the config file. The lookup order for each key:

1. Environment variable (e.g. `LANGFUSE_PUBLIC_KEY`)
2. Config file value
3. Default (empty / `false`)

This allows per-project or per-session overrides without editing the config file.

## Architecture

Config loading happens **only in the Rust binary**. The TypeScript plugins do not read the config file and have no awareness of Langfuse credentials.

### Rust changes

- New module `src/config.rs` with `pub fn load_config() -> HashMap<String, String>`
- `load_config()` reads `~/.config/code-trace/config` (or `$XDG_CONFIG_HOME/code-trace/config`), parses `KEY=value` lines, skips `#` comments and blank lines
- `src/main.rs` calls `load_config()` once at startup and merges values into the process environment (env vars already set take priority)
- The existing env-var reading code throughout `src/main.rs` and `src/tags.rs` is unchanged — it continues to call `std::env::var()`

### TypeScript plugin changes

Both plugins drop the `if (process.env.TRACE_TO_LANGFUSE?.toLowerCase() !== "true") return;` guard. They always invoke the binary. The binary decides whether to send based on its own config.

This makes the plugins truly dumb: their only job is to collect session entries and pipe them to the binary.

## Install script

`install.sh` creates `~/.config/code-trace/config` with placeholder values if the file does not already exist:

```
# code-trace configuration
# Set TRACE_TO_LANGFUSE=true and add your Langfuse keys to enable tracing.
TRACE_TO_LANGFUSE=false
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# CODE_TRACE_DEBUG=false
```

If the file already exists, the install script does not modify it.

## README changes

- "Configuration" section restructured: lead with the config file, note that env vars override it
- Per-agent sections (Claude Code, OpenCode, Pi) reduced to only the agent-specific setup (hook registration, plugin copy), no longer include env var instructions
- "Environment variables" table kept as reference but noted as override mechanism
- New "Config file" section added above the per-agent sections

## Testing

- Unit test for `load_config()`: parses well-formed file, skips comments and blank lines, handles missing file gracefully
- Existing integration tests are unchanged — they work via env vars which already override file values
- Manual smoke test: set credentials in config file only (no env vars), run each agent, verify traces appear in Langfuse

## What does not change

- The `CC_LANGFUSE_` prefix aliases
- `src/tags.rs`, `src/emit.rs`, `src/state.rs`, `src/turns.rs` — no changes
- The Langfuse ingestion logic
- Cursor state files
