# code-trace Privacy Controls

- **Date:** 2026-07-01
- **Status:** Design / ready to implement
- **Scope:** `code-trace` binary only (Claude Code path first, portable to OpenCode/Pi)
- **Target version:** `0.2.1 → 0.3.0` (minor: new features)
- **Sibling work (later, separate repo):** `ai-plugins` skills + Claude Code hook wiring — out of scope for this document

---

## 1. Background

`code-trace` is a single Rust binary invoked as an agent hook. On each invocation it reads a payload on stdin, groups messages into turns, and POSTs a Langfuse ingestion batch. Once `TRACE_TO_LANGFUSE=true` is set, **every** session is traced unconditionally. There is no CLI, no notion of "this session", and no way to suppress, inspect, or remove tracing for a single session.

Engineers sometimes work with confidential data (e.g. staff records) where nothing must be traced. Today the only lever is the global `TRACE_TO_LANGFUSE` flag, which is unsuitable because:

- it is global, so it affects every concurrently-running agent, and
- toggling it is manual and easy to forget to re-enable.

This plan adds three privacy features:

1. **Startup reminder** — when an agent starts and tracing is enabled, remind the user.
2. **Private mode (per session)** — pause tracing for the current session; it stays paused for that session's lifetime, while every *new* session traces by default.
3. **Purge** — remove an accidentally-traced session from Langfuse **and** delete its local transcript and code-trace state.

## 2. Goals & non-goals

### Goals

- Per-session suppression with zero interference between concurrently-running agents.
- A pause that takes effect *before* any transcript is read or any HTTP send is forked.
- Purge that removes all three copies of a session's data: Langfuse traces, local transcript, code-trace state.
- All new behaviour implemented in the **binary**, so every agent integration (Claude Code, OpenCode, Pi) inherits it for free, because they all pipe through the same binary.
- Everything testable in isolation from the shell, with no agent running.

### Non-goals (this phase)

- Claude Code `settings.json` hook wiring and `ai-plugins` skills (separate PR / repo).
- OpenCode / Pi startup-event hooks (later; the binary already supports them via the same CLI).
- Scrubbing data outside code-trace's ownership: shell history, OS logs, model-provider logs, etc.
- Redacting sensitive fields *within* a traced turn (the model here is all-or-nothing per session).

## 3. Locked design decisions

| Decision | Choice | Rationale |
|---|---|---|
| Pause granularity | **Per session** (keyed on `session_id`) | Parallel agents are common; a global flag would let one session flip state for another. Per-session is fully isolated. |
| "Switch back" semantics | **(A) Suppression persists for the session's lifetime** | Least surprising: a session marked private stays private, including across `--resume`. Only a genuinely *new* session traces by default. |
| Purge scope | **Langfuse + local transcript + code-trace state** | A purge must remove every copy; otherwise "purge" gives false confidence. |
| Where the logic lives | **In the binary** | Portable across agents; skills are a thin UX layer added later. |
| CLI parsing | **Hand-rolled arg dispatch** | Avoids a new dependency; the surface is small. |

### Behaviour under decision (A)

| Event | Behaviour |
|---|---|
| New session starts | Traces by default; `--on-start` prints the ENABLED reminder. |
| User runs `pause` mid-session | That session stops tracing immediately; the `Stop` hook skips it. |
| Same session resumed later (`--resume`) | **Stays private** — the suppressed registry entry persists. |
| Different, new session | Traces by default. |

Consequence: suppressed registry entries **must not be age-pruned** (see §5.1), otherwise a private session left dormant would silently re-trace on resume. No `SessionEnd` hook is required under (A).

## 4. CLI surface

`main.rs` currently has **no argument parsing** — it reads stdin unconditionally. Add a small dispatch on `args[1]`:

| Invocation | Behaviour |
|---|---|
| `code-trace` (bare, stdin piped) | **Existing Stop-hook emit path** — backward compatible, plus the suppression early-exit (§6). |
| `code-trace --on-start` | SessionStart handler; records session, prints reminder, never emits (§7). |
| `code-trace status` | Prints: configured? host? active session count; suppressed session count. |
| `code-trace sessions` | Lists registry entries: id (truncated), source, suppressed?, last-seen, transcript path. |
| `code-trace pause [--session ID]` | Suppress a session (§8). |
| `code-trace resume [--session ID]` | Un-suppress a session. |
| `code-trace purge --session ID [flags]` | Purge a session's data (§9). |
| `code-trace --version` | Print version (also fixes the existing onboarding skill's `code-trace --version`, which currently does nothing useful). |
| `code-trace --help` | Usage. |

Dispatch rule: if `args[1]` is a known subcommand or flag, run it; otherwise fall through to the existing stdin/emit behaviour. This keeps the installed `Stop` hook (`"command": "code-trace"`) working unchanged.

## 5. State schema change (foundational)

### 5.1 Problem

`state.rs` stores cursors in `GlobalState = HashMap<String, SessionState>` keyed by
`state_key(source, session_id, handle) = sha256("{source}:{session_id}:{handle}")`.
The session id and transcript path are **hashed into the key and discarded** — they are unrecoverable. Listing sessions, targeting a session by id, and recovering a transcript path for purge are all impossible without storing this metadata.

### 5.2 New shape

```jsonc
{
  "cursors": {
    "<hash>": { "offset": 0, "buffer": "", "turn_count": 3, "updated_epoch": 1751400000 }
  },
  "sessions": {
    "<session_id>": {
      "session_id": "00893aaf-...",
      "source": "claude-code",
      "transcript_path": "/home/doug/.claude/projects/<enc>/00893aaf-....jsonl",
      "cwd": "/home/doug/projects/foo",
      "suppressed": false,
      "last_seen_epoch": 1751400000,
      "cursor_key": "<hash>"          // == state_key(source, session_id, handle); links into cursors
    }
  }
}
```

- `cursor_key` is the existing `state_key(source, session_id, handle)` value (`handle` = transcript path for Claude Code, session id for OpenCode/Pi — matching the emit path), so a purge can drop the matching cursor in one step.
- Registry keyed by `session_id` (Claude Code session ids are UUIDs; collision across sources is theoretical — store `source` in the value to disambiguate if ever needed).

### 5.3 Migration (must preserve cursors)

In `load_state`, detect the legacy flat map and wrap it. Losing cursors would reset offsets to 0 and **re-emit every prior turn** as duplicates, so this is load-bearing:

- If the JSON object has a top-level `cursors` key → load the new shape directly.
- Else (legacy flat map of `<hash> → SessionState`) → migrate to `{ "cursors": <legacy>, "sessions": {} }`.

The registry starts empty and is populated as hooks fire. Existing cursor offsets are preserved byte-for-byte.

(The existing migration from `~/.claude/state/code_trace_state.json` is independent and remains in place.)

### 5.4 Pruning

Extend the existing 7-day age prune (`prune_old_sessions`) to the registry, with one rule:

- **Active** entries (non-suppressed) prune at 7 days, as today.
- **Suppressed** entries are **exempt** from age pruning. They persist until explicitly `resume`d or `purge`d.

This is required by decision (A). Optional later refinement: a long TTL (e.g. 30 days) on suppressed entries; not in v1.

### 5.5 `state.rs` additions

- New `SessionRecord` struct with the fields above.
- New top-level `State { cursors: GlobalState, sessions: HashMap<String, SessionRecord> }` (keep `SessionState` / `GlobalState` for the cursor map unchanged).
- `record_session(&mut State, source, session_id, transcript_path, cwd)` — insert or refresh; preserve an existing `suppressed` flag; set `cursor_key` and `last_seen_epoch`.
- `set_suppressed(&mut State, session_id, bool)`.
- `remove_session(&mut State, session_id)` — drops `sessions[id]` and `cursors[cursor_key]` (used by purge).
- `prune_sessions(&mut State)` — active-only, 7 days.
- `most_recent_session(&State) -> Option<&SessionRecord>` — highest `last_seen_epoch` (used by bare `pause`).
- `load_state` / `save_state` updated for the new shape + migration. `save_state` already does atomic tmp+rename under flock — reuse unchanged.

## 6. Suppression enforcement (the privacy guarantee)

In `run()`, on the bare/emit path, **after** `payload::parse_payload` and **before** `transcript::read_new_jsonl` / `send_batch_fire_and_forget`:

1. `record_session(...)` with the current `session_id`, `transcript_path`, `cwd`, `source`.
2. If `sessions[session_id].suppressed == true` → save state, `log::debug("session suppressed, skipping")`, `return 0`.
3. Otherwise proceed with the existing emit logic.

Because this sits above all three `Input` match arms, it protects Claude Code, OpenCode, and Pi identically. It runs **before** the fork in `send_batch_fire_and_forget`; once forked, a send cannot be recalled, so pausing after a turn has forked cannot stop that turn (that is what purge is for).

## 7. Feature 1 — `--on-start` (SessionStart handler)

Confirmed from the Claude Code hooks docs: `SessionStart` passes `session_id`, `transcript_path`, and `cwd` on stdin (plus `source: "startup" | "resume" | "clear" | "compact"`), and **SessionStart stdout is injected as agent context** — which is exactly the reminder channel.

`payload.rs` already handles this: `Source::parse("startup")` returns `None`, so `parse_payload` falls through to `parse_claude_code_payload` and extracts the fields correctly. (Verify `Source::parse` rejects unknown strings; add a test.)

`--on-start` flow:

1. Read stdin, `parse_payload` → `source`, `session_id`, `transcript_path`, `cwd`.
2. `record_session(...)`; `prune_sessions`.
3. Print **one line** to stdout:
   - not configured (no keys, or `TRACE_TO_LANGFUSE != true`) → print nothing;
   - active → `⚠️ code-trace: tracing ENABLED → <host>. Use the pause command to make this session private.`;
   - this session suppressed → `code-trace: tracing PAUSED for this session (private mode).`
4. `return 0`. **Never reads the transcript, never emits.**

> Note: the exact reminder wording/agent-command name is finalised in the `ai-plugins` skill work. The binary prints a stable, human-readable line.

## 8. Feature 2 — `pause` / `resume`

- `code-trace pause` (no arg): suppress the **most-recently-seen** session (`most_recent_session`). This is reliable because the pause command is issued inside the current session, after at least one `Stop`/`SessionStart` hook has recorded it. **Print which session was paused** (id + source) so the caller can confirm it targeted the right one — important for parallel sessions.
- `code-trace pause --session <id>`: explicit.
- `code-trace resume [--session <id>]`: clear suppression (default: most-recent).
- Both acquire the file lock, mutate the registry, save.
- `code-trace sessions` lists the registry (most-recent first) so an operator or skill can pick an explicit id when there is any doubt about which session is "current".

Under (A), suppression persists until explicit `resume`/`purge`; a new session always traces by default.

## 9. Feature 3 — `purge` (Langfuse + local transcript + state)

`code-trace purge --session <id>` performs all three steps:

1. **Langfuse** — read keys from config (same Basic auth as ingestion: `base64(public:secret)`):
   - `GET {host}/api/public/traces?sessionId=<id>` — paginate (response is `{ data: [ { id, ... } ], meta: { ... } }`) and collect trace ids;
   - `DELETE {host}/api/public/traces` with body `{ "traceIds": [...] }` in chunks of **≤ 1000** (API limit; cascades to the generations/spans beneath each trace).
   - Report count deleted.
2. **Local transcript** — if `transcript_path` is set and the file exists, remove it. (Claude Code only; OpenCode/Pi have no local transcript.)
3. **State** — `remove_session(...)` drops `sessions[id]` and `cursors[cursor_key]`; save.

Flags:

- `--langfuse-only`, `--local-only` — restrict to one layer.
- `--yes` — non-interactive confirmation (the skill confirms in chat, then calls with `--yes`).
- `--transcript-path <p>` — purge a session **not** in the registry (e.g. traced before this feature shipped), supplying the transcript path explicitly.

Default (no flags): all three layers. Confirm before deleting unless `--yes`.

Caveats to surface in the skill layer (not the binary):

- Deleting the transcript removes the session from Claude Code's `--resume`/history — destructive, so the skill must confirm.
- Purge cannot un-send a turn that already forked before a pause; pause-early is the primary defence, purge is remediation.
- Anything outside code-trace's ownership (shell history, etc.) is out of scope.

## 10. Supporting code changes

- **Langfuse HTTP helpers** — the codebase only POSTs today (`emit.rs`). Factor the ureq + Basic-auth pattern into reusable `get_json(host, keys, path_and_query)` and `delete_json(host, keys, path, body)` (in `emit.rs` or a new `src/langfuse.rs`). Reuse for purge's list + bulk-delete.
- **`Input::transcript_path()` accessor** — does not exist (`main.rs` matches `Input::ClaudeCode { transcript_path, .. }` directly). Add a method so `--on-start` and `purge` can read it cleanly.
- **`Source::parse`** — confirm it returns `None` for the SessionStart `source` values (`startup`, `resume`, …); add a unit test.

## 11. File-by-file change list

| File | Change |
|---|---|
| `src/state.rs` | New `SessionRecord`, `State` container; `record_session`, `set_suppressed`, `remove_session`, `prune_sessions` (active-only), `most_recent_session`; `load_state` migration; keep `save_state`. |
| `src/main.rs` | Arg dispatch; wire `record_session` + suppression early-exit into `run()`; route subcommands to `cli`. |
| `src/cli.rs` (new) | Handlers: `on_start`, `status`, `sessions`, `pause`, `resume`, `purge`, `version`, `help`. |
| `src/langfuse.rs` (new) *or* `src/emit.rs` | `get_json`, `delete_json` helpers; `list_trace_ids(host, keys, session_id)`; `bulk_delete_traces(host, keys, ids)`. |
| `src/payload.rs` | Add `Input::transcript_path()` accessor; test SessionStart `source:"startup"` parsing. |
| `src/source.rs` | Confirm/test `Source::parse` rejects `startup`/`resume`/etc. |
| `Cargo.toml` | `0.2.1 → 0.3.0`. |
| `README.md` | Document the new CLI (pause/resume/status/sessions/purge, `--on-start`), the privacy model, and the state schema. |

## 12. Testing strategy (all shell-drivable, no agent required)

- **Migration**: a legacy flat-map `state.json` loads to the new shape with cursors byte-identical (no offset reset → no re-emit).
- **Registry**: `record_session` refresh preserves an existing `suppressed` flag; `cursor_key` equals the emit path's key; `prune_sessions` removes active entries >7d but keeps suppressed entries.
- **pause/resume**: mutates registry; no-arg targets highest `last_seen_epoch`; errors cleanly on an empty registry.
- **Suppression**: the bare emit path with a suppressed session advances no cursor and sends nothing (assert against a stubbed/mocked sender).
- **`--on-start`**: `echo '{"hook_event_name":"SessionStart","source":"startup","session_id":"x","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}' | code-trace --on-start` records the session, prints the correct line, and does not emit.
- **purge**: pure-function tests for trace-id pagination and ≤1000 chunking; an integration test against a local mock HTTP server for the list+delete round-trip (extend whatever pattern `tests/integration_test.rs` already uses).
- **Arg dispatch**: bare `code-trace` still emits; `code-trace --version` prints the version.

## 13. Implementation sequence

1. **State schema + migration** (test first; everything depends on it).
2. **`record_session` + suppression early-exit** wired into the bare emit path.
3. **`pause` / `resume` / `sessions` / `status`** (pure CLI; test with crafted `state.json`).
4. **`--on-start`** (test by piping SessionStart JSON on stdin).
5. **Langfuse GET/DELETE helpers + `purge`** (mock server, then a real test Langfuse project).
6. **`--version` / `--help`, README, version bump.**

Each step is independently testable from the shell.

## 14. Follow-on work (separate PRs / repos)

- **`ai-plugins` (ai-onboarding plugin):** add a `SessionStart` hook entry (calling `code-trace --on-start`) alongside the existing `Stop` hook; add skills for pause/resume, purge, and status; bump the plugin version in both `plugin.json` and `marketplace.json`.
- **OpenCode / Pi:** add a startup-event hook in each TS plugin/extension that calls `code-trace --on-start`. No new privacy logic is needed — the binary already enforces it.

## 15. Open items / risks

- **Concurrency on `pause` default target.** "Most-recent session" is reliable in the common case (the command runs inside the current session) but technically racy if another session's hook fires between the last record and the pause. Mitigation: always print the targeted session; provide `--session <id>` and `sessions` for explicit targeting. (Deferred: read the current session id directly if Claude Code exposes it as an env var — to verify during the `ai-plugins` phase.)
- **Session id collisions across sources** in the registry — theoretical; mitigated by storing `source` in the record. Consider keying by `source:session_id` if it ever bites.
- **Purge of pre-feature sessions** not in the registry — handled via `--transcript-path`.
- **`Source::parse` and SessionStart `source` field collision** — the SessionStart payload's `source` ("startup"…) must not be misread as an agent source. Confirmed by code path; locked with a test.
