# code-trace

Send [Claude Code](https://docs.anthropic.com/en/docs/claude-code) session traces to [Langfuse](https://langfuse.com) for observability.

Runs as a Claude Code [Stop hook](https://docs.anthropic.com/en/docs/claude-code/hooks) — after each assistant response, it reads the session transcript, assembles conversational turns, and sends them to Langfuse as structured traces with generations and tool spans.

Written in Rust for fast startup and zero runtime dependencies. The HTTP request is fire-and-forget (spawns `curl` in the background), so the hook adds minimal latency to your workflow.

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

## Install

### From release (recommended)

```bash
curl -sfL https://raw.githubusercontent.com/isotoma/code-trace/main/install.sh | bash
```

This installs the binary to `~/.local/bin/code-trace`. Make sure `~/.local/bin` is in your `PATH`.

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

### 1. Register the hook

Add to `~/.claude/settings.json`:

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

### 2. Set credentials per project

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

## Environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| `TRACE_TO_LANGFUSE` | Yes | Set to `true` to enable tracing |
| `LANGFUSE_PUBLIC_KEY` | Yes | Langfuse public key |
| `LANGFUSE_SECRET_KEY` | Yes | Langfuse secret key |
| `LANGFUSE_BASE_URL` | No | Langfuse host (default: `https://cloud.langfuse.com`) |
| `CC_TRACE_DEBUG` | No | Set to `true` for debug logging |

The `CC_LANGFUSE_` prefix is also accepted for all Langfuse variables (e.g. `CC_LANGFUSE_PUBLIC_KEY`).

## How it works

1. Claude Code calls the hook after each assistant response, passing a JSON payload on stdin with the `sessionId` and `transcriptPath`
2. The binary reads new lines from the transcript JSONL file (tracking offset in `~/.claude/state/code_trace_state.json`)
3. Messages are grouped into turns (user message + assistant responses + tool results)
4. A batch of Langfuse ingestion events is built (`trace-create`, `generation-create`, `span-create`)
5. The batch is sent via a background `curl` process — the hook exits immediately without waiting

## Logs

Logs are written to `~/.claude/state/code_trace.log`. Enable debug logging with `CC_TRACE_DEBUG=true`.

## License

MIT
