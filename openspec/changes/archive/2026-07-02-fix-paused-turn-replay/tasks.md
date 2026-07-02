# Tasks: fix-paused-turn-replay

## 1. Red tests first

- [x] 1.1 Flipped `pause_and_resume_round_trip` to assert `[1, 3]`; inverted the cli test (renamed `suppressed_session_emits_nothing_but_consumes_turns`, asserts cursor advances + turn counted while nothing is sent); confirmed both failed against the pre-fix binary

## 2. Fix

- [x] 2.1 `src/main.rs`: suppression exit moved from before the source match into each arm via a shared `consume_suppressed` helper — cursor (offset/buffer/turn_count) advances past the paused turns, state saved, exit 0 before any `build_ingestion_batch`, for all three sources

## 3. Verify and document

- [x] 3.1 Both flipped tests pass; full `cargo test` green (105 passed, 0 failed); clippy clean incl. `--all-targets --features harness`
- [x] 3.2 Track 1 scenarios green against the fixed binary (all 5, run locally); README private-mode section now states paused turns are never traced (not deferred) with the visible numbering gap
