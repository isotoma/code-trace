# session-purge

## Purpose
Removing all copies of a session's data: Langfuse traces, local transcript, and code-trace state.

## Requirements

### Requirement: purge removes all three copies of a session's data
`code-trace purge --session <id>` SHALL, by default, (1) delete the session's traces from Langfuse, (2) delete the local transcript file when `transcript_path` is set and the file exists, and (3) remove the session's registry entry and its linked cursor (`cursors[cursor_key]`) from state.

#### Scenario: Full purge
- **WHEN** `purge --session <id> --yes` runs for a registered Claude Code session with traces in Langfuse and an existing transcript file
- **THEN** the Langfuse traces for that session are deleted, the transcript file is removed, and neither `sessions[id]` nor `cursors[cursor_key]` remain in state

### Requirement: Langfuse deletion lists then bulk-deletes with chunking
The Langfuse step SHALL collect trace ids via paginated `GET {host}/api/public/traces?sessionId=<id>` and delete them via `DELETE {host}/api/public/traces` with body `{ "traceIds": [...] }` in chunks of at most 1000 ids, using the same Basic auth (`base64(public:secret)`) as ingestion, and SHALL report the number of traces deleted.

#### Scenario: More than 1000 traces
- **WHEN** the session has 2500 traces in Langfuse
- **THEN** deletion is issued in three requests (1000, 1000, 500) and the reported count is 2500

#### Scenario: Paginated listing
- **WHEN** the traces listing spans multiple pages
- **THEN** all pages are fetched and every returned trace id is included in the delete set

### Requirement: purge scope flags restrict layers
`--langfuse-only` SHALL limit the purge to the Langfuse step; `--local-only` SHALL limit it to the transcript and state steps. `--transcript-path <p>` SHALL allow purging a session absent from the registry by supplying its transcript path explicitly.

#### Scenario: Langfuse-only purge
- **WHEN** `purge --session <id> --langfuse-only --yes` runs
- **THEN** Langfuse traces are deleted but the local transcript and state entry are untouched

#### Scenario: Pre-feature session
- **WHEN** `purge --session <id> --transcript-path /path/t.jsonl --yes` runs for a session with no registry entry
- **THEN** the Langfuse traces are deleted and the given transcript file is removed

### Requirement: purge confirms before deleting
`purge` SHALL require interactive confirmation before deleting unless `--yes` is passed.

#### Scenario: Non-interactive purge
- **WHEN** `purge --session <id> --yes` runs
- **THEN** deletion proceeds without prompting

#### Scenario: Declined confirmation
- **WHEN** `purge --session <id>` runs and the confirmation is declined
- **THEN** nothing is deleted and the command exits without error
