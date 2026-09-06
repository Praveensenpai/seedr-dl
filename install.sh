#!/usr/bin/env bash
set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[1;32m'
RED='\033[0;31m'
BOLD='\033[1m'
RESET='\033[0m'

echo -e "${BOLD}${CYAN}⚡ seedr-dl installer${RESET}"

# Detect OS/arch
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}-${ARCH}" in
  Linux-x86_64)
    TARGET="x86_64-unknown-linux-musl" ;;
  Darwin-arm64)
    TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64)
    TARGET="x86_64-apple-darwin" ;;
  *)
    echo -e "${RED}✖ Unsupported platform: ${OS}-${ARCH}${RESET}"
    exit 1 ;;
esac

DEST_DIR="${HOME}/.local/bin"
DEST="${DEST_DIR}/seedr-dl"

LATEST=$(curl -LsSf "https://api.github.com/repos/Praveensenpai/seedr-dl/releases/latest" 2>/dev/null \
  | grep '"tag_name":' | head -1 | cut -d'"' -f4 || true)

if [ -z "${LATEST}" ]; then
  echo -e "${RED}✖ Could not fetch latest release from GitHub API.${RESET}"
  exit 1
fi

echo -e "${CYAN}  • Latest release : ${BOLD}${LATEST}${RESET}"
echo -e "${CYAN}  • Target         : ${TARGET}${RESET}"
echo -e "${CYAN}  • Installing to  : ${DEST}${RESET}\n"

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT INT TERM

TAR_NAME="seedr-dl-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/Praveensenpai/seedr-dl/releases/download/${LATEST}/${TAR_NAME}"
FALLBACK_URL="https://github.com/Praveensenpai/seedr-dl/releases/download/${LATEST}/seedr-dl-${LATEST}-${TARGET}.tar.gz"

echo -e "${CYAN}  • Downloading...${RESET}"
if curl -LsSf -H 'Cache-Control: no-cache' "${DOWNLOAD_URL}" -o "${TMP_DIR}/archive.tar.gz" 2>/dev/null; then
  echo -e "${CYAN}  • Extracting...${RESET}"
  tar -xzf "${TMP_DIR}/archive.tar.gz" -C "${TMP_DIR}"
elif curl -LsSf -H 'Cache-Control: no-cache' "${FALLBACK_URL}" -o "${TMP_DIR}/archive.tar.gz" 2>/dev/null; then
  echo -e "${CYAN}  • Extracting...${RESET}"
  tar -xzf "${TMP_DIR}/archive.tar.gz" -C "${TMP_DIR}"
else
  # Fallback to direct raw binary if release only has uncompressed binary
  RAW_URL="https://github.com/Praveensenpai/seedr-dl/releases/download/${LATEST}/seedr-dl-${LATEST}-${TARGET}"
  if curl -LsSf -H 'Cache-Control: no-cache' "${RAW_URL}" -o "${TMP_DIR}/seedr-dl" 2>/dev/null; then
    :
  else
    echo -e "${RED}✖ Download failed. Check your connection or try: cargo install --git https://github.com/Praveensenpai/seedr-dl${RESET}"
    exit 1
  fi
fi

if [ ! -f "${TMP_DIR}/seedr-dl" ]; then
  echo -e "${RED}✖ Failed to find 'seedr-dl' executable inside extracted archive.${RESET}"
  exit 1
fi

mkdir -p "${DEST_DIR}"
chmod +x "${TMP_DIR}/seedr-dl"
mv -f "${TMP_DIR}/seedr-dl" "${DEST}"

echo -e "${GREEN}✔ seedr-dl ${LATEST} installed!${RESET}"

# Ensure ~/.local/bin is on PATH
if ! echo "${PATH}" | grep -q "${HOME}/.local/bin"; then
  echo -e "\n${CYAN}  ℹ  Add this to your shell profile (~/.bashrc / ~/.zshrc):${RESET}"
  echo -e "     ${BOLD}export PATH=\"\$HOME/.local/bin:\$PATH\"${RESET}"
fi

echo -e "\n${BOLD}  Run: seedr-dl${RESET}"
