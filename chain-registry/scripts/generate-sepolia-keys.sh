#!/usr/bin/env bash
# scripts/generate-sepolia-keys.sh
# Generate fresh keys for Sepolia testnet participation.
#
# Usage:
#   ./scripts/generate-sepolia-keys.sh [--out-dir ./validator-keys]

set -euo pipefail

OUT_DIR="${1:-./validator-keys}"
mkdir -p "$OUT_DIR"

# ── Ed25519 validator key ──
# If creg binary is available, use it. Otherwise generate with openssl.
if command -v creg &>/dev/null; then
    creg keygen --out "$OUT_DIR/validator.key"
    VALIDATOR_PUBKEY=$(creg keygen --show-pubkey "$OUT_DIR/validator.key" 2>/dev/null || echo "")
else
    # Fallback: generate Ed25519 key with openssl
    openssl genpkey -algorithm Ed25519 -out "$OUT_DIR/validator.key" 2>/dev/null
    VALIDATOR_PUBKEY=$(openssl pkey -in "$OUT_DIR/validator.key" -pubout -outform DER 2>/dev/null | tail -c 32 | xxd -p -c 64 || echo "")
fi

# ── secp256k1 bridge key ──
if command -v cast &>/dev/null; then
    cast wallet new > "$OUT_DIR/bridge.key"
    BRIDGE_ADDR=$(cast wallet address --private-key-file "$OUT_DIR/bridge.key")
else
    # Fallback: generate with openssl
    openssl ecparam -name secp256k1 -genkey -noout -out "$OUT_DIR/bridge.key" 2>/dev/null
    BRIDGE_ADDR="(install cast to derive address)"
fi

echo "=== Sepolia Key Generation Complete ==="
echo ""
echo "Validator key: $OUT_DIR/validator.key"
if [ -n "$VALIDATOR_PUBKEY" ]; then
    echo "Validator pubkey (Ed25519): $VALIDATOR_PUBKEY"
fi
echo "Bridge key:    $OUT_DIR/bridge.key"
echo "Bridge address: $BRIDGE_ADDR"
echo ""
echo "NEXT STEPS:"
echo "  1. Back up $OUT_DIR to a password manager or HSM."
echo "  2. Fund the bridge address ($BRIDGE_ADDR) with SepoliaETH."
echo "  3. Set CREG_VALIDATOR_KEY and CREG_BRIDGE_KEY in your .env.sepolia"
echo "  4. NEVER commit these keys to git."
