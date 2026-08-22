#!/usr/bin/env bash
# Install creg + creg-node from GitHub Releases or build from source.
#
# Usage:
#   ./scripts/install-creg.sh
#   ./scripts/install-creg.sh --version v0.4.3-testnet
#   ./scripts/install-creg.sh --build

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.creg/bin}"
VERSION=""
BUILD=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --build) BUILD=1; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

mkdir -p "$INSTALL_DIR"

if [[ "$BUILD" -eq 1 ]]; then
  cd "$ROOT"
  export CREG_SPEC_SIGNING_PUBKEY="${CREG_SPEC_SIGNING_PUBKEY:-3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01}"
  cargo build --locked --release --package chain-registry-cli --package chain-registry-node
  install -m 755 target/release/creg target/release/creg-node "$INSTALL_DIR/"
  echo "Installed to $INSTALL_DIR"
  exit 0
fi

if [[ -z "$VERSION" ]]; then
  echo "No --version; building from source (publish a GitHub release to download binaries)."
  exec "$0" --build
fi

ARCH="$(uname -m)"
OS="$(uname -s)"
case "$OS:$ARCH" in
  Linux:x86_64|Linux:amd64) TARBALL="chain-registry-${VERSION}-linux-amd64.tar.gz" ;;
  Darwin:x86_64|Darwin:amd64) TARBALL="chain-registry-${VERSION}-macos-amd64.tar.gz" ;;
  *) echo "Unsupported platform: $OS/$ARCH"; exit 1 ;;
esac

# Override when releases live on a fork: export CREG_GITHUB_REPO=owner/repo
GITHUB_REPO="${CREG_GITHUB_REPO:-samuel-1-avson/chain-registry-blockchain-CREG-}"
URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${TARBALL}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$URL" -o "$TMP/$TARBALL"
curl -fsSL "https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/SHA256SUMS" \
  -o "$TMP/SHA256SUMS"

EXPECTED="$(awk -v asset="$TARBALL" '$2 == asset || $2 == "*" asset { print $1; exit }' "$TMP/SHA256SUMS")"
if [[ ! "$EXPECTED" =~ ^[[:xdigit:]]{64}$ ]]; then
  echo "SHA256SUMS does not contain an entry for $TARBALL" >&2
  exit 1
fi
ACTUAL="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
if [[ "${ACTUAL,,}" != "${EXPECTED,,}" ]]; then
  echo "Checksum mismatch for $TARBALL (expected $EXPECTED, got $ACTUAL)" >&2
  exit 1
fi
echo "SHA-256 verified: $ACTUAL"
tar -xzf "$TMP/$TARBALL" -C "$INSTALL_DIR"
echo "Installed creg + creg-node to $INSTALL_DIR"
echo "export PATH=\"$INSTALL_DIR:\$PATH\""
