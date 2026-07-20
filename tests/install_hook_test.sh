#!/usr/bin/env bash
#
# Tests for register_claude_code_hook in install.sh.
#
# Sources install.sh as a library (CODE_TRACE_INSTALL_LIB=1 suppresses main)
# and drives the hook-registration logic against throwaway settings files.
#
# Run directly:  bash tests/install_hook_test.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_SH="${SCRIPT_DIR}/../install.sh"

CODE_TRACE_INSTALL_LIB=1 source "${INSTALL_SH}"

TMPROOT="$(mktemp -d)"
trap 'rm -rf "${TMPROOT}"' EXIT

PASS=0
FAIL=0

# Count code-trace Stop hooks and report their commands, via python.
# Prints: "<count> <cmd1>|<cmd2>|..."
inspect() {
  python3 - "$1" <<'PY'
import json, os, sys
with open(sys.argv[1]) as f:
    settings = json.load(f)
cmds = []
for entry in settings.get("hooks", {}).get("Stop", []):
    for h in entry.get("hooks", []):
        cmd = h.get("command")
        if isinstance(cmd, str) and os.path.basename(cmd.strip().split()[0]) == "code-trace":
            cmds.append(cmd)
print(f"{len(cmds)} {'|'.join(cmds)}")
PY
}

# Assert a full-file predicate via grep; args: <file> <description> <grep-pattern> <expect-present:0|1>
check() {
  local desc="$1" actual="$2" expected="$3"
  if [ "${actual}" = "${expected}" ]; then
    echo "ok   - ${desc}"
    PASS=$((PASS + 1))
  else
    echo "FAIL - ${desc}"
    echo "         expected: ${expected}"
    echo "         actual:   ${actual}"
    FAIL=$((FAIL + 1))
  fi
}

# --- Test 1: no settings file at all -> creates one with the canonical hook ---
f="${TMPROOT}/fresh.json"
register_claude_code_hook "${f}" >/dev/null
check "fresh install registers exactly one canonical hook" "$(inspect "${f}")" "1 code-trace"

# --- Test 2: legacy absolute-path hook -> migrated to canonical (the reported bug) ---
f="${TMPROOT}/legacy.json"
cat > "${f}" <<'JSON'
{
  "model": "opus",
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "~/.claude/hooks/code-trace" } ] }
    ]
  }
}
JSON
register_claude_code_hook "${f}" >/dev/null
check "legacy ~/.claude/hooks/code-trace hook is migrated to bare command" "$(inspect "${f}")" "1 code-trace"
# Model key must be preserved.
check "migration preserves unrelated settings (model)" \
  "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("model"))' "${f}")" "opus"

# --- Test 3: canonical hook already present -> idempotent, stays single ---
f="${TMPROOT}/already.json"
cat > "${f}" <<'JSON'
{
  "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "code-trace" } ] } ] }
}
JSON
register_claude_code_hook "${f}" >/dev/null
check "existing canonical hook stays single (idempotent)" "$(inspect "${f}")" "1 code-trace"

# --- Test 4: unrelated Stop hook present -> canonical added, unrelated preserved ---
f="${TMPROOT}/coexist.json"
cat > "${f}" <<'JSON'
{
  "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "some-other-tool" } ] } ] }
}
JSON
register_claude_code_hook "${f}" >/dev/null
check "canonical hook added alongside unrelated Stop hook" "$(inspect "${f}")" "1 code-trace"
check "unrelated Stop hook is preserved" \
  "$(grep -c 'some-other-tool' "${f}")" "1"

# --- Test 5: running twice is idempotent ---
f="${TMPROOT}/twice.json"
register_claude_code_hook "${f}" >/dev/null
register_claude_code_hook "${f}" >/dev/null
check "running twice yields exactly one hook" "$(inspect "${f}")" "1 code-trace"

# --- Test 6: legacy hook that also carries arguments is still recognised ---
f="${TMPROOT}/legacy-args.json"
cat > "${f}" <<'JSON'
{
  "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "/opt/bin/code-trace --verbose" } ] } ] }
}
JSON
register_claude_code_hook "${f}" >/dev/null
check "legacy hook with args is migrated (not duplicated)" "$(inspect "${f}")" "1 code-trace"

echo ""
echo "${PASS} passed, ${FAIL} failed"
[ "${FAIL}" -eq 0 ]
