# Tasks: fix-state-locking

## 1. Red tests first

- [x] 1.1 Removed `#[ignore]` from all three red tests in `tests/concurrency_test.rs` (lock mutual-exclusion, pause-vs-emit lost-update, concurrency stress); confirmed all three failed against the pre-fix binary

## 2. Fix the lock

- [x] 2.1 `FileLock::acquire` now blocks (`LOCK_EX`, no `LOCK_NB`), checks the return value, retries on EINTR, and logs + proceeds best-effort on genuine failure (open error or non-EINTR flock error)
- [x] 2.2 Documented on `FileLock`: single-acquisition-per-process invariant (blocking re-acquire on a new fd self-deadlocks) and the accepted purge-holds-lock-across-HTTP trade-off

## 3. Lock-scope audit

- [x] 3.1 Audited: exactly one acquisition per mutating entry point (`run()`; `on_start`, `set_suppression`, `purge`), helpers never acquire. Two deliberate unlocked spots noted: `status`/`sessions` are read-only (safe — `save_state` is atomic tmp+rename) and `load_state`'s one-time legacy migration write on those paths is benign (identical bytes from the same legacy source). No code changes needed

## 4. Verify

- [x] 4.1 All three previously-red tests pass; full `cargo test` green (105 passed, 0 failed, 0 ignored), clippy clean incl. `--all-targets --features harness`
- [x] 4.2 Track 1 scenario suite green against the fixed binary (all 5, run locally — Docker unavailable in this environment); version bumped 0.3.0 → 0.3.1; fix noted in README's state section
