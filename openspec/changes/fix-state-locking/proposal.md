# Proposal: fix-state-locking

## Why

`state::FileLock::acquire` calls `flock(LOCK_EX | LOCK_NB)` and ignores the result — the lock excludes nobody. Every code-trace invocation (Stop hooks of parallel agents, `pause`, `resume`, `purge`, `--on-start`) does load→modify→save on the shared `state.json`, so concurrent invocations are last-writer-wins. Concretely: a `pause` landing while another session's emit is mid-flight can be silently overwritten, and the "paused" session keeps tracing confidential data — the exact guarantee `add-privacy-controls` exists to provide. Cursor updates can be lost the same way, causing duplicate trace re-emission. The `add-integration-harness` change lands `#[ignore]`d red tests demonstrating both; this change turns them green.

## What Changes

- **Blocking lock**: `FileLock::acquire` uses blocking `LOCK_EX` (drop `LOCK_NB`) and checks the return value. Invocations queue instead of racing. Failure to acquire (error return) is logged and proceeds best-effort — degraded, but no worse than today, and never silent.
- **Lock-scope audit** across `main.rs` and `cli.rs`: exactly one `FileLock` acquisition per process lifetime on every path that touches state (a second blocking acquire in the same process would self-deadlock); lock ordering documented in `state.rs`.
- **Accepted**: `purge` holds the lock across its Langfuse HTTP calls, so concurrent hooks queue behind a purge. Purge is rare and hook work is quick; documented rather than engineered around.
- **Un-ignore** the two red harness tests (lock mutual exclusion, pause-vs-emit lost update) — they are this change's acceptance tests.

Out of scope: the pause-targeting semantic race (bare `pause` picking the most-recently-seen session — lock cannot fix intent); the purge-vs-in-flight-send window (fork escapes any lock; documented and pinned by a harness test); finer-grained locking or lock-free state.

## Capabilities

### New Capabilities

- `state-locking`: mutual exclusion for all state-file read-modify-write cycles via a blocking advisory lock — acquisition semantics, failure handling, and single-acquisition-per-process rule.

### Modified Capabilities

None — purge/pause/emit behaviour is unchanged; they just become atomic with respect to each other. (No main specs exist yet to modify; `add-privacy-controls` specs are unaffected.)

## Impact

- **Code**: `src/state.rs` (`FileLock::acquire`), audit-only touches in `src/main.rs` / `src/cli.rs` if any path double-acquires; comment/doc updates.
- **Tests**: removes `#[ignore]` from two tests in `tests/concurrency_test.rs` (from `add-integration-harness`); stress test gains teeth.
- **Behaviour**: parallel hook invocations now serialize on state access; worst case a hook waits for another hook's local work (fast) or a rare purge's network round-trips (accepted).
- **Sequencing**: depends on `add-integration-harness` landing first (provides the red tests and the harness to verify with).
