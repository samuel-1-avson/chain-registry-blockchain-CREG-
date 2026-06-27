#!/usr/bin/env bash
# Run on GCP edge VM after sync-local-repo + waitlist dist present.
# Waitlist-only mode: does not start explorer, hub, faucet, or full fleet Caddy.
set -euo pipefail

REPO_ROOT="${1:-$HOME/creg-hosting/chain-registry-blockchain-CREG-/chain-registry}"
exec bash "$REPO_ROOT/testnet/gcp/start-waitlist-edge-only.sh" "$REPO_ROOT"
