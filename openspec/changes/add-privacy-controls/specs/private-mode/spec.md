# private-mode

## ADDED Requirements

### Requirement: Suppressed sessions are never emitted
On the bare emit path, after parsing the stdin payload and before reading any transcript content or forking any HTTP send, the binary SHALL record the session in the registry and, if the session's `suppressed` flag is true, SHALL save state and exit 0 without emitting anything. This check SHALL apply identically to all input sources (Claude Code, OpenCode, Pi).

#### Scenario: Suppressed session skipped
- **WHEN** the Stop hook invokes the binary for a session whose registry entry has `suppressed: true`
- **THEN** no transcript bytes are consumed, no cursor advances, no batch is sent, and the process exits 0

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
