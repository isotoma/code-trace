# Track 1 harness: real `claude` ↔ code-trace seam tests

Runs the real Claude Code CLI (pinned version) headless in a container with
code-trace wired as SessionStart/Stop hooks, a **stub Anthropic Messages API**
(canned responses — no key, no cost, no network), and the **fake Langfuse**
from the Track 2 test suite exposed as a service. Verifies the things crafted
payloads cannot: hook wiring fires, real payload shapes parse, config-file
discovery works, and pause survives `claude --resume`.

Concurrency/race coverage lives in Track 2 (`cargo test`), not here — see
`tests/concurrency_test.rs`.

## Run it

```bash
docker compose -f harness/docker-compose.yml up --build \
  --exit-code-from runner --abort-on-container-exit
```

The `runner` service executes `run-scenarios.sh` and exits non-zero on the
first failing scenario, dumping the fake Langfuse event log.

Hooks are registered by the **real installer** (`code-trace setup
--register-hook`), not hand-written JSON, so the scenarios exercise the wiring
users actually get — including the SessionStart reminder hook.

### Behind a private npm registry

The image build runs `npm install -g @anthropic-ai/claude-code`. On networks
that only reach npm through a private registry (so a clean container cannot hit
`registry.npmjs.org` directly), point `NPMRC_FILE` at an `~/.npmrc` that routes
through it — passed to the build as a BuildKit secret, never baked into a layer:

```bash
NPMRC_FILE="$HOME/.npmrc" docker compose -f harness/docker-compose.yml up --build \
  --exit-code-from runner --abort-on-container-exit
```

Leave `NPMRC_FILE` unset (the default) where the registry is directly reachable,
such as CI.

## Run scenarios without Docker

The runner script only needs `claude`, the two binaries, and python3 on PATH:

```bash
cargo build --bin code-trace
cargo build --bin fake-langfuse --features harness
./target/debug/fake-langfuse &                      # port 3080
STUB_MODEL_PORT=3081 python3 harness/stub-model/server.py &

PATH="$PWD/target/debug:$PATH" \
FAKE_LANGFUSE_URL=http://127.0.0.1:3080 \
ANTHROPIC_BASE_URL=http://127.0.0.1:3081 ANTHROPIC_API_KEY=sk-ant-dummy \
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_TELEMETRY=1 DISABLE_ERROR_REPORTING=1 \
HARNESS_HOME=/tmp/harness-home HARNESS_WORK=/tmp/harness-work \
bash harness/run-scenarios.sh
```

Uses an isolated `HOME`, so your real `~/.claude` and code-trace state are
untouched.

## Scenarios

| # | Verifies |
|---|---|
| a | One prompted turn → one trace with claude's own session id |
| b | Tracing configured **only** via the code-trace config file still works |
| c | `--on-start` reminder appears in session context; `--on-start` never ingests |
| d | `code-trace pause` mid-session → subsequent turn sends nothing |
| e | `claude --resume` of the paused session stays paused |

## Pieces

- `Dockerfile` — one image, three roles (runner / stub-model / fake-langfuse)
- `docker-compose.yml` — wires the three services
- `stub-model/server.py` — minimal Messages API; see `NOTES.md` for the spike
  that derived exactly what `claude -p` requires
- `run-scenarios.sh` — the scenario suite and its assertions
- `NOTES.md` — spike findings; **re-verify when bumping the pinned CLI version**
  (`CLAUDE_CODE_VERSION` arg in the Dockerfile)
