#!/usr/bin/env bash
# Chain Registry — Start Single Local Node (Linux/macOS/WSL)
# Usage: ./start-local-node.sh [-k VALIDATOR_KEY] [-n NODE_ID] [-b NODE_BINARY]
#
# This starts a single chain-registry node against Sepolia using the signed chain spec.
# No Docker required. Prefers the packaged release binaries; cargo is a
# developer-only fallback when no release binary is available.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VALIDATOR_KEY=""
NODE_ID="local-test"
DATA_DIR="$SCRIPT_DIR/node-data"
RPC_URL="https://ethereum-sepolia-rpc.publicnode.com"
VALIDATOR=false
NODE_BINARY="${CREG_NODE_BINARY:-}"

usage() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -k, --key KEY       Validator private key (hex, 64 chars)"
    echo "  -n, --node-id ID    Node identifier (default: local-test)"
    echo "  -d, --data-dir DIR  Data directory (default: ./node-data)"
    echo "  -r, --rpc URL       Sepolia RPC URL (default: publicnode)"
    echo "  -b, --node-binary P Use a packaged creg-node binary"
    echo "  -v, --validator     Enable validator mode"
    echo "  -h, --help          Show this help"
    exit 0
}

while [[ $# -gt 0 ]]; do
    case $1 in
        -k|--key) VALIDATOR_KEY="$2"; shift 2 ;;
        -n|--node-id) NODE_ID="$2"; shift 2 ;;
        -d|--data-dir) DATA_DIR="$2"; shift 2 ;;
        -r|--rpc) RPC_URL="$2"; shift 2 ;;
        -b|--node-binary) NODE_BINARY="$2"; shift 2 ;;
        -v|--validator) VALIDATOR=true; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

CLI_BINARY="${CREG_CLI_BINARY:-}"
if [[ -n "$NODE_BINARY" && ! -x "$NODE_BINARY" ]]; then
    echo "ERROR: Node binary is not executable: $NODE_BINARY"
    exit 1
fi
if [[ -z "$NODE_BINARY" ]] && command -v creg-node >/dev/null 2>&1; then
    NODE_BINARY="$(command -v creg-node)"
fi
if [[ -z "$CLI_BINARY" ]] && command -v creg >/dev/null 2>&1; then
    CLI_BINARY="$(command -v creg)"
fi
if [[ -n "$NODE_BINARY" && -z "$CLI_BINARY" ]]; then
    candidate="$(dirname "$NODE_BINARY")/creg"
    [[ -x "$candidate" ]] && CLI_BINARY="$candidate"
fi

echo "═══════════════════════════════════════════════════════"
echo "  Chain Registry — Local Node Starter"
echo "═══════════════════════════════════════════════════════"

# [1/5] Check prerequisites
echo ""
echo "[1/5] Checking prerequisites..."

if ! command -v cargo &> /dev/null && [[ -z "$NODE_BINARY" ]]; then
    echo "ERROR: Rust/Cargo not found. Install from https://rustup.rs/"
    exit 1
fi
command -v cargo &> /dev/null && echo "  ✓ Cargo found: $(cargo --version)"
[[ -n "$NODE_BINARY" ]] && echo "  ✓ Release node: $NODE_BINARY"
[[ -n "$CLI_BINARY" ]] && echo "  ✓ Release CLI:  $CLI_BINARY"

SPEC_PATH="$REPO_ROOT/testnet/chain-spec.sepolia.json"
SIG_PATH="$REPO_ROOT/testnet/chain-spec.sepolia.json.sig"

if [[ ! -f "$SPEC_PATH" ]]; then
    echo "ERROR: Chain spec not found: $SPEC_PATH"
    exit 1
fi
echo "  ✓ Chain spec found"

if [[ ! -f "$SIG_PATH" ]]; then
    echo "ERROR: Signature file not found: $SIG_PATH. Refusing to start without a signed chain spec."
    exit 1
fi

mkdir -p "$DATA_DIR"
echo "  ✓ Data directory: $(cd "$DATA_DIR" && pwd)"

# [2/5] Configure environment
echo ""
echo "[2/5] Configuring environment..."

export CREG_CHAIN_SPEC_URL="file://$SPEC_PATH"
export CREG_SPEC_SIGNING_PUBKEY="3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01"
export CREG_ETH_RPC_URL="$RPC_URL"
export CREG_NODE_ID="$NODE_ID"
export CREG_DATA_DIR="$(cd "$DATA_DIR" && pwd)"
export CREG_LISTEN="0.0.0.0:8080"
export CREG_P2P_LISTEN="/ip4/0.0.0.0/tcp/9000"
export CREG_P2P_SEEDS=""   # Empty = you are the bootstrap
export RUST_LOG="info,chain_registry_node=debug"

if [[ "$VALIDATOR" == true ]]; then
    export CREG_IS_VALIDATOR="true"
    if [[ -n "$VALIDATOR_KEY" ]]; then
        export CREG_VALIDATOR_KEY="$VALIDATOR_KEY"
        echo "  ✓ Validator mode ENABLED (key provided)"
    else
        echo "ERROR: Validator mode requires --key matching an identity in the signed chain spec."
        exit 1
    fi
else
    export CREG_IS_VALIDATOR="false"
    echo "  ✓ Full node mode (non-validator)"
fi

echo "  ✓ Chain spec: $CREG_CHAIN_SPEC_URL"
echo "  ✓ L1 RPC: $RPC_URL"
echo "  ✓ Node ID: $NODE_ID"

# [3/5] Verify chain spec
echo ""
echo "[3/5] Verifying chain spec..."
SIG=$(tr -d ' \n\r' < "$SIG_PATH")
if [[ ! "$SIG" =~ ^[0-9a-fA-F]{128}$ ]]; then
    echo "ERROR: Detached signature is not 64-byte hexadecimal"
    exit 1
fi
if [[ -n "$CLI_BINARY" ]]; then
    "$CLI_BINARY" chain-spec validate "$SPEC_PATH" --signature "$SIG_PATH" --pubkey "$CREG_SPEC_SIGNING_PUBKEY" --require-signature >/dev/null
elif ! cargo run --locked --example verify_chain_spec --package common --quiet -- "$SPEC_PATH" "$SIG" 2>/dev/null; then
    echo "ERROR: Chain spec signature verification failed; refusing to start"
    exit 1
fi
echo "  ✓ Chain spec signature VALID"

# [4/5] Compute genesis hash
echo ""
echo "[4/5] Computing genesis hash..."
if [[ -n "$CLI_BINARY" ]]; then
    GENESIS_HASH=$("$CLI_BINARY" chain-spec compute-genesis-hash "$SPEC_PATH" 2>/dev/null)
else
    GENESIS_HASH=$(cargo run --locked --example compute_genesis_hash --package common --quiet -- "$SPEC_PATH" 2>/dev/null)
fi
if [[ -z "$GENESIS_HASH" || "$GENESIS_HASH" == "unknown" ]]; then
    echo "ERROR: Could not compute genesis hash; refusing to start"
    exit 1
fi
export CREG_GENESIS_HASH="$(printf '%s\n' "$GENESIS_HASH" | tail -n 1 | tr -d ' \r')"
echo "  ✓ Genesis hash: $GENESIS_HASH"

# [5/5] Start node
echo ""
echo "[5/5] Starting node..."
echo "  Press Ctrl+C to stop"
echo ""
echo "  API:     http://localhost:8080/v1/health"
echo "  Metrics: http://localhost:8080/metrics"
echo "  gRPC:    localhost:50051"
echo "  P2P:     localhost:9000"
echo ""

cd "$REPO_ROOT"
if [[ -n "$NODE_BINARY" ]]; then
    exec "$NODE_BINARY"
else
    exec cargo run --locked --bin creg-node --package chain-registry-node
fi
