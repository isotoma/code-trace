# code-trace OpenCode Plugin

Sends [OpenCode](https://opencode.ai) session traces to [Langfuse](https://langfuse.com) for observability.

This is the companion plugin for the [`code-trace`](https://github.com/isotoma/code-trace) Rust binary.

## Setup

### 1. Install code-trace binary

```bash
curl -sfL https://raw.githubusercontent.com/isotoma/code-trace/main/install.sh | bash
```

Or build from source:

```bash
cargo install --git https://github.com/isotoma/code-trace.git
```

### 2. Install the plugin

Copy `code-trace.ts` to your OpenCode plugins directory:

```bash
mkdir -p ~/.config/opencode/plugins/
cp code-trace.ts ~/.config/opencode/plugins/code-trace.ts
```

Or add to your `opencode.json`:

```json
{
  "plugin": ["code-trace"]
}
```

### 3. Set environment variables

Enable tracing and set your Langfuse credentials:

```bash
export TRACE_TO_LANGFUSE=true
export LANGFUSE_PUBLIC_KEY=pk-lf-...
export LANGFUSE_SECRET_KEY=sk-lf-...
export LANGFUSE_BASE_URL=https://cloud.langfuse.com  # optional, defaults to cloud.langfuse.com
```

Or add them to your shell profile (`.bashrc`, `.zshrc`, etc.).

## How it works

The plugin hooks into OpenCode's `session.idle` event, which fires after each assistant response.

On each idle event:

1. The plugin fetches new messages since the last processed message (tracked per-session in `~/.local/share/code-trace/opencode_cursor.json`)
2. Messages are assembled into turns (user input + assistant responses + tool results)
3. A JSON payload is sent to the `code-trace` binary over stdin
4. The binary forks — the parent returns immediately while the child sends the trace to Langfuse

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
| `oc-version:<ver>` | `oc-version:0.4.5` |

## Troubleshooting

Enable debug logging:

```bash
export CODE_TRACE_DEBUG=true
```

Then check the log file at `~/.local/share/code-trace/code_trace.log`.

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `TRACE_TO_LANGFUSE` | Yes | Set to `true` to enable tracing |
| `LANGFUSE_PUBLIC_KEY` | Yes | Langfuse public key |
| `LANGFUSE_SECRET_KEY` | Yes | Langfuse secret key |
| `LANGFUSE_BASE_URL` | No | Langfuse host (default: `https://cloud.langfuse.com`) |
| `CODE_TRACE_DEBUG` | No | Set to `true` for debug logging |
| `CODE_TRACE_BIN` | No | Path to `code-trace` binary (default: `code-trace` on PATH) |
