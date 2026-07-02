# fake-langfuse

## ADDED Requirements

### Requirement: Fake Langfuse implements the ingestion endpoint
The fake Langfuse server SHALL accept `POST /api/public/ingestion`, record the request body and its `batch` events in arrival order, and respond `207` in the shape the real API uses. It SHALL reject requests whose Basic auth credentials do not match its configured public/secret key pair.

#### Scenario: Batch recorded
- **WHEN** code-trace POSTs an ingestion batch with correct Basic auth
- **THEN** the server responds 207 and the batch's events are retrievable, in order, via the control plane

#### Scenario: Bad credentials rejected
- **WHEN** a POST arrives with missing or incorrect Basic auth
- **THEN** the server responds 401 and records the rejected attempt separately from accepted events

### Requirement: Fake Langfuse implements trace listing and deletion
The server SHALL serve `GET /api/public/traces?sessionId=<id>` returning the traces derived from recorded ingestion events for that session, paginated in the shape code-trace's purge path consumes, and SHALL serve `DELETE /api/public/traces` with a `{"traceIds": [...]}` body removing those traces.

#### Scenario: Purge round-trip
- **WHEN** traces exist for a session and code-trace purges it
- **THEN** the GET lists exactly that session's trace ids, the DELETE removes them, and a subsequent GET returns no traces

### Requirement: Fake Langfuse exposes a test control plane
The server SHALL expose `GET /_test/events` (all recorded requests and events, ordered), `POST /_test/reset` (clear all recorded state), and `POST /_test/hold` (delay responding to subsequent ingestion POSTs by a requested duration, to hold a send in flight).

#### Scenario: Latency injection holds a send in flight
- **WHEN** a test sets a hold of N ms and code-trace's forked child POSTs a batch
- **THEN** the response is delayed by at least N ms and the event is recorded on completion, allowing the test to interleave other operations inside the window

### Requirement: Fake Langfuse binds an ephemeral port
When started by a test the server SHALL bind port 0 and report the assigned address, so concurrent tests never collide on a fixed port.

#### Scenario: Parallel test isolation
- **WHEN** two tests each start their own fake Langfuse
- **THEN** both get distinct addresses and record only their own traffic
