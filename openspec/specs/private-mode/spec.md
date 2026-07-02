# private-mode

## Purpose
Per-session tracing suppression: pause/resume and the emit-path guarantee for suppressed sessions.

## Requirements

### Requirement: Suppressed sessions are never emitted
On the bare emit path, after parsing the stdin payload, the binary SHALL record the session in the registry. If the session's `suppressed` flag is true, the binary SHALL still consume its input — reading new transcript content (Claude Code) or accepting the piped messages (OpenCode, Pi) and advancing the session's cursor (offset, buffer, turn count) — but SHALL NOT build any ingestion event or fork any send, saving state and exiting 0. Turns that occur while a session is suppressed are thereby never emitted, including after a later `resume`. This behaviour SHALL apply identically to all input sources.

#### Scenario: Suppressed session consumed but not emitted
- **WHEN** the Stop hook invokes the binary for a session whose registry entry has `suppressed: true`
- **THEN** the cursor advances past the new turns, no ingestion event is built, no batch is sent, and the process exits 0

#### Scenario: Paused-period turns never replay after resume
- **WHEN** a session emits turn 1, is paused, completes turn 2, is resumed, and completes turn 3
- **THEN** Langfuse receives turns 1 and 3 only; turn 2 is never emitted and the numbering gap is visible

#### Scenario: Unsuppressed session emits as before
- **WHEN** the binary is invoked for a session not marked suppressed
- **THEN** the existing emit behaviour proceeds unchanged


### Requirement: pause suppresses a session
`code-trace pause` SHALL set `suppressed: true` on the target session: the session named by `--session <id>`, or the most-recently-seen registry entry (highest `last_seen_epoch`) when no id is given. It SHALL print which session was paused (id and source) and SHALL fail with a clear error when the registry is empty or the id is unknown.

#### Scenario: Bare pause targets most recent
- **WHEN** `code-trace pause` runs with no arguments and the registry is non-empty
- **THEN** the entry with the highest `last_seen_epoch` becomes suppressed and its id and source are printed

#### Scenario: Explicit pause
- **WHEN** `code-trace pause --session <id>` runs with a known id
- **THEN** that session's record is marked suppressed

#### Scenario: Empty registry
- **WHEN** `code-trace pause` runs and the registry has no entries
- **THEN** the command exits non-zero with a message and no state change

### Requirement: resume un-suppresses a session
`code-trace resume [--session <id>]` SHALL clear the `suppressed` flag using the same target-selection rules as `pause` and print which session was resumed.

#### Scenario: Resume most recent
- **WHEN** `code-trace resume` runs and the most-recently-seen session is suppressed
- **THEN** its `suppressed` flag becomes false and the session id is printed

### Requirement: Suppression persists across session resume
A suppressed session SHALL remain suppressed for its lifetime, including when the agent resumes it later; only explicit `resume` or `purge` clears suppression. New sessions SHALL trace by default.

#### Scenario: Resumed private session stays private
- **WHEN** a session paused yesterday is resumed by the agent (same session id) and its Stop hook fires
- **THEN** the session is still suppressed and nothing is emitted

### Requirement: State mutations are lock-safe
`pause` and `resume` SHALL acquire the state file lock before mutating the registry and SHALL persist via the existing atomic save.

#### Scenario: Concurrent hook and pause
- **WHEN** `pause` runs while another invocation holds the state lock
- **THEN** the mutation waits for the lock and no state corruption occurs
