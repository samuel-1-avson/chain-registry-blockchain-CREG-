#!/usr/bin/env bash
# Enforces emergency containment rules for security-critical retired paths.
# This is intentionally dependency-free so it runs in a clean CI checkout.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

specs=(
  testnet/chain-spec.local.json
  testnet/chain-spec.sepolia.json
  testnet/spec-server/chain-spec.json
  testnet/spec-server/chain-spec.sepolia.json
  testnet/genesis.json
)

for spec in "${specs[@]}"; do
  [[ -f "$spec" ]] || { echo "missing required chain specification: $spec" >&2; exit 1; }
  if grep -Eq '"zk_validation"[[:space:]]*:[[:space:]]*true' "$spec"; then
    echo "retired ZK qualification is enabled in $spec" >&2
    exit 1
  fi
done

# The only retained contract entry points are deliberate reverts. This catches
# an accidental restoration before Foundry is available in CI.
if ! grep -q 'error ZKQualificationRetired' contracts/Registry.sol \
  || ! grep -q 'revert ZKQualificationRetired' contracts/Registry.sol; then
  echo 'Registry.sol no longer contains the ZK qualification retirement guard' >&2
  exit 1
fi

echo 'security freeze checks passed: ZK qualification is disabled in shipped specs and retired in Registry.sol'
