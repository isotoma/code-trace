This project sends Claude Code and OpenCode session traces to Langfuse for observability.

## Architecture

- Single Rust binary that supports three input sources
- Claude Code: invoked as a Stop hook, reads transcript JSONL
- OpenCode: invoked by a TypeScript plugin, receives messages via stdin
- Pi: invoked by a TypeScript extension, receives session entries via stdin

## Key files

- `src/main.rs` — entry point, dispatches on `Input` enum
- `src/source.rs` — `Source` enum (ClaudeCode | Opencode | PiAgent)
- `src/payload.rs` — parses stdin into `Input` enum
- `src/opencode.rs` — normalizes OpenCode SDK message format to Claude-format Values
- `src/pi_agent.rs` — normalizes Pi session entry format to Claude-format Values
- `src/transcript.rs` — reads Claude Code JSONL transcript
- `src/turns.rs` — groups messages into user/assistant/tool turns
- `src/emit.rs` — builds Langfuse ingestion batch
- `src/tags.rs` — gathers env tags (repo, branch, user, host, os, agent version)
- `src/state.rs` — persisted cursor state per session
- `src/log.rs` — logging to `~/.local/share/code-trace/`
- `plugin/pi-agent/code-trace.ts` — pi extension

## State location

State moved from `~/.claude/state/` to `~/.local/share/code-trace/`. Migration happens on first run.

## Building

```bash
cargo build --release
```

## Testing

```bash
cargo test
```

Integration tests share `tests/support/` (env-isolated `TestEnv`, payload builders, in-process fake Langfuse). `tests/concurrency_test.rs` is the Track 2 race suite; tests `#[ignore]`d "red until fix-state-locking" are expected failures demonstrating the non-blocking-flock bug. `CODE_TRACE_SYNC_SEND=1` makes sends inline (no fork) for exact delivery assertions. The Track 1 container harness (real `claude` + stub model API + fake Langfuse) lives in `harness/` — see `harness/README.md`.

## Adding a new source

1. Add variant to `src/source.rs` `Source` enum
2. Add variant to `src/payload.rs` `Input` enum with parsing
3. Add message normalizer (e.g. `src/opencode.rs`) if needed
4. Update `src/tags.rs` `gather_env_tags` for source-specific tags
5. Update `src/emit.rs` `build_ingestion_batch` for source-specific trace name/metadata
6. Update `src/main.rs` match arm for the new source
7. Update `tests/integration_test.rs` with fixture
