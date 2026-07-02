# Proposal: add-privacy-controls

## Why

Once `TRACE_TO_LANGFUSE=true` is set, code-trace traces **every** session unconditionally — there is no way to suppress, inspect, or remove tracing for a single session. Engineers sometimes work with confidential data (e.g. staff records) where nothing must be traced, and the only lever today is the global flag, which affects every concurrently-running agent and is easy to forget to re-enable.

## What Changes

- Add a small CLI surface to the binary (currently it has **no** argument parsing): `--on-start`, `status`, `sessions`, `pause`, `resume`, `purge`, `--version`, `--help`. Bare invocation (stdin piped) keeps the existing Stop-hook emit behaviour unchanged.
- **Startup reminder**: `code-trace --on-start` acts as a SessionStart handler — records the session and prints a one-line reminder that tracing is enabled (or paused for this session). Never emits.
- **Private mode (per session)**: `code-trace pause` suppresses tracing for the current session; suppression persists for the session's lifetime (including `--resume`) until explicit `resume` or `purge`. New sessions trace by default. Enforcement lives in the binary's emit path, before any transcript read or HTTP fork, so all three sources (Claude Code, OpenCode, Pi) inherit it.
- **Purge**: `code-trace purge --session <id>` removes all three copies of a session's data — Langfuse traces (list + bulk delete via API), the local transcript file, and code-trace state.
- **State schema change**: the persisted state grows a session registry (`{ cursors, sessions }`) alongside the existing cursor map, with a load-time migration that preserves cursor offsets byte-for-byte (losing them would re-emit every prior turn as duplicates). Suppressed registry entries are exempt from age pruning.
- Langfuse HTTP helpers (`GET`/`DELETE` with Basic auth) factored out for purge; today the code only POSTs.
- Version bump `0.2.1 → 0.3.0`.

Out of scope (separate PRs/repos): Claude Code `settings.json` hook wiring and `ai-plugins` skills; OpenCode/Pi startup-event hooks; redacting fields within a traced turn; scrubbing data outside code-trace's ownership (shell history, provider logs).

## Capabilities

### New Capabilities

- `session-registry`: persistent per-session metadata (id, source, transcript path, cwd, suppressed flag, last-seen, cursor key) in the state file; legacy-state migration; age pruning that exempts suppressed entries.
- `private-mode`: `pause`/`resume` commands and the suppression early-exit on the emit path — the per-session privacy guarantee.
- `startup-reminder`: `--on-start` SessionStart handler that records the session and prints the tracing-status reminder line.
- `session-purge`: `purge` command removing Langfuse traces, local transcript, and code-trace state for a session, with `--langfuse-only` / `--local-only` / `--yes` / `--transcript-path` flags.
- `cli-dispatch`: argument dispatch on the binary — subcommands route to handlers, everything else falls through to the existing stdin/emit path; `status`, `sessions`, `--version`, `--help`.

### Modified Capabilities

None — no existing specs; current emit behaviour is unchanged apart from the suppression check.

## Impact

- **Code**: `src/state.rs` (schema + migration), `src/main.rs` (dispatch + suppression early-exit), new `src/cli.rs`, new `src/langfuse.rs` (GET/DELETE helpers), `src/payload.rs` (`Input::transcript_path()` accessor), `src/source.rs` (test `Source::parse` rejects SessionStart source values), `Cargo.toml`, `README.md`.
- **APIs**: uses Langfuse public API `GET /api/public/traces?sessionId=` and `DELETE /api/public/traces` (≤1000 ids per request).
- **Data**: state file at `~/.local/share/code-trace/` changes shape; migration is load-bearing (cursor loss ⇒ duplicate re-emission).
- **Integrations**: installed `Stop` hook (`"command": "code-trace"`) keeps working unchanged; agent-side hook wiring for `--on-start` is follow-on work.
