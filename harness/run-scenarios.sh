#!/usr/bin/env bash
# Track 1 scenarios: real `claude` + code-trace hooks against the stub model
# and fake Langfuse. Runs inside the harness container (see docker-compose.yml).
# Requires: FAKE_LANGFUSE_URL, ANTHROPIC_BASE_URL, ANTHROPIC_API_KEY.
set -euo pipefail

FAKE=${FAKE_LANGFUSE_URL:?}
MODEL=${CLAUDE_MODEL:-claude-sonnet-5}
WORK=${HARNESS_WORK:-/work}
export HOME=${HARNESS_HOME:-/tmp/harness-home}
PASS=0

say()  { echo "=== $*"; }
fail() { echo "FAIL: $*" >&2; echo "--- fake langfuse dump ---" >&2; curl -s "$FAKE/_test/events" >&2; echo >&2; exit 1; }

# events_py <python-expr over d> — evaluate against the fake's event dump.
events_py() {
  curl -s "$FAKE/_test/events" | python3 -c "
import json, sys
d = json.load(sys.stdin)
traces = [(e['body'].get('sessionId'), e['body'].get('metadata', {}).get('turn_number'))
          for e in d['events'] if e.get('type') == 'trace-create']
posts = [r for r in d['requests'] if r['path'].startswith('/api/public/ingestion') and r['authorized']]
print($1)
"
}

reset_fake() { curl -s -X POST "$FAKE/_test/reset" > /dev/null; }

# Compose's depends_on only orders container start, not server readiness —
# wait until both services actually answer before running any scenario.
wait_for_services() {
  for _ in $(seq 60); do
    if curl -sf -o /dev/null "$FAKE/_test/events" \
       && curl -s -o /dev/null -X POST -d '{}' "${ANTHROPIC_BASE_URL:?}/v1/messages"; then
      return 0
    fi
    sleep 1
  done
  fail "services not reachable after 60s (fake langfuse / stub model)"
}

new_home() {
  rm -rf "$HOME" && mkdir -p "$HOME/.claude" "$WORK"
}

# Hook wiring with tracing configured via the settings env block.
write_settings_env_mode() {
  cat > "$HOME/.claude/settings.json" <<EOF
{
  "env": {
    "TRACE_TO_LANGFUSE": "true",
    "LANGFUSE_PUBLIC_KEY": "pk-test",
    "LANGFUSE_SECRET_KEY": "sk-test",
    "LANGFUSE_BASE_URL": "$FAKE",
    "CODE_TRACE_SYNC_SEND": "1",
    "CODE_TRACE_REQUIRE_GIT_REPO": "false"
  },
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "code-trace --on-start"}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "code-trace"}]}]
  }
}
EOF
}

# Hook wiring only; tracing configured solely via the code-trace config file.
write_settings_config_mode() {
  cat > "$HOME/.claude/settings.json" <<'EOF'
{
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "code-trace --on-start"}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "code-trace"}]}]
  }
}
EOF
  mkdir -p "$HOME/.config/code-trace"
  cat > "$HOME/.config/code-trace/config" <<EOF
TRACE_TO_LANGFUSE=true
LANGFUSE_PUBLIC_KEY=pk-test
LANGFUSE_SECRET_KEY=sk-test
LANGFUSE_BASE_URL=$FAKE
CODE_TRACE_SYNC_SEND=1
# /work is not a git repo in the harness; keep the default git-repo gate off
# so scenarios exercise the trace pipeline, not the gate (covered by unit tests).
CODE_TRACE_REQUIRE_GIT_REPO=false
EOF
}

run_claude() { # session-id, prompt, extra flags...
  local sid=$1 prompt=$2; shift 2
  (cd "$WORK" && timeout 120 claude -p "$prompt" --model "$MODEL" "$@" --session-id "$sid" < /dev/null) \
    || fail "claude -p exited non-zero (session $sid)"
}

run_claude_resume() { # session-id, prompt
  local sid=$1 prompt=$2
  (cd "$WORK" && timeout 120 claude -p "$prompt" --model "$MODEL" --resume "$sid" < /dev/null) \
    || fail "claude -p --resume exited non-zero (session $sid)"
}

uuid() { python3 -c "import uuid; print(uuid.uuid4())"; }

transcript_of() { # session-id
  find "$HOME/.claude/projects" -name "$1.jsonl" | head -1
}

wait_for_services

# --- (a) one turn -> one trace with the session id ---------------------------
say "scenario a: one turn produces one trace"
new_home; write_settings_env_mode; reset_fake
S=$(uuid)
run_claude "$S" "say hi"
[ "$(events_py "traces == [('$S', 1)]")" = "True" ] || fail "expected exactly [('$S', 1)], scenario a"
PASS=$((PASS+1))

# --- (b) config-file-only configuration produces a trace ----------------------
say "scenario b: config-file-only tracing"
new_home; write_settings_config_mode; reset_fake
S=$(uuid)
run_claude "$S" "say hi"
[ "$(events_py "traces == [('$S', 1)]")" = "True" ] || fail "config-file-only turn did not trace, scenario b"
PASS=$((PASS+1))

# --- (c) --on-start reminder appears; --on-start never ingests ----------------
say "scenario c: startup reminder in session context"
T=$(transcript_of "$S")
[ -n "$T" ] || fail "no transcript found for $S, scenario c"
grep -q "tracing ENABLED" "$T" || fail "reminder line missing from transcript, scenario c"
# The only ingestion so far is scenario b's single Stop-hook post.
[ "$(events_py "len(posts)")" = "1" ] || fail "--on-start must never ingest, scenario c"
PASS=$((PASS+1))

# --- (d) pause mid-session -> subsequent turn sends nothing --------------------
say "scenario d: paused session sends nothing"
# Continue in scenario b's HOME/session: pause it, then another turn.
TRACE_TO_LANGFUSE=true LANGFUSE_PUBLIC_KEY=pk-test LANGFUSE_SECRET_KEY=sk-test \
  LANGFUSE_BASE_URL=$FAKE code-trace pause --session "$S" || fail "pause failed, scenario d"
run_claude_resume "$S" "say more"
[ "$(events_py "len(posts)")" = "1" ] || fail "paused session emitted, scenario d"
[ "$(events_py "traces == [('$S', 1)]")" = "True" ] || fail "unexpected traces, scenario d"
PASS=$((PASS+1))

# --- (e) resume of the paused session stays paused -----------------------------
say "scenario e: suppression survives resume"
run_claude_resume "$S" "and again"
[ "$(events_py "len(posts)")" = "1" ] || fail "resumed paused session emitted, scenario e"
grep -q "tracing PAUSED" "$(transcript_of "$S")" || fail "paused reminder missing on resume, scenario e"
PASS=$((PASS+1))

say "all $PASS scenarios passed"
