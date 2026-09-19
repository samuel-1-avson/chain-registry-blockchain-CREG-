#!/usr/bin/env bash
# scripts/test-relayer-soak.sh
# Automated contract and state root relayer soak drill
# Anchors finalized PBFT state roots to Sepolia contracts using L1RelayerService

set -euo pipefail

COUNT="${1:-100}"
START_HEIGHT="${2:-1}"
RPC_URL="${CREG_SEPOLIA_RPC_URL:-https://ethereum-sepolia-rpc.publicnode.com}"
CONTRACT_ADDR="${CREG_L1_REGISTRY_ADDRESS:-0xF232cC6F5C6EfC74C7517B209D15baBd57b817eE}"
DRY_RUN="${DRY_RUN:-true}"

echo "================================================================================"
echo "  CREG L1 CONTRACT & STATE ROOT RELAYER SOAK TEST DRILL (Sepolia)"
echo "================================================================================"
echo "  Contract:      ${CONTRACT_ADDR}"
echo "  Sepolia RPC:   ${RPC_URL}"
echo "  Checkpoints:   ${COUNT} batches"
echo "  Start Height:  #${START_HEIGHT}"
echo "  Dry Run:       ${DRY_RUN}"
echo "================================================================================"

cargo run --bin creg -- relayer soak \
    --count "${COUNT}" \
    --start-height "${START_HEIGHT}" \
    --rpc-url "${RPC_URL}" \
    --contract "${CONTRACT_ADDR}" \
    --dry-run "${DRY_RUN}"

echo "✓ L1 Relayer Soak Drill completed successfully."
