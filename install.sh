#!/bin/bash
set -euo pipefail

REPO="LarryStanley/kabel"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) OS="apple-darwin" ;;
  linux) OS="unknown-linux-gnu" ;;
  *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

TARGET="${ARCH}-${OS}"

LATEST=$(curl -s "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)
if [ -z "$LATEST" ]; then
  echo "Could not determine latest release"
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${LATEST}/kabel-${TARGET}.tar.gz"

echo "Installing kabel ${LATEST} for ${TARGET}..."

TMP=$(mktemp -d)
curl -sL "$URL" -o "${TMP}/kabel.tar.gz"
tar xzf "${TMP}/kabel.tar.gz" -C "${TMP}"

if [ -w "$INSTALL_DIR" ]; then
  mv "${TMP}/kabel" "${INSTALL_DIR}/kabel"
else
  sudo mv "${TMP}/kabel" "${INSTALL_DIR}/kabel"
fi

rm -rf "$TMP"

echo "kabel ${LATEST} installed to ${INSTALL_DIR}/kabel"
echo "Run 'kabel --help' to get started"
