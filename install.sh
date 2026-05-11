#!/usr/bin/env bash
set -euo pipefail

REPO="isotoma/code-trace"
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SETTINGS_FILE="${HOME}/.claude/settings.json"
OPENCODE_PLUGIN_DIR="${HOME}/.config/opencode/plugins"
PI_EXTENSION_DIR="${HOME}/.pi/agent/extensions"

# Parse flags
INSTALL_OPENCODE=false
if [ "${1:-}" = "--opencode" ] || [ "${1:-}" = "-o" ]; then
  INSTALL_OPENCODE=true
fi

INSTALL_PI=false
if [ "${1:-}" = "--pi" ] || [ "${1:-}" = "-p" ]; then
  INSTALL_PI=true
fi

detect_opencode() {
  [ -d "${HOME}/.config/opencode" ] || [ -f "${HOME}/.config/opencode/opencode.json" ]
}

detect_pi() {
  [ -d "${HOME}/.pi/agent" ]
}

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
  LOCAL_BIN="${SCRIPT_DIR}/target/release/${BINARY}"
  if [ -f "${LOCAL_BIN}" ]; then
    echo "No release found; using local build: ${LOCAL_BIN}"
    mkdir -p "${INSTALL_DIR}"
    cp "${LOCAL_BIN}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"
    echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
  else
    echo "Could not find release asset for ${ASSET} and no local build found." >&2
    echo "Run 'cargo build --release' first, or check that the repo has a published release." >&2
    exit 1
  fi
else
  echo "Downloading ${BINARY} for ${TARGET}..."
  mkdir -p "${INSTALL_DIR}"
  curl -sfL "${DOWNLOAD_URL}" -o "${INSTALL_DIR}/${BINARY}"
  chmod +x "${INSTALL_DIR}/${BINARY}"
  echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
fi

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

# Determine plugin source location
PLUGIN_SRC=""
if [ -f "${SCRIPT_DIR}/plugin/opencode/code-trace.ts" ]; then
  PLUGIN_SRC="${SCRIPT_DIR}/plugin/opencode/code-trace.ts"
elif [ -f "${SCRIPT_DIR}/../plugin/opencode/code-trace.ts" ]; then
  PLUGIN_SRC="$(cd "${SCRIPT_DIR}/../plugin/opencode" && pwd)/code-trace.ts"
fi

PI_PLUGIN_SRC=""
if [ -f "${SCRIPT_DIR}/plugin/pi-agent/code-trace.ts" ]; then
  PI_PLUGIN_SRC="${SCRIPT_DIR}/plugin/pi-agent/code-trace.ts"
elif [ -f "${SCRIPT_DIR}/../plugin/pi-agent/code-trace.ts" ]; then
  PI_PLUGIN_SRC="$(cd "${SCRIPT_DIR}/../plugin/pi-agent" && pwd)/code-trace.ts"
fi

# Register Claude Code hook
HOOK_ENTRY='{"type":"command","command":"code-trace"}'

if [ -f "${SETTINGS_FILE}" ]; then
  if grep -q "code-trace" "${SETTINGS_FILE}"; then
    echo "Hook already registered in ${SETTINGS_FILE}"
  else
    python3 -c "
import json, sys

with open('${SETTINGS_FILE}') as f:
    settings = json.load(f)

hook = {'type': 'command', 'command': 'code-trace'}
hooks = settings.setdefault('hooks', {})
stop = hooks.setdefault('Stop', [])

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

# Install OpenCode plugin
install_opencode_plugin() {
  if [ -z "${PLUGIN_SRC}" ] || [ ! -f "${PLUGIN_SRC}" ]; then
    echo ""
    echo "Note: Plugin source not found at ${PLUGIN_SRC}. Skipping OpenCode plugin install."
    echo "You can manually copy plugin/opencode/code-trace.ts to ${OPENCODE_PLUGIN_DIR}/"
    return
  fi

  mkdir -p "${OPENCODE_PLUGIN_DIR}"
  cp "${PLUGIN_SRC}" "${OPENCODE_PLUGIN_DIR}/code-trace.ts"
  echo "Installed OpenCode plugin to ${OPENCODE_PLUGIN_DIR}/code-trace.ts"
}

# Install Pi Agent extension
install_pi_extension() {
  if [ -z "${PI_PLUGIN_SRC}" ] || [ ! -f "${PI_PLUGIN_SRC}" ]; then
    echo ""
    echo "Note: Pi extension source not found at ${PI_PLUGIN_SRC}. Skipping Pi Agent extension install."
    echo "You can manually copy plugin/pi-agent/code-trace.ts to ${PI_EXTENSION_DIR}/"
    return
  fi

  mkdir -p "${PI_EXTENSION_DIR}"
  cp "${PI_PLUGIN_SRC}" "${PI_EXTENSION_DIR}/code-trace.ts"
  echo "Installed Pi Agent extension to ${PI_EXTENSION_DIR}/code-trace.ts"
}

if [ "${INSTALL_OPENCODE}" = true ]; then
  install_opencode_plugin
elif detect_opencode; then
  echo ""
  echo "OpenCode detected. Install the code-trace plugin?"
  echo "  ${OPENCODE_PLUGIN_DIR}/code-trace.ts"
  echo ""
  read -p "Install OpenCode plugin? [y/N] " -r
  if [[ "${REPLY}" =~ ^[Yy]$ ]]; then
    install_opencode_plugin
  fi
fi

if [ "${INSTALL_PI}" = true ]; then
  install_pi_extension
elif detect_pi; then
  echo ""
  echo "Pi Agent detected. Install the code-trace extension?"
  echo "  ${PI_EXTENSION_DIR}/code-trace.ts"
  echo ""
  read -p "Install Pi Agent extension? [y/N] " -r
  if [[ "${REPLY}" =~ ^[Yy]$ ]]; then
    install_pi_extension
  fi
fi

# Create config file if it does not already exist
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/code-trace"
CONFIG_FILE="${CONFIG_DIR}/config"

if [ -f "${CONFIG_FILE}" ]; then
  echo "Config file already exists: ${CONFIG_FILE}"
else
  mkdir -p "${CONFIG_DIR}"
  cat > "${CONFIG_FILE}" << 'EOF'
# code-trace configuration
# Set TRACE_TO_LANGFUSE=true and add your Langfuse keys to enable tracing.
TRACE_TO_LANGFUSE=false
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
# CODE_TRACE_DEBUG=false
EOF
  echo "Created config file: ${CONFIG_FILE}"
fi

echo ""
echo "Done! Edit ${CONFIG_FILE} to enable tracing:"
echo "  Set TRACE_TO_LANGFUSE=true and add your LANGFUSE_PUBLIC_KEY / LANGFUSE_SECRET_KEY."
echo ""
echo "Environment variables override the config file if you need per-project overrides."
