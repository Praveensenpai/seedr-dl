#!/usr/bin/env bash
set -e

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

LATEST=$(curl -LsSf "https://api.github.com/repos/Praveensenpai/seedr-dl/releases/latest" \
  | grep '"tag_name"' | head -1 | cut -d'"' -f4)

BIN_URL="https://github.com/Praveensenpai/seedr-dl/releases/download/${LATEST}/seedr-dl-${LATEST}-${TARGET}"
DEST="${HOME}/.local/bin/seedr-dl"

echo -e "${CYAN}  • Latest release : ${BOLD}${LATEST}${RESET}"
echo -e "${CYAN}  • Target          : ${TARGET}${RESET}"
echo -e "${CYAN}  • Installing to   : ${DEST}${RESET}\n"

mkdir -p "$(dirname "${DEST}")"

if curl -LsSf -H 'Cache-Control: no-cache' "${BIN_URL}" -o "${DEST}.tmp"; then
  chmod +x "${DEST}.tmp"
  mv -f "${DEST}.tmp" "${DEST}"
  echo -e "${GREEN}✔ seedr-dl ${LATEST} installed!${RESET}"
else
  echo -e "${RED}✖ Download failed. Check your connection or try: cargo install --git https://github.com/Praveensenpai/seedr-dl${RESET}"
  exit 1
fi

# Ensure ~/.local/bin is on PATH
if ! echo "${PATH}" | grep -q "${HOME}/.local/bin"; then
  echo -e "\n${CYAN}  ℹ  Add this to your shell profile (~/.bashrc / ~/.zshrc):${RESET}"
  echo -e "     ${BOLD}export PATH=\"\$HOME/.local/bin:\$PATH\"${RESET}"
fi

echo -e "\n${BOLD}  Run: seedr-dl${RESET}"
