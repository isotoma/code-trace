# Tasks: add-privacy-controls

## 1. State schema + migration (foundational — everything depends on it)

- [x] 1.1 Write failing tests in `src/state.rs`: legacy flat-map JSON loads to `{ cursors: <identical map>, sessions: {} }` (offsets byte-identical); new-shape JSON loads directly
- [x] 1.2 Add `SessionRecord` struct (`session_id`, `source`, `transcript_path`, `cwd`, `suppressed`, `last_seen_epoch`, `cursor_key`) and top-level `State { cursors: GlobalState, sessions: HashMap<String, SessionRecord> }`; update `load_state`/`save_state` with the migration; keep atomic tmp+rename under flock unchanged
- [x] 1.3 Add registry ops with tests: `record_session` (insert/refresh, preserves existing `suppressed`, sets `cursor_key` equal to the emit path's `state_key` and `last_seen_epoch`), `set_suppressed`, `remove_session` (drops `sessions[id]` and `cursors[cursor_key]`), `most_recent_session` (highest `last_seen_epoch`)
- [x] 1.4 Extend pruning to the registry with tests: active entries prune at 7 days, suppressed entries are never age-pruned

## 2. Suppression enforcement on the emit path

- [x] 2.1 Add `Input::transcript_path()` accessor in `src/payload.rs`
- [x] 2.2 In `run()` after `parse_payload` and before any transcript read / send fork: `record_session(...)`; if suppressed → save state, `log::debug`, return 0 (covers all three `Input` arms)
- [x] 2.3 Test: emit path with a suppressed session advances no cursor and sends nothing (stub/mock the sender); unsuppressed session emits as before

## 3. CLI dispatch + pause/resume/sessions/status

- [x] 3.1 Add arg dispatch in `src/main.rs`: known subcommands/flags route to new `src/cli.rs`; anything else falls through to the existing stdin/emit path (test: bare invocation still emits)
- [x] 3.2 Implement `pause`/`resume` in `src/cli.rs`: `--session <id>` or default to most-recent; print targeted session (id + source); error cleanly on empty registry or unknown id; mutate under the file lock (tests with crafted `state.json`)
- [x] 3.3 Implement `sessions` (registry list, most-recent first: truncated id, source, suppressed, last-seen, transcript path) and `status` (configured?, host, active count, suppressed count)
- [x] 3.4 Implement `--version` (crate version) and `--help` (usage covering all subcommands)

## 4. --on-start (SessionStart handler)

- [x] 4.1 Unit test: `Source::parse` returns `None` for `startup`, `resume`, `clear`, `compact` (SessionStart payload falls through to Claude Code parsing)
- [x] 4.2 Implement `--on-start`: read stdin, `parse_payload`, `record_session`, `prune_sessions`; print one status line (nothing if unconfigured / ENABLED reminder with host / PAUSED for suppressed session); never read transcript, never emit
- [x] 4.3 Integration test: pipe a SessionStart JSON payload to `code-trace --on-start`; assert the registry record, the printed line, and that no ingestion request happens

## 5. Langfuse helpers + purge

- [x] 5.1 Create `src/langfuse.rs`: factor ureq + Basic-auth into `get_json` / `delete_json`; build `list_trace_ids` (paginated `GET /api/public/traces?sessionId=`) and `bulk_delete_traces` (`DELETE /api/public/traces`, ≤1000 ids per chunk)
- [x] 5.2 Pure-function tests for pagination collection and 1000-id chunking (e.g. 2500 ids → 1000/1000/500)
- [x] 5.3 Implement `purge --session <id>` with `--langfuse-only`, `--local-only`, `--yes`, `--transcript-path`: Langfuse delete + transcript removal + `remove_session`; confirm before deleting unless `--yes`; report traces deleted
- [x] 5.4 Integration test against a local mock HTTP server for the list+delete round-trip (extend the `tests/integration_test.rs` pattern); verify state and transcript cleanup

## 6. Finish

- [x] 6.1 Bump `Cargo.toml` to 0.3.0
- [x] 6.2 Update `README.md`: new CLI, privacy model (per-session pause semantics, purge caveats), state schema
- [x] 6.3 Full `cargo test` + manual shell smoke test of each subcommand
