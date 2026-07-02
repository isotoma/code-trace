# session-registry

## ADDED Requirements

### Requirement: State file holds a session registry alongside cursors
The persisted state SHALL have the shape `{ "cursors": <map of hash → cursor state>, "sessions": <map of session_id → SessionRecord> }`. A `SessionRecord` SHALL contain `session_id`, `source`, `transcript_path`, `cwd`, `suppressed`, `last_seen_epoch`, and `cursor_key`, where `cursor_key` equals the emit path's `state_key(source, session_id, handle)` value (handle = transcript path for Claude Code, session id for OpenCode/Pi).

#### Scenario: Record links to its cursor
- **WHEN** a session is recorded for a Claude Code payload with a given transcript path
- **THEN** the stored `cursor_key` equals `state_key("claude-code", session_id, transcript_path)` used by the emit path

### Requirement: Legacy state migrates without losing cursors
`load_state` SHALL detect a legacy flat-map state file (no top-level `cursors` key) and wrap it as `{ "cursors": <legacy map>, "sessions": {} }`, preserving every cursor entry unchanged. A file that already has a top-level `cursors` key SHALL load as the new shape directly.

#### Scenario: Legacy flat map is wrapped
- **WHEN** `load_state` reads a legacy state file containing a flat map of hash → cursor state
- **THEN** the loaded state has those exact cursor entries under `cursors` (offsets unchanged) and an empty `sessions` map

#### Scenario: New shape loads directly
- **WHEN** `load_state` reads a state file with top-level `cursors` and `sessions` keys
- **THEN** both maps load as-is with no migration applied

### Requirement: Recording a session preserves its suppression flag
`record_session` SHALL insert a new record or refresh an existing one (updating `transcript_path`, `cwd`, `cursor_key`, and `last_seen_epoch`) and SHALL NOT reset an existing `suppressed` flag.

#### Scenario: Refresh keeps suppression
- **WHEN** a session already recorded with `suppressed: true` is recorded again by a later hook invocation
- **THEN** the record's `suppressed` flag remains `true` and `last_seen_epoch` is updated

### Requirement: Age pruning exempts suppressed sessions
Registry pruning SHALL remove active (non-suppressed) entries older than 7 days and SHALL never remove suppressed entries; suppressed entries persist until explicit `resume` or `purge`.

#### Scenario: Old active entry pruned
- **WHEN** pruning runs and an entry with `suppressed: false` has `last_seen_epoch` older than 7 days
- **THEN** that entry is removed

#### Scenario: Old suppressed entry kept
- **WHEN** pruning runs and an entry with `suppressed: true` has `last_seen_epoch` older than 7 days
- **THEN** that entry is retained
