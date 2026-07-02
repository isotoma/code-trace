# Proposal: add-integration-harness

## Why

The privacy controls (`add-privacy-controls`) make hard guarantees — "a paused session sends nothing", "purge removes everything" — that are currently verified only by unit tests with crafted state files. Nothing verifies the real Claude Code ↔ code-trace seam (hook wiring, real payload shapes, config discovery), and nothing at all exercises concurrent invocations, where exploration found a real defect: the state-file `flock` is non-blocking with its result ignored, so concurrent invocations are last-writer-wins on `state.json` — a `pause` landing during another session's emit can be silently dropped, defeating the privacy guarantee. A harness must exist to demonstrate this bug (red) before `fix-state-locking` fixes it (green), and to prevent regressions after.

## What Changes

- **Fake Langfuse server**: a small HTTP server implementing the three endpoints code-trace uses (`POST /api/public/ingestion`, `GET /api/public/traces?sessionId=`, `DELETE /api/public/traces`) plus a test control plane (inspect received events, reset, inject latency to hold a send in flight). Shared by both test tracks.
- **Track 1 — integration (containerized, real `claude`)**: a container running real Claude Code headless (`claude -p`) with code-trace wired as Stop/SessionStart hooks, a stub Anthropic Messages API (canned responses — no real key, no cost, deterministic), and the fake Langfuse. Verifies the seam: hooks fire, real payloads parse, config file discovery works, traces land, `--resume` keeps a paused session paused.
- **Track 2 — concurrency (no claude, plain `cargo test`)**: tests drive the code-trace binary directly with crafted payloads and tmpdir-isolated env (`XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `LANGFUSE_BASE_URL`). Deterministic sequencing uses the state lock file itself (the test holds `flock` on `state.lock` to stall an invocation) and fake-Langfuse latency injection for fork-window scenarios. Includes a stochastic stress test (N concurrent emits + pause/resume) asserting global invariants.
- **`CODE_TRACE_SYNC_SEND=1` test hook in the binary**: skips the fire-and-forget fork and sends inline, so process exit means all sends are complete — making negative assertions ("the paused session sent *nothing*") exact rather than sleep-and-hope. One env check, inert in production.
- **Known-bug tests land red**: the lock mutual-exclusion test and the pause-vs-emit lost-update test are expected failures (`#[ignore]`d with reasons) against today's binary; `fix-state-locking` turns them green. Purge semantics tests land green — exploration initially suspected a `--langfuse-only` cursor bug, but the implementation already preserves state there; the tests pin that behaviour.

Out of scope: OpenCode/Pi plugin seams (existing fixture tests cover them); fixing the bugs the harness demonstrates (that is `fix-state-locking`); testing real Langfuse or a real Anthropic model.

## Capabilities

### New Capabilities

- `fake-langfuse`: in-repo fake Langfuse server — ingestion/list/delete endpoints with Basic-auth checking, plus test controls (event inspection, reset, latency injection).
- `sync-send-mode`: `CODE_TRACE_SYNC_SEND=1` bypasses the send fork so tests get deterministic send completion; production behaviour unchanged when unset.
- `claude-seam-tests`: containerized Track 1 suite — real `claude` + stub model API + hooks → fake Langfuse; verifies hook wiring, payload contract, config discovery, pause-across-resume.
- `concurrency-tests`: Track 2 suite — deterministic lock-sequenced scenarios (mutual exclusion, pause-vs-emit, purge semantics including the live-session `--langfuse-only` pin, purge-vs-in-flight-send) and a stochastic stress test with invariant checks (no lost suppression, no duplicate `(session, turn)` pairs).

### Modified Capabilities

None — no main specs exist yet; behaviour changes to purge/locking are specified in `fix-state-locking`.

## Impact

- **Code**: new test crate/binary for the fake Langfuse (e.g. `tests/support/` or a workspace member); new `tests/concurrency_test.rs`; `src/emit.rs` gains the `CODE_TRACE_SYNC_SEND` check; container assets (Dockerfile/compose + stub model server + scenario scripts) under `harness/` or similar.
- **CI**: Track 2 runs in normal `cargo test`; Track 1 is a separate, slower job (needs Docker + a pinned `claude` CLI version).
- **Dependencies**: a minimal HTTP server dependency for the fake Langfuse (dev-dependency only); stub model server can reuse it.
- **Sequencing**: lands before `fix-state-locking`; two tests are expected-fail until that change merges.
