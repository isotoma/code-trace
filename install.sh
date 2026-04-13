#!/usr/bin/env bash
set -euo pipefail

REPO="isotoma/code-trace"
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"
SETTINGS_FILE="${HOME}/.claude/settings.json"

# Detect platform
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${ARCH}" in
  x86_64|amd64)  ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *)
    echo "Unsupported architecture: ${ARCH}" >&2
    exit 1
    ;;
esac

case "${OS}" in
  linux)  TARGET="${ARCH}-unknown-linux-gnu" ;;
  darwin) TARGET="${ARCH}-apple-darwin" ;;
  *)
    echo "Unsupported OS: ${OS}" >&2
    exit 1
    ;;
esac

ASSET="${BINARY}-${TARGET}"

# Get latest release URL
DOWNLOAD_URL="$(curl -sfL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep "browser_download_url.*${ASSET}" \
  | head -1 \
  | cut -d '"' -f 4)"

if [ -z "${DOWNLOAD_URL}" ]; then
  echo "Could not find release asset for ${ASSET}" >&2
  exit 1
fi

echo "Downloading ${BINARY} for ${TARGET}..."
mkdir -p "${INSTALL_DIR}"
curl -sfL "${DOWNLOAD_URL}" -o "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"
echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

# Add ~/.local/bin to PATH if not already present
if ! echo "${PATH}" | tr ':' '\n' | grep -qx "${INSTALL_DIR}"; then
  SHELL_NAME="$(basename "${SHELL:-bash}")"
  case "${SHELL_NAME}" in
    zsh)  RC_FILE="${HOME}/.zshrc" ;;
    *)    RC_FILE="${HOME}/.bashrc" ;;
  esac

  if [ -f "${RC_FILE}" ] && grep -q "${INSTALL_DIR}" "${RC_FILE}"; then
    echo "${INSTALL_DIR} already in ${RC_FILE} (not currently in PATH — restart your shell)"
  else
    echo "" >> "${RC_FILE}"
    echo "export PATH=\"${INSTALL_DIR}:\${PATH}\"" >> "${RC_FILE}"
    echo "Added ${INSTALL_DIR} to PATH in ${RC_FILE}"
    echo "Run: source ${RC_FILE} (or restart your shell)"
  fi
fi

# Register the Claude Code hook
HOOK_ENTRY='{"type":"command","command":"code-trace"}'

if [ -f "${SETTINGS_FILE}" ]; then
  # Check if code-trace hook is already registered
  if grep -q "code-trace" "${SETTINGS_FILE}"; then
    echo "Hook already registered in ${SETTINGS_FILE}"
  else
    # Merge the hook into existing settings using python (available on macOS and most Linux)
    python3 -c "
import json, sys

with open('${SETTINGS_FILE}') as f:
    settings = json.load(f)

hook = {'type': 'command', 'command': 'code-trace'}
hooks = settings.setdefault('hooks', {})
stop = hooks.setdefault('Stop', [])

# Find or create the hooks list entry
for entry in stop:
    if 'hooks' in entry:
        entry['hooks'].append(hook)
        break
else:
    stop.append({'hooks': [hook]})

with open('${SETTINGS_FILE}', 'w') as f:
    json.dump(settings, f, indent=2)
    f.write('\n')
" && echo "Registered hook in ${SETTINGS_FILE}" || echo "Could not update ${SETTINGS_FILE} — please add the hook manually"
  fi
else
  mkdir -p "$(dirname "${SETTINGS_FILE}")"
  cat > "${SETTINGS_FILE}" << 'EOF'
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
EOF
  echo "Created ${SETTINGS_FILE} with hook"
fi

echo ""
echo "Done! To enable tracing, add to your project's .claude/settings.local.json:"
echo ""
cat << 'EOF'
{
  "env": {
    "TRACE_TO_LANGFUSE": "true",
    "LANGFUSE_PUBLIC_KEY": "pk-lf-...",
    "LANGFUSE_SECRET_KEY": "sk-lf-..."
  }
}
EOF
