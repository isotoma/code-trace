#!/usr/bin/env bash
set -euo pipefail

REPO="isotoma/code-trace"
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SETTINGS_FILE="${HOME}/.claude/settings.json"

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
# Delegates to the installed binary (`code-trace setup --register-hook`), which
# owns the JSON logic: any pre-existing Stop hook that invokes code-trace —
# whether by bare command, an absolute path, or the legacy
# ~/.claude/hooks/code-trace layout — is normalised to the canonical PATH-based
# `code-trace` command, and duplicates are collapsed to a single entry. Keeping
# this in the binary means it is unit-tested and needs no python3 interpreter.
register_claude_code_hook() {
  local settings_file="${1:-${SETTINGS_FILE}}"

  if "${INSTALL_DIR}/${BINARY}" setup --register-hook --settings-file "${settings_file}"; then
    : # the binary prints its own confirmation line
  else
    echo "Could not update ${settings_file} — please add the hook manually" >&2
    return 1
  fi
}

# Install the OpenCode plugin (forced with --opencode, otherwise offered when
# OpenCode is detected). The binary owns detection, the prompt, and the embedded
# plugin source, so this works under curl | bash with no local checkout.
maybe_install_opencode() {
  if [ "${INSTALL_OPENCODE}" = true ]; then
    "${INSTALL_DIR}/${BINARY}" setup --install-opencode || true
  else
    "${INSTALL_DIR}/${BINARY}" setup --offer-opencode || true
  fi
}

# Install the Pi Agent extension (forced with --pi, otherwise offered when Pi is
# detected). As above, the binary owns detection, the prompt, and the source.
maybe_install_pi() {
  if [ "${INSTALL_PI}" = true ]; then
    "${INSTALL_DIR}/${BINARY}" setup --install-pi || true
  else
    "${INSTALL_DIR}/${BINARY}" setup --offer-pi || true
  fi
}

# Create the config file, via the binary. `setup --write-config` also offers to
# set the Langfuse user id from your email; it reads the terminal itself, so a
# piped curl | bash install can prompt without the shell consuming its script.
create_config() {
  local config_file="${XDG_CONFIG_HOME:-${HOME}/.config}/code-trace/config"

  "${INSTALL_DIR}/${BINARY}" setup --write-config || true

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
