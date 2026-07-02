# Design: fix-state-locking

## Context

`FileLock` (src/state.rs) wraps `flock` on `state.lock`, acquired once near the top of each state-touching entry point: `run()` in `main.rs` (emit path), and `on_start` / `set_suppression` (pause/resume) / `purge` in `cli.rs`. The current call passes `LOCK_EX | LOCK_NB` and discards the return value, so contention is never detected and never waited on; `save_state` (tmp file + rename) then makes concurrent writers last-writer-wins. The lock is process-scoped RAII: `Drop` unlocks explicitly, and OS semantics release it on process death regardless. The emit path's fire-and-forget fork inherits the open lock fd, but the parent's explicit `LOCK_UN` on drop unlocks the shared open file description, so children do not prolong the lock.

## Goals / Non-Goals

**Goals:**
- Any state read-modify-write cycle is atomic with respect to every other code-trace process on the machine.
- Lock acquisition failure is loud (logged), never silent.
- The two `#[ignore]`d harness tests pass unmodified apart from removing the ignore attributes.

**Non-Goals:**
- Fixing the pause-targeting semantic race (which session "most recent" means) — orthogonal to locking.
- Recalling in-flight forked sends — impossible by design; pinned by a harness test as accepted.
- Timeouts, reader/writer distinction, or per-session lock granularity — the critical sections are milliseconds (except rare purge); simplicity wins.

## Decisions

1. **Blocking `LOCK_EX`, check the result.** Drop `LOCK_NB`; on `flock` error (not contention — actual failure, e.g. EINTR loop exhausted or ENOLCK) log via `log::error` and continue without the lock, preserving today's best-effort worst case but making it observable. Alternative: fail the invocation on lock error — rejected; a hook that refuses to run loses trace data over a lock hiccup, and degraded-but-loud matches the tool's fire-and-forget ethos. EINTR is retried.
2. **Keep the single coarse lock file.** One `state.lock` guarding all of `state.json`. Alternative: per-session locks — rejected; the state file is a single JSON document, so writes are whole-file regardless, and cursors/sessions cross-reference each other during purge.
3. **One acquisition per process, held for the process's state-touching lifetime.** With a blocking lock, re-acquiring `flock` on a *new* fd in the same process would deadlock against itself (flock is per open-file-description). Audit confirms each entry point acquires exactly once and helpers (`load_state`, `save_state`) never acquire. Document this invariant on `FileLock` — it is now load-bearing.
4. **Purge holds the lock across Langfuse HTTP (accepted).** Restructuring purge to release during network I/O would reintroduce a read-modify-write gap on `remove_session`, trading a correctness property for latency of a rare command. Concurrent hooks queue for the duration of purge's paginated GET/DELETE cycle; documented in `--help` text is unnecessary, a code comment suffices.
5. **Emit path holds the lock across transcript read and batch building.** Already the case structurally (`_lock` lives for all of `run()`); with blocking semantics this now serializes parallel Stop hooks. The work is local file I/O and JSON assembly — milliseconds. With `CODE_TRACE_SYNC_SEND=1` (tests only) the inline send also runs under the lock; acceptable in tests, does not occur in production (production forks after the parent's critical section and the parent exits, releasing the lock).
6. **Verification is the harness.** First task removes the `#[ignore]`s; the mutual-exclusion test (test holds `state.lock`, spawned `pause` must block) and the pause-vs-emit lost-update test define done. The stress test provides confidence at higher contention.

## Risks / Trade-offs

- [A hung code-trace process holding the lock stalls all hooks indefinitely (no timeout)] → critical sections contain no unbounded waits except purge's HTTP, which uses ureq's default timeouts; flock releases on process death, so a killed process cannot wedge the system.
- [Blocking behind a slow purge delays Stop hooks of live sessions] → accepted per decision 4; bounded by HTTP timeouts × page count.
- [Some filesystem/environment where flock fails consistently (e.g. odd network home dirs)] → behaviour degrades to exactly today's (no exclusion) but with error logs pointing at the cause.
- [Future code adds a second `FileLock::acquire` on a path that already holds it → self-deadlock] → invariant documented on the type; lock-scope audit in this change establishes the pattern; mutual-exclusion test would hang (and time out) in CI rather than pass silently.

## Migration Plan

Single-binary change, no state-shape or on-disk changes; ships as a patch/minor bump with `add-integration-harness`'s suite green. Rollback is reverting the commit — the lock file format is unchanged (empty file, advisory lock only).

## Open Questions

None — semantics were settled in exploration (2026-07-02): blocking lock, loud failure, purge holds lock, no timeout.
