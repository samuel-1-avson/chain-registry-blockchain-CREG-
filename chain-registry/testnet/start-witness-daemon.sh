#!/usr/bin/env bash
# ==============================================================================
# CREG Transparency Log Witness Daemon (WITNESSED-LOG Track B)
#
# Runs an independent transparency log witness.
# Continuously fetches signed tree heads, checks Merkle consistency proofs,
# and signs WitnessedTreeHead for client split-view protection.
#
# Usage:
#   ./start-witness-daemon.sh
#   ./start-witness-daemon.sh --once
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

NODE_URL="${CREG_NODE_URL:-https://api.testnet.cregnet.dev}"
LOG_ID="${CREG_TRANSPARENCY_LOG_ID:-creg-transparency-log-1}"
LOG_PUBKEY="${CREG_TRANSPARENCY_LOG_PUBKEY:-3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01}"
WITNESS_ID="${CREG_WITNESS_ID:-witness-alpha-independent}"

DATA_DIR="${HOME}/.creg/witness"
mkdir -p "${DATA_DIR}"

KEY_PATH="${CREG_WITNESS_KEY:-${DATA_DIR}/witness.key}"
STATE_PATH="${DATA_DIR}/witness-state.json"
OUT_PATH="${DATA_DIR}/witnessed-head.json"

echo "============================================================"
echo " CREG Transparency Witness Daemon (Track B)"
echo "============================================================"
echo "Witness ID       : ${WITNESS_ID}"
echo "Node URL         : ${NODE_URL}"
echo "Log ID           : ${LOG_ID}"
echo "Log Public Key   : ${LOG_PUBKEY}"
echo "Key Path         : ${KEY_PATH}"
echo "State File       : ${STATE_PATH}"
echo "Output Head      : ${OUT_PATH}"
echo "============================================================"

# Ensure witness key exists
if [ ! -f "${KEY_PATH}" ]; then
  echo "[INFO] Witness key not found. Generating fresh Ed25519 witness key..."
  cargo run --quiet --package chain-registry-cli --bin creg -- keygen witness --key-path "${KEY_PATH}"
  echo "[OK] Generated witness key at ${KEY_PATH}"
fi

EXTRA_FLAGS=()
if [[ "${1:-}" == "--once" ]]; then
  EXTRA_FLAGS+=("--once")
fi

echo "[INFO] Launching creg witness daemon..."
exec cargo run --quiet --package chain-registry-cli --bin creg -- witness run \
  --node-url "${NODE_URL}" \
  --log-id "${LOG_ID}" \
  --log-pubkey "${LOG_PUBKEY}" \
  --witness-id "${WITNESS_ID}" \
  --key-path "${KEY_PATH}" \
  --state-path "${STATE_PATH}" \
  --out-path "${OUT_PATH}" \
  --interval-secs 30 \
  "${EXTRA_FLAGS[@]}"
