# Design: add-integration-harness

Background: exploration conversation on 2026-07-02, following completion of `add-privacy-controls`. The harness exists to prove the privacy guarantees end-to-end and to demonstrate (red) the concurrency defects fixed by `fix-state-locking`.

## Context

code-trace is invoked as short-lived processes: Claude Code Stop/SessionStart hooks, or CLI subcommands (`pause`, `resume`, `purge`). All invocations share `state.json` via load→modify→save under an flock that is currently non-blocking with its result ignored — so concurrent invocations are last-writer-wins. Sends to Langfuse are fire-and-forget: the process forks, the child `setsid()`s and POSTs, and is unwaitable. Everything the binary touches is env-routable: `XDG_DATA_HOME` (state, logs), `XDG_CONFIG_HOME` (config file), `LANGFUSE_BASE_URL`/keys (sink), `TRACE_TO_LANGFUSE` (master switch).

Test assets today: unit tests in modules, `tests/cli_test.rs` (crafted state files), `tests/integration_test.rs` (payload fixtures). Nothing spawns concurrent processes; nothing runs a real agent.

## Goals / Non-Goals

**Goals:**
- Verify the real Claude Code ↔ code-trace seam once, in a hermetic container: hook firing, payload contract, config discovery, pause persistence across `--resume`.
- Verify concurrency behaviour deterministically, without an agent: scripted interleavings, exact negative assertions, invariant-checked stress.
- Land the known-bug tests red so `fix-state-locking` has an executable definition of done.
- Track 2 runs in plain `cargo test` on every push; Track 1 is a separate CI job.

**Non-Goals:**
- Fixing the locking/purge bugs (that is `fix-state-locking`).
- OpenCode/Pi seam testing (fixture tests remain the coverage).
- Testing against real Langfuse or the real Anthropic API.
- Performance/latency benchmarking.

## Decisions

1. **Two tracks by what only each can prove.** Track 1 (real `claude`) answers "did we understand claude's contract?" — crafted payloads cannot, because they encode our assumptions. Track 2 (direct binary control) answers "is the binary correct under parallelism?" — a real agent cannot, because its timing is uncontrollable. Track 1 stays small (~5 scenarios); everything behavioural lives in Track 2.
2. **Fake Langfuse is one implementation serving both tracks.** Endpoints: `POST /api/public/ingestion` (record batch, verify Basic auth, 207), `GET /api/public/traces?sessionId=` (paginated list from recorded batches), `DELETE /api/public/traces` (remove by ids). Control plane under `/_test/`: `GET /_test/events` (ordered dump), `POST /_test/reset`, `POST /_test/hold` (latency injection — opens the purge-vs-in-flight-send window). Runs in-process for Track 2 (spawned per test on an ephemeral port) and as a container service for Track 1. Alternative considered: mock at the ureq layer — rejected, Track 1 needs a real listener and one fake serving both keeps assertions identical.
3. **Track 1 model stub, not a real key.** `ANTHROPIC_BASE_URL` points at a stub speaking just enough of the Messages API to complete a scripted turn (fixed responses, streaming if `claude` requires it). Assertions never depend on model output — only that a turn completes so hooks fire. Hermetic, free, CI-able. Alternative: real API key in CI — rejected (cost, nondeterminism, secret handling).
4. **Track 2 sequencing primitive is the lock file itself.** The orchestrating test takes `flock(state.lock)` and spawns a code-trace invocation, which must block until release. No injection points in the binary needed. Note: against today's binary this test FAILS (the binary ignores the failed non-blocking acquire and proceeds) — that failure is the executable bug report for `fix-state-locking`. Alternative: env-gated hold points (FIFO handshake) in the binary — rejected as unnecessary once the lock is real; revisit only if finer-than-lock granularity is ever needed.
5. **`CODE_TRACE_SYNC_SEND=1`** makes `send_batch_fire_and_forget` call `send_batch_blocking` inline instead of forking. Needed because the forked child is `setsid`-detached and unwaitable, making "nothing was sent" assertions sleep-and-hope. With sync send, process exit ⇒ all sends complete; negative assertions become exact. Production path untouched when unset.
6. **Known-bug tests land red, marked `#[ignore]` with a reason string** naming `fix-state-locking` (Rust has no native expected-fail). CI for this change passes; `fix-state-locking` removes the `#[ignore]`s as its first task. Alternative: land them failing — rejected, breaks CI for everyone in between.
7. **Track 2 scenario list** (deterministic unless noted):
   - lock mutual exclusion: test holds `state.lock`; spawned `pause` must block, then complete after release. *(red — expected-fail)*
   - pause-vs-emit lost update: emit for session A stalled at the lock; `pause` session B completes; A proceeds; B must still be suppressed. *(red — subsumed by the lock fix, kept as an explicit regression test)*
   - suppressed session emits nothing: pause session, pipe Stop payload, assert zero POSTs (exact, via sync send).
   - new session traces by default; resume of paused session stays paused (state-level).
   - `purge --langfuse-only` on a live session: registry entry and cursor survive, next Stop hook emits only new turns, never re-emits purged history. *(green — pins existing correct behaviour; an early exploration hypothesis that this was buggy proved wrong on reading `cli.rs`: `remove_session` only runs when `!langfuse_only`)*
   - purge-vs-in-flight send: `/_test/hold` delays the forked child's POST; purge completes; held send then lands; assert documented behaviour (trace reappears — this is accepted and documented, the test pins the window's existence).
   - stress: N concurrent emit invocations across M sessions with interleaved pause/resume, then assert invariants: every pause issued is still in effect at the end; no duplicate `(session_id, turn)` pairs at the fake Langfuse; cursor offsets monotonic. Runs bounded iterations; failures dump the event log.
8. **Layout**: fake Langfuse + payload builders + env-isolation helpers in `tests/support/` (shared module, not a workspace member — keeps `cargo test` zero-setup); Track 2 in `tests/concurrency_test.rs`; Track 1 under `harness/` (Dockerfile, compose file, stub model server, scenario runner script). The stub model server reuses the same HTTP dependency as the fake Langfuse (dev-dependency, e.g. `tiny_http` — final choice at implementation).
9. **Track 1 scenarios**: (a) one prompted turn → trace arrives with correct session id/turn; (b) config-file-only setup (no env vars) → trace arrives, proving `XDG_CONFIG_HOME` discovery; (c) `--on-start` reminder line appears in SessionStart context; (d) `pause` mid-session → subsequent turns send nothing; (e) `claude --resume` of the paused session → still nothing. Pinned `claude` CLI version in the image; upgrades are deliberate.

## Risks / Trade-offs

- [Claude Code CLI changes hook payloads or `-p` behaviour across versions] → pin the version in the Track 1 image; treat upgrades as explicit contract re-verification runs.
- [Stub model API drifts from what `claude` requires (streaming, tool-use handshake)] → keep the stub minimal and assert only on hook side-effects; if `claude` refuses the stub, fall back to recording one real exchange and replaying it.
- [Stress test flakiness in CI] → deterministic tests carry the correctness burden; stress runs bounded iterations with a fixed seed where possible and dumps the fake-Langfuse event log on failure.
- [`#[ignore]`d red tests rot if `fix-state-locking` stalls] → tasks in `fix-state-locking` explicitly start with un-ignoring them; ignore reason strings name the change.
- [Port collisions running fake Langfuse per-test] → bind port 0 and read the assigned port; never fixed ports in Track 2.
- [Track 1 requires Docker locally] → Track 1 is optional for local dev; Track 2 carries day-to-day coverage.

## Open Questions

- Exact streaming/handshake surface the stub model API must implement for current `claude -p` — discover during implementation (first Track 1 task is a spike that captures real traffic).
- Whether `claude` in the container needs auth material even with `ANTHROPIC_BASE_URL` overridden (e.g. a dummy `ANTHROPIC_API_KEY`) — resolve in the same spike.
