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

### Claude Code

#### 1. Register the hook

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

#### 2. Set credentials per project

Add to `.claude/settings.local.json` in your project root:

```json
{
  "env": {
    "TRACE_TO_LANGFUSE": "true",
    "LANGFUSE_PUBLIC_KEY": "pk-lf-...",
    "LANGFUSE_SECRET_KEY": "sk-lf-..."
  }
}
```

Or set them globally if you want tracing on all projects.

### OpenCode

#### 1. Install the plugin

Copy `plugin/opencode/code-trace.ts` to your OpenCode plugins directory:

```bash
mkdir -p ~/.config/opencode/plugins/
cp plugin/opencode/code-trace.ts ~/.config/opencode/plugins/code-trace.ts
```

#### 2. Set environment variables

Enable tracing in your shell profile (`.bashrc`, `.zshrc`, etc.):

```bash
export TRACE_TO_LANGFUSE=true
export LANGFUSE_PUBLIC_KEY=pk-lf-...
export LANGFUSE_SECRET_KEY=sk-lf-...
export LANGFUSE_BASE_URL=https://cloud.langfuse.com  # optional, defaults to cloud.langfuse.com
```

### Pi

#### 1. Install the extension

Copy `plugin/pi-agent/code-trace.ts` to your Pi extensions directory:

```bash
mkdir -p ~/.pi/agent/extensions/
cp plugin/pi-agent/code-trace.ts ~/.pi/agent/extensions/code-trace.ts
```


Or use the install script with `--pi` (see above).

#### 2. Set environment variables

Enable tracing in your shell profile (`.bashrc`, `.zshrc`, etc.):

```bash
export TRACE_TO_LANGFUSE=true
export LANGFUSE_PUBLIC_KEY=pk-lf-...
export LANGFUSE_SECRET_KEY=sk-lf-...
export LANGFUSE_BASE_URL=https://cloud.langfuse.com  # optional
```

## Environment variables

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

### Pi

1. The Pi extension hooks into the `agent_end` event after each user prompt completes
2. It reads new session entries since the last processed entry (tracked per-session in `~/.local/share/code-trace/pi_agent_cursor.json`)
3. Entries are piped to the `code-trace` binary over stdin
4. The binary normalises Pi's session entry format into turns and forks — the parent returns immediately, while the child sends the batch to Langfuse

## State and logs

State is stored in `~/.local/share/code-trace/`:
- `state.json` — turn cursor per session
- `state.lock` — file lock for concurrent access
- `opencode_cursor.json` — OpenCode per-session message cursor
- `pi_agent_cursor.json` — Pi per-session entry cursor
- `code_trace.log` — trace log

Note: State was previously stored in `~/.claude/state/`. On first run, existing state is migrated automatically.

Enable debug logging with `CODE_TRACE_DEBUG=true`.

## State migration

On first run after an update, any existing state in `~/.claude/state/code_trace_state.json` is migrated to `~/.local/share/code-trace/state.json`.

## License

MIT
