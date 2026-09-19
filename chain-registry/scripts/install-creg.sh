#!/usr/bin/env bash
# ==============================================================================
# Chain Registry (CREG) — Universal Remote Installer
# ==============================================================================
# Installs `creg` (CLI) and `creg-node` (Validator / Node Daemon).
#
# Quick remote install (Linux / macOS):
#   curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash
#
# Options:
#   --version <tag>      Install specific version (e.g., v0.4.4-testnet)
#   --install-dir <dir>  Target directory (default: $HOME/.creg/bin)
#   --build              Compile from local source using Cargo (requires Rust)
#   --docker             Install Docker wrapper shim for creg
#   --no-modify-path     Do not append PATH export to ~/.bashrc or ~/.zshrc
# ==============================================================================

set -euo pipefail

DEFAULT_VERSION="v0.4.4-testnet"
SPEC_HOST="https://spec.testnet.cregnet.dev"
GITHUB_REPO="${CREG_GITHUB_REPO:-samuel-1-avson/chain-registry-blockchain-CREG-}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.creg/bin}"
CONFIG_DIR="${CONFIG_DIR:-$HOME/.creg}"
VERSION="${DEFAULT_VERSION}"
BUILD=0
USE_DOCKER=0
MODIFY_PATH=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --build) BUILD=1; shift ;;
    --docker) USE_DOCKER=1; shift ;;
    --no-modify-path) MODIFY_PATH=0; shift ;;
    -h|--help)
      echo "CREG Installer Usage:"
      echo "  curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash"
      echo "  curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash -s -- --version v0.4.4-testnet"
      echo "  curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash -s -- --docker"
      echo "  ./scripts/install-creg.sh --build"
      exit 0
      ;;
    *) echo "Unknown argument: $1"; exit 1 ;;
  esac
done

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${CYAN}============================================================${NC}"
echo -e "${CYAN}   Chain Registry (CREG) — Universal Remote Installer       ${NC}"
echo -e "${CYAN}============================================================${NC}"

mkdir -p "$INSTALL_DIR"
mkdir -p "$CONFIG_DIR"

# ── Strategy 1: Build from source if requested ────────────────────────────────
if [[ "$BUILD" -eq 1 ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || echo "")"
  REPO_ROOT=""
  if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
  elif [[ -f "./Cargo.toml" ]]; then
    REPO_ROOT="$(pwd)"
  fi

  if [[ -z "$REPO_ROOT" ]]; then
    echo -e "${RED}Error: --build specified but not running inside the chain-registry source tree.${NC}" >&2
    exit 1
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo -e "${RED}Error: cargo toolchain is not installed. Install Rust from https://rustup.rs${NC}" >&2
    exit 1
  fi

  echo -e "${YELLOW}Compiling CREG from source (${REPO_ROOT})...${NC}"
  cd "$REPO_ROOT"
  export CREG_SPEC_SIGNING_PUBKEY="${CREG_SPEC_SIGNING_PUBKEY:-3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01}"
  cargo build --locked --release --package chain-registry-cli --package chain-registry-node
  install -m 755 target/release/creg target/release/creg-node "$INSTALL_DIR/"
  echo -e "${GREEN}✓ Successfully built and installed to $INSTALL_DIR${NC}"
  exit 0
fi

# ── Strategy 2: Docker Shim Wrapper ───────────────────────────────────────────
if [[ "$USE_DOCKER" -eq 1 ]]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not installed on this machine.${NC}" >&2
    exit 1
  fi
  cat << 'EOF' > "$INSTALL_DIR/creg"
#!/usr/bin/env bash
exec docker run --rm -it \
  -v "$HOME/.creg:/root/.creg" \
  -v "$(pwd):/work" \
  -w /work \
  ghcr.io/samuel-1-avson/chain-registry:latest creg "$@"
EOF
  chmod 755 "$INSTALL_DIR/creg"
  echo -e "${GREEN}✓ Installed Docker-based creg CLI shim to $INSTALL_DIR/creg${NC}"
  exit 0
fi

# ── Strategy 3: Download Precompiled Binary ───────────────────────────────────
ARCH="$(uname -m)"
OS="$(uname -s)"
TARBALL=""

case "$OS:$ARCH" in
  Linux:x86_64|Linux:amd64)
    TARBALL="chain-registry-${VERSION}-linux-amd64.tar.gz"
    ;;
  Darwin:x86_64|Darwin:amd64|Darwin:arm64)
    # macOS binary or fallback to docker
    TARBALL="chain-registry-${VERSION}-macos-amd64.tar.gz"
    ;;
  *)
    echo -e "${YELLOW}Notice: Direct binary not precompiled for $OS/$ARCH.${NC}"
    if command -v docker >/dev/null 2>&1; then
      echo -e "${GREEN}Detected Docker. Installing Docker CLI wrapper instead...${NC}"
      exec "$0" --docker
    else
      echo -e "${RED}Please install Docker or install Rust to build from source via 'cargo build'.${NC}"
      exit 1
    fi
    ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Primary: Testnet Spec Server CDN, Fallback: GitHub Releases
SPEC_URL="${SPEC_HOST}/dist/${TARBALL}"
GH_URL="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${TARBALL}"

echo -e "${CYAN}Fetching CREG ${VERSION} for ${OS}/${ARCH}...${NC}"

DOWNLOAD_SUCCESS=0
if curl -fsSL "$SPEC_URL" -o "$TMP/$TARBALL" 2>/dev/null; then
  echo -e "${GREEN}✓ Downloaded package from CREG Testnet CDN${NC}"
  DOWNLOAD_SUCCESS=1
  # Download checksums
  curl -fsSL "${SPEC_HOST}/dist/SHA256SUMS" -o "$TMP/SHA256SUMS" 2>/dev/null || true
elif curl -fsSL "$GH_URL" -o "$TMP/$TARBALL" 2>/dev/null; then
  echo -e "${GREEN}✓ Downloaded package from GitHub Releases${NC}"
  DOWNLOAD_SUCCESS=1
  curl -fsSL "https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/SHA256SUMS" -o "$TMP/SHA256SUMS" 2>/dev/null || true
fi

if [[ "$DOWNLOAD_SUCCESS" -ne 1 ]]; then
  echo -e "${YELLOW}Direct binary download not available for ${TARBALL}.${NC}"
  if command -v docker >/dev/null 2>&1; then
    echo -e "${CYAN}Falling back to Docker CLI wrapper installation...${NC}"
    exec "$0" --docker
  else
    echo -e "${RED}Failed to download ${TARBALL}. Check network connection or specify valid --version.${NC}"
    exit 1
  fi
fi

# Verify Checksum if SHA256SUMS is available
if [[ -f "$TMP/SHA256SUMS" ]]; then
  EXPECTED="$(awk -v asset="$TARBALL" '$2 == asset || $2 == "*" asset { print $1; exit }' "$TMP/SHA256SUMS" || true)"
  if [[ -n "$EXPECTED" && "$EXPECTED" =~ ^[[:xdigit:]]{64}$ ]]; then
    if command -v sha256sum >/dev/null 2>&1; then
      ACTUAL="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
      ACTUAL="$(shasum -a 256 "$TMP/$TARBALL" | awk '{print $1}')"
    else
      ACTUAL=""
    fi

    if [[ -n "$ACTUAL" ]]; then
      if [[ "${ACTUAL,,}" != "${EXPECTED,,}" ]]; then
        echo -e "${RED}Security Error: Checksum mismatch for $TARBALL!${NC}" >&2
        echo -e "Expected: $EXPECTED" >&2
        echo -e "Actual:   $ACTUAL" >&2
        exit 1
      fi
      echo -e "${GREEN}✓ SHA-256 integrity verified: ${ACTUAL}${NC}"
    fi
  fi
fi

# Extract binaries
tar -xzf "$TMP/$TARBALL" -C "$INSTALL_DIR"
chmod 755 "$INSTALL_DIR/creg"* 2>/dev/null || true

# Fetch testnet chain-spec so the user is immediately configured
echo -e "${CYAN}Syncing signed testnet chain specification...${NC}"
curl -fsSL "${SPEC_HOST}/chain-spec.sepolia.json" -o "$CONFIG_DIR/chain-spec.sepolia.json" 2>/dev/null || true
curl -fsSL "${SPEC_HOST}/chain-spec.sepolia.json.sig" -o "$CONFIG_DIR/chain-spec.sepolia.json.sig" 2>/dev/null || true

# Configure default config.toml pointing to public decentralized testnet
CONFIG_FILE="$CONFIG_DIR/config.toml"
if [[ ! -f "$CONFIG_FILE" ]]; then
  cat << 'EOF' > "$CONFIG_FILE"
[node]
url = "https://api.testnet.cregnet.dev"
timeout = 30

[defaults]
allow_unverified = false

[display]
colors = true
format = "Text"
progress = true

[ipfs]
url = "https://spec.testnet.cregnet.dev/ipfs"
timeout = 120
EOF
  echo -e "${GREEN}✓ Preconfigured public testnet endpoint (https://api.testnet.cregnet.dev)${NC}"
fi

# Check PATH & Environment
PATH_CONFIGURED=0
if [[ ":$PATH:" == *":$INSTALL_DIR:"* ]]; then
  PATH_CONFIGURED=1
fi

if [[ "$MODIFY_PATH" -eq 1 ]]; then
  SHELL_PROFILE=""
  if [[ -n "${ZSH_VERSION:-}" || "$SHELL" == *"zsh"* ]]; then
    SHELL_PROFILE="$HOME/.zshrc"
  elif [[ -n "${BASH_VERSION:-}" || "$SHELL" == *"bash"* ]]; then
    SHELL_PROFILE="$HOME/.bashrc"
  elif [[ -f "$HOME/.profile" ]]; then
    SHELL_PROFILE="$HOME/.profile"
  fi

  if [[ -n "$SHELL_PROFILE" && -w "$SHELL_PROFILE" ]]; then
    if ! grep -qs "$INSTALL_DIR" "$SHELL_PROFILE"; then
      echo -e "\n# CREG CLI PATH & Network" >> "$SHELL_PROFILE"
      echo -e "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_PROFILE"
      echo -e "export CREG_NODE_URL=\"https://api.testnet.cregnet.dev\"" >> "$SHELL_PROFILE"
      echo -e "${GREEN}✓ Added $INSTALL_DIR and CREG_NODE_URL to $SHELL_PROFILE${NC}"
      PATH_CONFIGURED=1
    fi
  fi
fi

echo ""
echo -e "${GREEN}============================================================${NC}"
echo -e "${GREEN}   Chain Registry (CREG) Installation Complete!             ${NC}"
echo -e "${GREEN}============================================================${NC}"
echo -e "Installed Binaries: ${INSTALL_DIR}/creg"
if [[ -f "$INSTALL_DIR/creg-node" ]]; then
  echo -e "                    ${INSTALL_DIR}/creg-node"
fi
echo -e "Testnet Spec:       ${CONFIG_DIR}/chain-spec.sepolia.json"
echo ""
if [[ "$PATH_CONFIGURED" -eq 0 ]]; then
  echo -e "${YELLOW}Please add the install directory to your PATH:${NC}"
  echo -e "  export PATH=\"$INSTALL_DIR:\$PATH\""
  echo ""
fi
echo -e "Get started:"
echo -e "  ${CYAN}creg --help${NC}                   View available commands"
echo -e "  ${CYAN}creg blocks${NC}                   View live blockchain blocks on testnet"
echo -e "  ${CYAN}creg keygen${NC}                   Create an Ed25519 identity keypair"
echo -e "  ${CYAN}creg init${NC}                     Interactive setup wizard"
echo -e "${GREEN}============================================================${NC}"
