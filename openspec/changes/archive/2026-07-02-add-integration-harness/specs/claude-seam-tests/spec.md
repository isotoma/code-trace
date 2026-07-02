# claude-seam-tests

## ADDED Requirements

### Requirement: Track 1 runs real Claude Code hermetically
The Track 1 harness SHALL run a pinned version of the real `claude` CLI in a container with code-trace installed as Stop and SessionStart hooks, `ANTHROPIC_BASE_URL` pointed at a stub Messages API returning canned responses, and `LANGFUSE_BASE_URL` pointed at the fake Langfuse. No real API key and no external network access SHALL be required.

#### Scenario: Hermetic run
- **WHEN** the Track 1 suite runs on a machine with Docker and no Anthropic or Langfuse credentials
- **THEN** all scenarios complete using only the stub model and fake Langfuse

### Requirement: Hook seam produces a trace end-to-end
Track 1 SHALL verify that completing a prompted turn in real `claude` fires the Stop hook and results in an ingestion batch at the fake Langfuse carrying the session id from claude's own payload.

#### Scenario: One turn, one trace
- **WHEN** `claude -p` completes a single scripted turn
- **THEN** the fake Langfuse receives a batch whose trace references that session id and turn

### Requirement: Config file discovery works through the real seam
Track 1 SHALL include a scenario configured only via the code-trace config file (no `LANGFUSE_*`/`TRACE_TO_LANGFUSE` variables in the hook's environment beyond what the config file supplies), proving `XDG_CONFIG_HOME` discovery works when invoked by claude.

#### Scenario: Config-file-only tracing
- **WHEN** the only tracing configuration lives in the config file
- **THEN** a completed turn still produces a trace at the fake Langfuse

### Requirement: Startup reminder appears via SessionStart
Track 1 SHALL verify that `code-trace --on-start`, wired as a SessionStart hook, emits its status line into the session context and never produces an ingestion POST.

#### Scenario: Reminder on session start
- **WHEN** a new claude session starts
- **THEN** the reminder line is present in the SessionStart hook output and the fake Langfuse records no ingestion from it

### Requirement: Pause suppresses tracing across resume through the real seam
Track 1 SHALL verify that after `code-trace pause` targets a live session, subsequent turns produce no ingestion POSTs, and that resuming the same session with `claude --resume` remains suppressed.

#### Scenario: Paused session sends nothing
- **WHEN** a session is paused and another turn completes
- **THEN** the fake Langfuse receives no new events for that session

#### Scenario: Suppression survives resume
- **WHEN** the paused session is resumed via `claude --resume` and a turn completes
- **THEN** the fake Langfuse still receives no new events for that session
