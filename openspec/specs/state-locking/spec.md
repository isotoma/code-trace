# state-locking

## Purpose
Mutual exclusion for state-file read-modify-write cycles via a blocking advisory lock.

## Requirements

### Requirement: State access is mutually exclusive across processes
Every code-trace invocation that reads or writes the persisted state (emit path, `--on-start`, `pause`, `resume`, `purge`) SHALL hold an exclusive advisory lock on the shared lock file for the full duration of its state read-modify-write cycle. A process contending for the lock SHALL block until it is released, not proceed without it.

#### Scenario: Contending invocation waits
- **WHEN** one process holds the state lock and a second code-trace invocation starts
- **THEN** the second invocation neither reads nor writes `state.json` until the lock is released, then completes normally

#### Scenario: Pause is never lost to a concurrent emit
- **WHEN** session A's emit and a `pause` of session B execute concurrently in any interleaving
- **THEN** after both complete, session B is suppressed and B's subsequent payloads produce no ingestion sends

### Requirement: Lock acquisition failure is loud, not silent
If the `flock` call itself fails (an error return, as distinct from waiting for a contended lock), the invocation SHALL log the failure and MAY proceed without the lock as a best-effort degradation. Interrupted acquisition (EINTR) SHALL be retried.

#### Scenario: Lock error is logged
- **WHEN** acquiring the lock returns an error
- **THEN** an error is written to the code-trace log and the invocation continues rather than dropping the hook's work silently

### Requirement: Each process acquires the lock exactly once
A code-trace process SHALL acquire the state lock at most once per invocation, at its entry point, before first state access; helper functions (`load_state`, `save_state`) SHALL never acquire it. This invariant SHALL be documented on the lock type, since a second blocking acquisition on a new file descriptor would self-deadlock.

#### Scenario: No path double-acquires
- **WHEN** any subcommand or the emit path runs to completion
- **THEN** exactly one lock acquisition occurred in that process, verified by audit of `main.rs` and `cli.rs` entry points
