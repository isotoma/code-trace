# Design: add-privacy-controls

Full background and rationale: `docs/2026-07-01-code-trace-privacy-controls.md`. This document condenses the locked decisions and the implementation approach.

## Context

code-trace is a single Rust binary invoked as an agent hook (Claude Code Stop hook; OpenCode/Pi via TS plugins). It reads a payload on stdin, groups messages into turns, and POSTs a Langfuse ingestion batch. `main.rs` currently has **no argument parsing**. State (`state.rs`) is a flat `HashMap<String, SessionState>` keyed by `sha256("{source}:{session_id}:{handle}")` — session id and transcript path are hashed into the key and unrecoverable, so listing sessions, targeting one by id, or finding a transcript path for purge is impossible today.

## Goals / Non-Goals

**Goals:**
- Per-session tracing suppression, fully isolated between concurrently-running agents.
- Suppression takes effect before any transcript read or HTTP fork.
- Purge removes all three copies: Langfuse traces, local transcript, code-trace state.
- All logic in the binary so every agent integration inherits it; everything testable from the shell without an agent.

**Non-Goals (this phase):**
- Claude Code hook wiring / `ai-plugins` skills (separate repo).
- OpenCode/Pi startup-event hooks (binary already supports them).
- Field-level redaction within a traced turn; scrubbing data outside code-trace's ownership.

## Decisions

1. **Pause granularity: per session**, keyed on `session_id`. A global flag would let one session flip state for another; parallel agents are common.
2. **Suppression persists for the session's lifetime** (decision A), including across `--resume`. Only a genuinely new session traces by default. Consequence: suppressed registry entries must **never be age-pruned**, else a dormant private session would silently re-trace on resume. No SessionEnd hook needed.
3. **State schema: wrap, don't replace.** New top-level shape `{ "cursors": <existing flat map>, "sessions": { "<session_id>": SessionRecord } }`. `SessionRecord` = `{ session_id, source, transcript_path, cwd, suppressed, last_seen_epoch, cursor_key }`, where `cursor_key` equals the emit path's `state_key(source, session_id, handle)` (handle = transcript path for Claude Code, session id for OpenCode/Pi), letting purge drop the matching cursor in one step.
4. **Migration is load-bearing.** In `load_state`: top-level `cursors` key present → new shape; else legacy flat map → wrap as `{ cursors: <legacy>, sessions: {} }`. Cursor offsets preserved byte-for-byte — losing them resets offsets to 0 and re-emits every prior turn as duplicates. The existing `~/.claude/state/` → `~/.local/share/code-trace/` migration stays independent. `save_state`'s atomic tmp+rename under flock is reused unchanged.
5. **Pruning**: extend the 7-day prune to registry entries, but only **active** (non-suppressed) ones. Suppressed entries persist until explicit `resume`/`purge`.
6. **Suppression enforcement point**: in `run()` on the bare emit path, after `payload::parse_payload` and before `transcript::read_new_jsonl` / `send_batch_fire_and_forget`: `record_session(...)`; if suppressed → save state, debug-log, exit 0. Sits above all three `Input` match arms so Claude Code, OpenCode, and Pi are protected identically, and runs before the send fork (a forked send cannot be recalled — that's what purge is for).
7. **CLI parsing: hand-rolled dispatch** on `args[1]` (no new dependency; the surface is small). Known subcommand/flag → handler in new `src/cli.rs`; otherwise fall through to the existing stdin/emit path, keeping the installed `Stop` hook (`"command": "code-trace"`) working unchanged.
8. **`pause`/`resume` default target: most-recently-seen session** (highest `last_seen_epoch`). Reliable because the command is issued inside the current session after at least one hook has recorded it. Always print the targeted session (id + source); `--session <id>` and `sessions` exist for explicit targeting.
9. **`--on-start`**: reads the SessionStart payload from stdin (Claude Code sends `session_id`, `transcript_path`, `cwd`, and `source: "startup"|"resume"|"clear"|"compact"`; SessionStart stdout is injected as agent context — the reminder channel). Records session, prunes, prints one status line, never emits. `payload.rs` already routes this correctly because `Source::parse("startup")` returns `None` → falls through to `parse_claude_code_payload`; lock with a test.
10. **Purge**: (1) Langfuse — `GET {host}/api/public/traces?sessionId=<id>` paginated to collect trace ids, then `DELETE {host}/api/public/traces` with `{ "traceIds": [...] }` in chunks of ≤1000 (API limit; cascades to child observations), same Basic auth as ingestion; (2) local transcript — delete file if `transcript_path` set and exists (Claude Code only); (3) state — `remove_session` drops `sessions[id]` and `cursors[cursor_key]`. Flags: `--langfuse-only`, `--local-only`, `--yes` (non-interactive), `--transcript-path <p>` (purge a pre-feature session not in the registry). Default: all three layers, confirm unless `--yes`.
11. **Langfuse HTTP helpers**: factor the ureq + Basic-auth pattern (currently POST-only in `emit.rs`) into `get_json` / `delete_json` in a new `src/langfuse.rs`, plus `list_trace_ids` and `bulk_delete_traces` on top.
12. **`Input::transcript_path()` accessor** added to `payload.rs` so `--on-start` and purge read it cleanly instead of matching variants inline.

## Risks / Trade-offs

- [Migration bug loses cursors → mass duplicate re-emission] → migration is step 1, test-first: legacy fixture must load with byte-identical cursor map.
- [Bare `pause` races another session's hook firing between record and pause] → always print the targeted session; provide `--session` and `sessions` for explicit targeting. (Deferred: read session id from an env var if Claude Code exposes one — verify during `ai-plugins` phase.)
- [Pause after a turn's send already forked cannot recall it] → documented; pause-early is the primary defence, purge is remediation.
- [Session id collision across sources in the registry] → theoretical (Claude Code ids are UUIDs); `source` stored in the record disambiguates; key by `source:session_id` if it ever bites.
- [SessionStart `source` field ("startup"…) misread as an agent source] → confirmed `Source::parse` returns `None` for unknown strings; locked with a unit test.
- [Purge gives false confidence] → skill layer must surface caveats: transcript deletion kills Claude Code `--resume` history; already-forked sends aren't recalled; shell history etc. out of scope.

## Migration Plan

1. State schema + migration (test first; everything depends on it).
2. `record_session` + suppression early-exit on the bare emit path.
3. `pause` / `resume` / `sessions` / `status` (pure CLI; test with crafted `state.json`).
4. `--on-start` (test by piping SessionStart JSON).
5. Langfuse GET/DELETE helpers + `purge` (mock HTTP server, then a real test Langfuse project).
6. `--version` / `--help`, README, bump `0.2.1 → 0.3.0`.

Rollback: the new shape is additive; a rollback binary reading the old flat shape would see the wrapped object and fail to parse — acceptable because state loss only causes duplicate traces, not data loss, and the feature ships as a minor version.

## Open Questions

- Exact reminder wording / agent-command name — finalised in the `ai-plugins` skill work; the binary prints a stable human-readable line.
- Whether Claude Code exposes the current session id as an env var (would remove the `pause` race) — verify during the `ai-plugins` phase.
- Optional long TTL (e.g. 30 days) on suppressed entries — explicitly not in v1.
