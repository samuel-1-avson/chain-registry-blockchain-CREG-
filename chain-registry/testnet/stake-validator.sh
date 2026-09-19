#!/usr/bin/env bash
# ==============================================================================
# Chain Registry (CREG) — Sepolia Staking CLI (Linux/macOS)
# ==============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "${SCRIPT_DIR}/stake-validator.js" "$@"
