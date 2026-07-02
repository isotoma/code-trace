# concurrency-tests

## Purpose
Track 2: deterministic concurrency and privacy-behaviour suite driving the binary directly.

## Requirements

### Requirement: Track 2 tests run isolated and deterministic in cargo test
Concurrency tests SHALL drive the code-trace binary directly (no agent, no container) with crafted payloads, each test isolated via per-test temporary directories for `XDG_DATA_HOME`/`XDG_CONFIG_HOME`/`HOME` and a per-test fake Langfuse, and SHALL use `CODE_TRACE_SYNC_SEND=1` wherever assertions depend on send completion or absence.

#### Scenario: Zero-setup execution
- **WHEN** `cargo test` runs on a clean checkout
- **THEN** Track 2 tests pass (or are `#[ignore]`d known-bug tests) without Docker, credentials, or fixed ports, and leave no state outside their tmpdirs

### Requirement: Lock mutual exclusion is verified from outside the binary
A test SHALL acquire `flock` on the binary's `state.lock` itself, spawn a state-mutating invocation (e.g. `pause`), and verify the invocation does not read or write state until the test releases the lock, completing successfully afterwards. This test is the executable specification for the blocking-lock fix and SHALL land `#[ignore]`d with a reason naming `fix-state-locking`.

#### Scenario: Invocation blocks while the test holds the lock
- **WHEN** the test holds `state.lock` and spawns `code-trace pause --session <id>`
- **THEN** the process has neither exited nor modified `state.json` while the lock is held, and after release it completes and the session is suppressed

### Requirement: A pause is never lost to a concurrent emit
A regression test SHALL interleave an emit invocation for session A with a `pause` of session B (using the lock as the sequencing point) and verify B's suppression survives A's state save, and that B's subsequent Stop payload produces zero ingestion POSTs. It SHALL land `#[ignore]`d, red against the current last-writer-wins behaviour.

#### Scenario: Pause during another session's emit
- **WHEN** session A's emit and session B's pause run concurrently in any order
- **THEN** B is suppressed afterwards and B's next emit sends nothing

### Requirement: Suppression and default-tracing behaviour verified end-to-end
Track 2 SHALL verify at the process level: a new session's Stop payload produces a trace by default; after `pause`, further payloads for that session produce zero POSTs; after `resume`, they produce traces again.

#### Scenario: Pause and resume round-trip
- **WHEN** payloads are piped before pause, during pause, and after resume
- **THEN** the fake Langfuse records traces, then nothing, then traces again

### Requirement: Purge of a live session must not resurrect purged traces
A test SHALL purge a live session with `--langfuse-only`, pipe a further Stop payload appending new turns, and verify the session's registry entry and cursor survive and previously purged turns are not re-emitted. This pins existing correct behaviour (`remove_session` is skipped for `--langfuse-only`).

#### Scenario: Turn after langfuse-only purge
- **WHEN** a session's traces are purged with `--langfuse-only` and the session then completes another turn
- **THEN** the fake Langfuse contains only the new turn, not re-emitted history

### Requirement: The purge-vs-in-flight-send window is pinned by a test
Using fake-Langfuse latency injection to hold a forked send in flight, a test SHALL run a purge inside the window and assert the documented outcome: the purge completes, the held send lands afterwards, and the trace reappears. The test pins the accepted limitation so any future change to it is deliberate.

#### Scenario: Held send lands after purge
- **WHEN** an ingestion POST is held, purge completes during the hold, and the POST then completes
- **THEN** the fake Langfuse again contains events for the purged session, and the test documents this as the accepted window

### Requirement: Stochastic stress run upholds global invariants
A bounded stress test SHALL run N concurrent emit invocations across M sessions with interleaved pause/resume operations, then assert: every pause issued with no subsequent resume is still in effect; the fake Langfuse contains no duplicate (session id, turn number) pairs; no suppressed session has events timestamped after its pause completed. On failure it SHALL dump the fake Langfuse event log.

#### Scenario: Invariants after randomized contention
- **WHEN** the stress run completes
- **THEN** all three invariants hold, and any violation output includes the ordered event log for diagnosis
