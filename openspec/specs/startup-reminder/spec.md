# startup-reminder

## Purpose
The --on-start SessionStart handler: records the session and prints the tracing-status reminder line.

## Requirements

### Requirement: --on-start records the session and never emits
`code-trace --on-start` SHALL read a SessionStart payload from stdin, extract `session_id`, `transcript_path`, and `cwd`, record the session in the registry, run registry pruning, and exit 0. It SHALL NOT read the transcript and SHALL NOT send anything to Langfuse.

#### Scenario: SessionStart payload recorded
- **WHEN** `{"hook_event_name":"SessionStart","source":"startup","session_id":"x","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}` is piped to `code-trace --on-start`
- **THEN** the registry contains a record for session `x` with that transcript path and cwd, and no ingestion request is made

### Requirement: --on-start prints a one-line tracing status
`--on-start` SHALL print exactly one line to stdout reflecting tracing status: nothing when tracing is not configured (missing keys or `TRACE_TO_LANGFUSE != true`); an ENABLED reminder naming the Langfuse host and how to pause when tracing is active; a PAUSED notice when this session is suppressed.

#### Scenario: Tracing active
- **WHEN** `--on-start` runs with tracing configured and the session not suppressed
- **THEN** stdout is a single line stating tracing is ENABLED, the target host, and that the session can be paused

#### Scenario: Session suppressed
- **WHEN** `--on-start` runs for a session whose registry entry is suppressed
- **THEN** stdout is a single line stating tracing is PAUSED for this session

#### Scenario: Not configured
- **WHEN** `--on-start` runs with `TRACE_TO_LANGFUSE` unset or keys missing
- **THEN** nothing is printed and the process exits 0

### Requirement: SessionStart source values are not agent sources
`Source::parse` SHALL return `None` for SessionStart `source` field values (`startup`, `resume`, `clear`, `compact`) and any other unknown string, so SessionStart payloads fall through to Claude Code payload parsing.

#### Scenario: startup is not a Source
- **WHEN** `Source::parse("startup")` is called
- **THEN** it returns `None`
