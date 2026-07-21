# code-trace

Send [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [OpenCode](https://opencode.ai), or [Pi](https://github.com/earendil-works/pi) session traces to [Langfuse](https://langfuse.com) for observability.

Runs as a Claude Code [Stop hook](https://docs.anthropic.com/en/docs/claude-code/hooks), an OpenCode plugin, or a Pi extension — after each assistant response, it assembles conversational turns and sends them to Langfuse as structured traces with generations and tool spans.

Written in Rust for fast startup and zero runtime dependencies. The process forks after assembling the payload — the parent exits immediately while the child sends the HTTP request in the background, adding minimal latency to your workflow.

## Supported agents

| Agent | Integration |
|-------|-------------|
| Claude Code | Stop hook (settings.json) |
| OpenCode | Plugin (`~/.config/opencode/plugins/`) |
| Pi | Extension (`~/.pi/agent/extensions/`) |

## What you get in Langfuse

Each turn produces:

- **Trace** with session grouping, input/output, and tags
- **Generation** span with model name and token-level cost tracking
- **Tool spans** for each tool call (Bash, Read, Edit, etc.) with input/output

Traces are automatically tagged with:

| Tag | Example |
|-----|---------|
| `repo:<name>` | `repo:my-project` |
| `branch:<name>` | `branch:main` |
| `user:<name>` | `user:doug` |
| `host:<hostname>` | `host:codex` |
| `os:<platform>` | `os:linux` |
| `cc-version:<ver>` | `cc-version:2.1.100 (Claude Code)` |
| `oc-version:<ver>` | `oc-version:0.4.5 (OpenCode)` |
| `pi-version:<ver>` | `pi-version:1.2.0 (Pi)` |

## Install

### From release (recommended)

```bash
curl -sfL https://raw.githubusercontent.com/isotoma/code-trace/main/install.sh | bash
```

This installs the binary to `~/.local/bin/code-trace`. Make sure `~/.local/bin` is in your `PATH`.

To also install the OpenCode plugin:

```bash
curl -sfL https://raw.githubusercontent.com/isotoma/code-trace/main/install.sh | bash -s -- --opencode
```

To also install the Pi extension:

```bash
curl -sfL https://raw.githubusercontent.com/isotoma/code-trace/main/install.sh | bash -s -- --pi
```

### From source

```bash
cargo install --git https://github.com/isotoma/code-trace.git
```

### Build locally

```bash
git clone https://github.com/isotoma/code-trace.git
cd code-trace
cargo build --release
cp target/release/code-trace ~/.local/bin/
```

## Configuration

### Config file (recommended)

Add your credentials to `~/.config/code-trace/config` (created by the install script):

```
TRACE_TO_LANGFUSE=true
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# LANGFUSE_USER_ID=you@example.com
# CODE_TRACE_DEBUG=false
```

This file is read by the binary at startup and works the same regardless of which agent you're using.

When run interactively, the install script offers to set `LANGFUSE_USER_ID` to your email (unless it is already configured); leave the prompt blank to skip.

Respects `$XDG_CONFIG_HOME`: if set, the file is read from `$XDG_CONFIG_HOME/code-trace/config`.

### Per-agent setup

Each agent also needs the hook/plugin installed so it invokes the binary.

#### Claude Code

The install script does this automatically. If not, add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "code-trace"
          }
        ]
      }
    ]
  }
}
```

#### OpenCode

Copy `plugin/opencode/code-trace.ts` to your OpenCode plugins directory:

```bash
mkdir -p ~/.config/opencode/plugins/
cp plugin/opencode/code-trace.ts ~/.config/opencode/plugins/code-trace.ts
```

Or use the install script with `--opencode` (see above).

#### Pi

Copy `plugin/pi-agent/code-trace.ts` to your Pi extensions directory:

```bash
mkdir -p ~/.pi/agent/extensions/
cp plugin/pi-agent/code-trace.ts ~/.pi/agent/extensions/code-trace.ts
```

Or use the install script with `--pi` (see above).

### Environment variable overrides

Environment variables take precedence over the config file. This is useful for per-project credentials or CI environments.

| Variable | Required | Description |
|----------|----------|-------------|
| `TRACE_TO_LANGFUSE` | Yes | Set to `true` to enable tracing |
| `LANGFUSE_PUBLIC_KEY` | Yes | Langfuse public key |
| `LANGFUSE_SECRET_KEY` | Yes | Langfuse secret key |
| `LANGFUSE_BASE_URL` | No | Langfuse host (default: `https://cloud.langfuse.com`) |
| `LANGFUSE_USER_ID` | No | User to attach to traces (typically your email); enables Langfuse's [user-scoped views](https://langfuse.com/docs/observability/features/users). Omitted when unset. |
| `CODE_TRACE_REQUIRE_GIT_REPO` | No | Restrict tracing to sessions whose working directory is inside a git repository. Defaults to `true`; set to `false` to trace everywhere. |
| `CODE_TRACE_MASK_SECRETS` | No | Redact shape-recognizable secrets (API keys, tokens, private keys) from trace content before sending. Defaults to `true`; set to `false` to disable. |
| `CODE_TRACE_DEBUG` | No | Set to `true` for debug logging (alias: `CC_TRACE_DEBUG`) |

The `CC_LANGFUSE_` prefix is also accepted for all Langfuse variables (e.g. `CC_LANGFUSE_PUBLIC_KEY`).

## Privacy controls

Once `TRACE_TO_LANGFUSE=true` is set, every session traces by default. When working with confidential data, individual sessions can be paused, inspected, and purged without touching the global flag or other concurrently-running agents.

### CLI

```
code-trace                          read a Stop-hook payload on stdin and emit (default)
code-trace --on-start               SessionStart handler: record session, print tracing reminder
code-trace status                   show tracing configuration and session counts
code-trace sessions                 list known sessions (most recent first)
code-trace pause [--session <id>]   pause tracing for a session (default: most recent)
code-trace resume [--session <id>]  resume tracing for a session (default: most recent)
code-trace purge --session <id>     delete a session's Langfuse traces, transcript, and state
code-trace --version / --help
```

Any other invocation falls through to the stdin/emit path, so the installed `Stop` hook keeps working unchanged.

### Private mode (pause/resume)

`code-trace pause` marks the most-recently-seen session as suppressed and prints which session it targeted — with parallel agents, pass `--session <id>` (find ids with `code-trace sessions`) to be explicit. A suppressed session:

- emits nothing, for all sources (Claude Code, OpenCode, Pi) — and turns that happen while paused are **never traced, not deferred**: the hook consumes them (the cursor advances past them) without sending, so they cannot replay after `resume`. Turn numbering in Langfuse shows a gap where the pause was;
- **stays private for its lifetime**, including across `--resume` — only an explicit `code-trace resume` or `purge` clears it;
- is never age-pruned from the registry while suppressed.

New sessions always trace by default. Note that pausing cannot recall a turn whose send was already forked — pause early; purge is the remediation.

**First contact never uploads history.** The first time code-trace sees a session (no cursor in state — e.g. it was just installed mid-session, or an old session is resumed after install), it emits only the turn that fired the hook and fast-forwards past everything earlier. Pre-install content is never traced; the skip is recorded in the log.

Known limitation: `TRACE_TO_LANGFUSE=false` is an on/off switch, not a privacy control — while it is off no state is touched at all, so for a session that already has a cursor, turns completed during the off period will emit when tracing is re-enabled. Use `pause` for "don't trace this"; it is airtight.

### Secret masking

By default (`CODE_TRACE_MASK_SECRETS=true`), trace content is scanned for shape-recognizable secrets — private-key blocks, AWS key ids, GitHub/Slack/Google/Stripe/Anthropic/OpenAI tokens, JWTs, `Bearer` headers, and credentials embedded in URLs — and each match is replaced with a typed placeholder such as `[REDACTED:github-token]` before anything is sent. Masking covers the user prompt, the assistant reply, and every tool call's input (including structured JSON inputs) and output. A per-field redaction count is recorded in the trace metadata (`"redacted": N`) so you can see masking fired without seeing what was masked. Set `CODE_TRACE_MASK_SECRETS=false` to disable.

This is **best-effort**: it catches secrets with recognizable shapes but will miss novel or unstructured ones. It reduces leakage, it does not eliminate it — `pause` and the git-repo restriction remain the airtight controls for confidential work.

### Startup reminder (`--on-start`)

Wired as a Claude Code `SessionStart` hook, `code-trace --on-start` records the session and emits a JSON `systemMessage` — `tracing ENABLED → <host>` or `tracing PAUSED for this session` — which Claude Code shows to the **user** as a terminal banner (not injected into the model's context; the warning is for the human). It prints nothing when tracing is not configured, and never emits traces.

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "code-trace --on-start" }] }
    ]
  }
}
```

### Purge

`code-trace purge --session <id>` removes all three copies of a session's data:

1. **Langfuse traces** — listed via `GET /api/public/traces?sessionId=` and bulk-deleted (cascades to generations and spans);
2. **local transcript** — the Claude Code transcript JSONL, if recorded;
3. **code-trace state** — the session's registry entry and cursor.

Flags: `--langfuse-only` / `--local-only` restrict the scope; `--yes` skips the confirmation prompt; `--transcript-path <p>` purges a session traced before it was in the registry.

Caveats: deleting the transcript removes the session from Claude Code's `--resume` history; purge cannot un-send anything outside code-trace's ownership (shell history, model-provider logs).

## How it works

### Claude Code

1. Claude Code calls the hook after each assistant response, passing a JSON payload on stdin with the `sessionId` and `transcriptPath`
2. The binary reads new lines from the transcript JSONL file (tracking offset in `~/.local/share/code-trace/state.json`)
3. Messages are grouped into turns (user message + assistant responses + tool results)
4. A batch of Langfuse ingestion events is built (`trace-create`, `generation-create`, `span-create`)
5. The process forks — the parent exits immediately, while the child sends the batch to the Langfuse API via HTTP and logs the result

### OpenCode

1. The OpenCode plugin hooks into the `session.idle` event after each assistant response
2. It fetches new messages since the last processed message (tracked per-session in `~/.local/share/code-trace/opencode_cursor.json`)
3. Messages are assembled into turns and piped to the `code-trace` binary over stdin
4. The binary forks — the parent returns immediately, while the child sends the batch to the Langfuse API via HTTP and logs the result

### Pi

1. The Pi extension hooks into the `agent_end` event after each user prompt completes
2. It reads new session entries since the last processed entry (tracked per-session in `~/.local/share/code-trace/pi_agent_cursor.json`)
3. Entries are piped to the `code-trace` binary over stdin
4. The binary normalises Pi's session entry format into turns and forks — the parent returns immediately, while the child sends the batch to Langfuse

## State and logs

State is stored in `~/.local/share/code-trace/`:
- `state.json` — `{ "cursors": ..., "sessions": ... }`: turn cursor per session, plus a session registry (id, source, transcript path, suppressed flag, last seen) used by the privacy CLI. Older flat-shaped files are migrated in place with cursors preserved.
- `state.lock` — blocking exclusive lock serializing all state writes; since 0.3.1 concurrent invocations (parallel agents, pause/purge commands) queue instead of racing, so a `pause` can no longer be lost to a concurrently-running hook
- `opencode_cursor.json` — OpenCode per-session message cursor
- `pi_agent_cursor.json` — Pi per-session entry cursor
- `code_trace.log` — trace log

Note: State was previously stored in `~/.claude/state/`. On first run, existing state is migrated automatically.

Enable debug logging with `CODE_TRACE_DEBUG=true`.

## State migration

On first run after an update, any existing state in `~/.claude/state/code_trace_state.json` is migrated to `~/.local/share/code-trace/state.json`.

## Testing

Two tracks, split by what only each can prove:

- **Track 2 — behaviour and concurrency** (`cargo test`): drives the binary directly with crafted payloads against an in-process fake Langfuse (`tests/support/`), with tmpdir-isolated environments. Covers pause/resume/purge semantics, the purge-vs-in-flight-send window, and concurrent-invocation scenarios (`tests/concurrency_test.rs`). Tests marked `#[ignore = "red until fix-state-locking..."]` demonstrate known state-locking defects and are un-ignored by that fix.
- **Track 1 — the real Claude Code seam** (`harness/`): runs the pinned real `claude` CLI in a container with code-trace wired as hooks, a stub model API, and the same fake Langfuse as a service. Proves hook wiring, real payload shapes, and config-file discovery. See `harness/README.md`.

`CODE_TRACE_SYNC_SEND=1` (tests only) makes the Langfuse send inline instead of forked, so process exit implies delivery — used for exact "nothing was sent" assertions.

## License

MIT
