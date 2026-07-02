# Tasks: add-integration-harness

## 1. Shared test support

- [x] 1.1 Create `tests/support/mod.rs` with env-isolation helpers (per-test tmpdir wiring for `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, Langfuse env vars) and crafted-payload builders for Claude Code Stop/SessionStart JSON — no new dependency needed: std `TcpListener` per the existing `cli_test.rs` pattern
- [x] 1.2 Implement fake Langfuse in `tests/support/fake_langfuse.rs`: `POST /api/public/ingestion` (record + Basic-auth check + 207), backed by an ordered in-memory event log; bind port 0 and expose the assigned address
- [x] 1.3 Add `GET /api/public/traces?sessionId=` (paginated, derived from recorded batches) and `DELETE /api/public/traces` (`{"traceIds": []}`) to the fake
- [x] 1.4 Add control plane: `GET /_test/events`, `POST /_test/reset`, `POST /_test/hold?ms=N` (latency injection on subsequent ingestion POSTs)
- [x] 1.5 Unit-test the fake itself: auth rejection, purge round-trip, hold semantics, parallel-instance isolation (`tests/fake_langfuse_test.rs`, 7 tests)

## 2. Sync send mode

- [x] 2.1 Add `CODE_TRACE_SYNC_SEND=1` check in `emit::send_batch_fire_and_forget` — inline blocking send when set, existing fork path otherwise
- [x] 2.2 Test: with sync send, process exit implies batch received; without it, parent returns before delivery (existing behaviour untouched) — `tests/sync_send_test.rs`

## 3. Track 2 — deterministic concurrency suite (`tests/concurrency_test.rs`)

- [x] 3.1 Behaviour round-trip: new session traces by default; after `pause` further payloads produce zero POSTs (exact, sync send); after `resume` traces flow again. NOTE: pinned current behaviour — the cursor does not advance while suppressed, so paused-period turns are emitted after `resume` (design question flagged for the privacy feature)
- [x] 3.2 Lock mutual-exclusion test: test acquires `flock` on `state.lock`, spawns `pause`, asserts no state read/write until release, completion after; landed `#[ignore = "red until fix-state-locking"]`, verified failing against current binary
- [x] 3.3 Pause-vs-emit lost-update regression test (large-transcript emit window for session A, pause session B inside it); landed `#[ignore = "red until fix-state-locking"]`, verified failing against current binary
- [x] 3.4 `purge --langfuse-only` live-session test: registry/cursor/transcript survive, purged history not re-POSTed (green pin); full-purge test deletes traces (251, exercising 3-page listing), transcript, and state
- [x] 3.5 Purge-vs-in-flight-send window test using hold: purge inside the window, held send lands after, trace reappears — asserted as the accepted limitation
- [x] 3.6 Stress test: 4 sessions × 5 rounds of concurrent emits with interleaved pause/resume; asserts no lost suppression, no duplicate (session, turn) pairs, no post-pause events; dumps event log on failure. DEVIATION: also landed `#[ignore]` — it exercises the same lost-update bug as 3.3 and would flake red in CI until `fix-state-locking`

## 4. Track 1 — containerized claude seam (`harness/`)

- [x] 4.1 Spike done against `claude` 2.1.198: only `POST /v1/messages?beta=true` needed (route must strip query string), `x-api-key` dummy auth, SSE + non-streaming fallback shapes; findings in `harness/NOTES.md`
- [x] 4.2 Stub Messages API at `harness/stub-model/server.py` (Python stdlib — DEVIATION from "reuse the HTTP dep": zero-build, derived directly from the validated spike recorder); fake Langfuse promoted to a standalone `fake-langfuse` bin (feature-gated `harness`, same implementation as Track 2)
- [x] 4.3 `harness/Dockerfile` (multi-stage: rust build → node runtime with pinned claude 2.1.198) + `harness/docker-compose.yml` (fake-langfuse / stub-model / runner services); hook wiring written by the runner per scenario mode
- [x] 4.4 `harness/run-scenarios.sh` with all five scenarios — verified end-to-end LOCALLY (real claude + stub + fake + hooks: all 5 pass); Docker itself unavailable in this environment, so the compose stack is unexercised — first CI run must confirm
- [x] 4.5 `harness` CI job added to `.github/workflows/ci.yml` (compose up, exit-code-from runner); `harness/README.md` documents Docker and no-Docker invocation

## 5. Wrap-up

- [x] 5.1 README gains a Testing section (two tracks, sync-send rationale); CLAUDE.md testing section expanded
- [x] 5.2 Verification: `cargo test` green (102 passed, 0 failed, 3 `#[ignore]`d with reasons), clippy clean incl. `--all-targets --features harness`; Track 1 suite green when run locally (all 5 scenarios) — in-container run pending first CI execution (no Docker in this environment)
