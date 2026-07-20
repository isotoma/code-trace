#!/usr/bin/env bash
set -euo pipefail

REPO="isotoma/code-trace"
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SETTINGS_FILE="${HOME}/.claude/settings.json"
OPENCODE_PLUGIN_DIR="${HOME}/.config/opencode/plugins"
PI_EXTENSION_DIR="${HOME}/.pi/agent/extensions"

detect_opencode() {
  [ -d "${HOME}/.config/opencode" ] || [ -f "${HOME}/.config/opencode/opencode.json" ]
}

detect_pi() {
  [ -d "${HOME}/.pi/agent" ]
}

# Resolve platform target triple into the global TARGET.
resolve_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "${arch}" in
    x86_64|amd64)  arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)
      echo "Unsupported architecture: ${arch}" >&2
      exit 1
      ;;
  esac

  case "${os}" in
    linux)  TARGET="${arch}-unknown-linux-musl" ;;
    darwin) TARGET="${arch}-apple-darwin" ;;
    *)
      echo "Unsupported OS: ${os}" >&2
      exit 1
      ;;
  esac
}

# Download the release asset, or fall back to a local build.
install_binary() {
  local asset download_url local_bin
  asset="${BINARY}-${TARGET}"

  download_url="$(curl -sfL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep "browser_download_url.*${asset}" \
    | head -1 \
    | cut -d '"' -f 4 || true)"

  if [ -z "${download_url}" ]; then
    local_bin="${SCRIPT_DIR}/target/release/${BINARY}"
    if [ -f "${local_bin}" ]; then
      echo "No release found; using local build: ${local_bin}"
      mkdir -p "${INSTALL_DIR}"
      cp "${local_bin}" "${INSTALL_DIR}/${BINARY}"
      chmod +x "${INSTALL_DIR}/${BINARY}"
      echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
    else
      echo "Could not find release asset for ${asset} and no local build found." >&2
      echo "Run 'cargo build --release' first, or check that the repo has a published release." >&2
      exit 1
    fi
  else
    echo "Downloading ${BINARY} for ${TARGET}..."
    mkdir -p "${INSTALL_DIR}"
    curl -sfL "${download_url}" -o "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"
    echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"
  fi
}

# Ensure ${INSTALL_DIR} is on PATH, adding it to the shell rc file if needed.
ensure_path() {
  if echo "${PATH}" | tr ':' '\n' | grep -qx "${INSTALL_DIR}"; then
    return
  fi

  local shell_name rc_file
  shell_name="$(basename "${SHELL:-bash}")"
  case "${shell_name}" in
    zsh)  rc_file="${HOME}/.zshrc" ;;
    *)    rc_file="${HOME}/.bashrc" ;;
  esac

  if [ -f "${rc_file}" ] && grep -q "${INSTALL_DIR}" "${rc_file}"; then
    echo "${INSTALL_DIR} already in ${rc_file} (not currently in PATH — restart your shell)"
  else
    echo "" >> "${rc_file}"
    echo "export PATH=\"${INSTALL_DIR}:\${PATH}\"" >> "${rc_file}"
    echo "Added ${INSTALL_DIR} to PATH in ${rc_file}"
    echo "Run: source ${rc_file} (or restart your shell)"
  fi
}

# Register (or migrate) the code-trace Stop hook in a Claude Code settings file.
#
# Idempotent and self-healing: any pre-existing Stop hook that invokes code-trace
# — whether by bare command, an absolute path, or the legacy
# ~/.claude/hooks/code-trace layout — is normalised to the canonical PATH-based
# `code-trace` command, and duplicates are collapsed to a single entry.
register_claude_code_hook() {
  local settings_file="${1:-${SETTINGS_FILE}}"
  mkdir -p "$(dirname "${settings_file}")"

  if python3 - "${settings_file}" <<'PYEOF'
import json, os, sys

path = sys.argv[1]
CANONICAL = {"type": "command", "command": "code-trace"}


def is_code_trace_command(cmd):
    """True if a hook command invokes the code-trace binary in any form.

    Matches the bare `code-trace`, an absolute/relative path such as
    `~/.claude/hooks/code-trace` or `/usr/local/bin/code-trace`, and any of
    these with trailing arguments.
    """
    if not isinstance(cmd, str):
        return False
    parts = cmd.strip().split()
    if not parts:
        return False
    return os.path.basename(parts[0]) == "code-trace"


try:
    with open(path) as f:
        settings = json.load(f)
    if not isinstance(settings, dict):
        settings = {}
except FileNotFoundError:
    settings = {}
except json.JSONDecodeError:
    sys.stderr.write("Existing settings file is not valid JSON; refusing to overwrite.\n")
    sys.exit(1)

hooks = settings.setdefault("hooks", {})
if not isinstance(hooks, dict):
    hooks = settings["hooks"] = {}

stop = hooks.setdefault("Stop", [])
if not isinstance(stop, list):
    stop = hooks["Stop"] = []

# Strip every existing code-trace hook from all Stop entries (migration + dedup).
for entry in stop:
    if isinstance(entry, dict) and isinstance(entry.get("hooks"), list):
        entry["hooks"] = [
            h for h in entry["hooks"]
            if not (isinstance(h, dict) and is_code_trace_command(h.get("command")))
        ]

# Add the canonical hook to the first matcher-style entry, or create one.
for entry in stop:
    if isinstance(entry, dict) and isinstance(entry.get("hooks"), list):
        entry["hooks"].append(dict(CANONICAL))
        break
else:
    stop.append({"hooks": [dict(CANONICAL)]})

with open(path, "w") as f:
    json.dump(settings, f, indent=2)
    f.write("\n")
PYEOF
  then
    echo "Registered code-trace Stop hook in ${settings_file}"
  else
    echo "Could not update ${settings_file} — please add the hook manually" >&2
    return 1
  fi
}

# Resolve plugin/extension source paths into globals.
resolve_plugin_sources() {
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
}

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

maybe_install_opencode() {
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
}

maybe_install_pi() {
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
}

# Create config file if it does not already exist
create_config() {
  local config_dir config_file
  config_dir="${XDG_CONFIG_HOME:-${HOME}/.config}/code-trace"
  config_file="${config_dir}/config"

  # Ask for an email to attach to traces as the Langfuse user id, unless one is
  # already configured. Read the prompt from the controlling terminal rather
  # than stdin, so a piped install (curl | bash) — whose stdin is the script
  # itself, not a TTY — can still prompt. Skip only when no terminal can be
  # opened at all (e.g. CI), so unattended installs neither hang nor consume
  # the piped script.
  local user_email="" tty_in=""
  if [ -t 0 ]; then
    tty_in="/dev/stdin"
  elif { : < /dev/tty; } 2>/dev/null; then
    tty_in="/dev/tty"
  fi

  if grep -Eq '^[[:space:]]*LANGFUSE_USER_ID[[:space:]]*=' "${config_file}" 2>/dev/null; then
    echo "LANGFUSE_USER_ID already configured in ${config_file} — leaving as-is"
  elif [ -n "${tty_in}" ]; then
    echo ""
    echo "Optionally attach your email to traces as the Langfuse user id"
    echo "(enables Langfuse's per-user views). Leave blank to skip."
    read -p "Email [skip]: " -r user_email < "${tty_in}" || user_email=""
    user_email="$(printf '%s' "${user_email}" | tr -d '[:space:]')"
  fi

  if [ -f "${config_file}" ]; then
    echo "Config file already exists: ${config_file}"
    if [ -n "${user_email}" ]; then
      printf 'LANGFUSE_USER_ID=%s\n' "${user_email}" >> "${config_file}"
      echo "Set LANGFUSE_USER_ID=${user_email} in ${config_file}"
    fi
  else
    mkdir -p "${config_dir}"
    cat > "${config_file}" << 'EOF'
# code-trace configuration
# Set TRACE_TO_LANGFUSE=true and add your Langfuse keys to enable tracing.
TRACE_TO_LANGFUSE=false
LANGFUSE_PUBLIC_KEY=pk-lf-...
LANGFUSE_SECRET_KEY=sk-lf-...
# LANGFUSE_BASE_URL=https://cloud.langfuse.com
EOF
    if [ -n "${user_email}" ]; then
      printf 'LANGFUSE_USER_ID=%s\n' "${user_email}" >> "${config_file}"
    else
      echo "# LANGFUSE_USER_ID=you@example.com" >> "${config_file}"
    fi
    echo "# CODE_TRACE_DEBUG=false" >> "${config_file}"
    echo "Created config file: ${config_file}"
    if [ -n "${user_email}" ]; then
      echo "Set LANGFUSE_USER_ID=${user_email}"
    fi
  fi

  echo ""
  echo "Done! Edit ${config_file} to enable tracing:"
  echo "  Set TRACE_TO_LANGFUSE=true and add your LANGFUSE_PUBLIC_KEY / LANGFUSE_SECRET_KEY."
  echo ""
  echo "Environment variables override the config file if you need per-project overrides."
}

main() {
  # Parse flags
  INSTALL_OPENCODE=false
  if [ "${1:-}" = "--opencode" ] || [ "${1:-}" = "-o" ]; then
    INSTALL_OPENCODE=true
  fi

  INSTALL_PI=false
  if [ "${1:-}" = "--pi" ] || [ "${1:-}" = "-p" ]; then
    INSTALL_PI=true
  fi

  resolve_target
  install_binary
  ensure_path
  resolve_plugin_sources

  if ! register_claude_code_hook "${SETTINGS_FILE}"; then
    : # message already printed; do not abort the rest of the install
  fi

  maybe_install_opencode
  maybe_install_pi
  create_config
}

# Run the installer unless sourced as a library (tests set CODE_TRACE_INSTALL_LIB=1).
if [ -z "${CODE_TRACE_INSTALL_LIB:-}" ]; then
  main "$@"
fi
