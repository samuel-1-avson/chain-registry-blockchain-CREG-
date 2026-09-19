#!/usr/bin/env bash
# ==============================================================================
# Chain Registry (CREG) — Dual-Signed Validator Registration (Linux/macOS)
# ==============================================================================

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
node "${SCRIPT_DIR}/register-validator-identity.js" "$@"
