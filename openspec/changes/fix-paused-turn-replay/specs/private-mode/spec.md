# private-mode

## MODIFIED Requirements

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
