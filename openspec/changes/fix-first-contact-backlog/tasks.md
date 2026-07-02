# Tasks: fix-first-contact-backlog

## 1. Red test first

- [x] 1.1 Added `first_contact_emits_only_latest_turn` (3-turn pre-existing transcript, fresh state → only turn 3 emitted, turn_count 3, then normal emission from turn 4); restructured `langfuse_only_purge_preserves_state_and_does_not_resurrect` to build its history hook-by-hook; new test confirmed failing (`[1, 2, 3]` emitted) against the pre-fix binary

## 2. Fix

- [x] 2.1 `src/main.rs`: each arm detects cursor absence before defaulting (`first_contact`), a shared `first_contact_skip` helper skips all but the last built turn and logs the skip, `turn_count` advances past the skipped turns

## 3. Verify and document

- [x] 3.1 Full `cargo test` green (106 passed, 0 failed); clippy clean incl. `--all-targets --features harness`; all 5 Track 1 scenarios green against the fixed binary
- [x] 3.2 README: first-contact-never-uploads-history note plus the documented `TRACE_TO_LANGFUSE` disable-gap limitation, pointing at `pause` as the airtight mechanism
