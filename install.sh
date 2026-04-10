#!/usr/bin/env bash
set -euo pipefail

REPO="isotoma/code-trace"
BINARY="code-trace"
INSTALL_DIR="${HOME}/.local/bin"

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
echo ""
echo "Make sure ${INSTALL_DIR} is in your PATH."
echo ""
echo "Add to your Claude Code hooks (~/.claude/settings.json):"
echo ""
cat << 'HOOKEOF'
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "command",
        "command": "code-trace"
      }]
    }]
  }
}
HOOKEOF
