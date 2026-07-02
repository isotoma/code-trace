# Track 1 spike notes: what `claude -p` needs from a stub model API

Captured 2026-07-02 against `claude` CLI **2.1.198** with a recording stub on
`ANTHROPIC_BASE_URL`. Re-verify when bumping the pinned CLI version.

## Requests the CLI makes

- Exactly one endpoint is required: `POST /v1/messages?beta=true`.
  **Route on the path with the query string stripped** — matching the raw
  path against `/v1/messages` fails and the CLI reports
  "There's an issue with the selected model".
- Auth arrives as `x-api-key: $ANTHROPIC_API_KEY`. Any non-empty dummy value
  works; nothing validates it.
- Headers seen: `anthropic-version: 2023-06-01`, `anthropic-beta:
  claude-code-20250219,interleaved-thinking...`, `accept: application/json`.
- The turn request has `stream: true`. On a failed response the CLI retries
  once without `stream`, so the stub should answer both shapes.
- With `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` no other network calls
  are attempted (no telemetry, no statsig, no update check).

## Response the CLI accepts

A minimal SSE stream suffices (HTTP/1.0-style connection-close framing from
Python's `http.server` is accepted):

```
message_start        {type, message: {id, type: "message", role, model, content: [],
                      stop_reason: null, stop_sequence: null, usage: {input_tokens, output_tokens}}}
content_block_start  {type, index: 0, content_block: {type: "text", text: ""}}
content_block_delta  {type, index: 0, delta: {type: "text_delta", text: "..."}}
content_block_stop   {type, index: 0}
message_delta        {type, delta: {stop_reason: "end_turn", stop_sequence: null},
                      usage: {output_tokens}}
message_stop         {type}
```

Non-streaming fallback: a plain `message` object with `content`,
`stop_reason: "end_turn"`, and `usage`.

## Environment for a hermetic run

- Isolated `HOME` works; `claude -p` has no onboarding blocker in print mode.
- `ANTHROPIC_BASE_URL=http://stub:port`, `ANTHROPIC_API_KEY=dummy`.
- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, `DISABLE_TELEMETRY=1`,
  `DISABLE_ERROR_REPORTING=1`.
- Pass a model explicitly (`--model`) so runs don't depend on account defaults.
- Redirect stdin (`< /dev/null`) — otherwise `-p` waits ~3s for piped stdin.

## Open items

- Tool-use turns: the stub only scripts text turns. Scenarios needing the
  model to "use a tool" would need `tool_use` content blocks; current Track 1
  scenarios don't require them.
- Multi-turn sessions use `--session-id <uuid>` / `--resume <uuid>` so the
  session id is known to the runner ahead of time.
