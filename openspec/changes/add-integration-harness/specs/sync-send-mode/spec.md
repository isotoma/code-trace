# sync-send-mode

## ADDED Requirements

### Requirement: Sync send mode bypasses the send fork
When the environment variable `CODE_TRACE_SYNC_SEND` is set to `1`, the emit path SHALL perform the Langfuse send inline (blocking) in the invoking process instead of forking a detached child, so that process exit guarantees every send has completed.

#### Scenario: Exit implies delivery
- **WHEN** `CODE_TRACE_SYNC_SEND=1` and a Stop payload producing one turn is piped to code-trace
- **THEN** by the time the process exits, the ingestion POST has been received by the Langfuse endpoint

#### Scenario: Exact negative assertion
- **WHEN** `CODE_TRACE_SYNC_SEND=1` and a Stop payload for a suppressed session is piped to code-trace
- **THEN** after the process exits, the test can assert that zero ingestion POSTs occurred, with no wait or polling

### Requirement: Production fork behaviour is unchanged when unset
When `CODE_TRACE_SYNC_SEND` is unset or has any value other than `1`, the emit path SHALL retain the existing fire-and-forget fork behaviour.

#### Scenario: Default path forks
- **WHEN** the variable is unset and a batch is sent
- **THEN** the parent process returns without waiting for the HTTP request, as today
