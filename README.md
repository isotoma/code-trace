# code-trace

Send [Claude Code](https://docs.anthropic.com/en/docs/claude-code) or [OpenCode](https://opencode.ai) session traces to [Langfuse](https://langfuse.com) for observability.

Runs as a Claude Code [Stop hook](https://docs.anthropic.com/en/docs/claude-code/hooks) or an OpenCode plugin — after each assistant response, it reads the session transcript, assembles conversational turns, and sends them to Langfuse as structured traces with generations and tool spans.

Written in Rust for fast startup and zero runtime dependencies. The process forks after assembling the payload — the parent exits immediately while the child sends the HTTP request in the background, adding minimal latency to your workflow.

## Supported agents

| Agent | Integration |
|-------|-------------|
| Claude Code | Stop hook (settings.json) |
| OpenCode | Plugin (`.opencode/plugins/` or npm) |

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
# CODE_TRACE_DEBUG=false
```

This file is read by the binary at startup and works the same regardless of which agent you're using.

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
| `CODE_TRACE_DEBUG` | No | Set to `true` for debug logging (alias: `CC_TRACE_DEBUG`) |

The `CC_LANGFUSE_` prefix is also accepted for all Langfuse variables (e.g. `CC_LANGFUSE_PUBLIC_KEY`).

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

## State and logs

State is stored in `~/.local/share/code-trace/`:
- `state.json` — turn cursor per session
- `state.lock` — file lock for concurrent access
- `opencode_cursor.json` — OpenCode per-session message cursor
- `code_trace.log` — trace log

Note: State was previously stored in `~/.claude/state/`. On first run, existing state is migrated automatically.

Enable debug logging with `CODE_TRACE_DEBUG=true`.

## State migration

On first run after an update, any existing state in `~/.claude/state/code_trace_state.json` is migrated to `~/.local/share/code-trace/state.json`.

## License

MIT
