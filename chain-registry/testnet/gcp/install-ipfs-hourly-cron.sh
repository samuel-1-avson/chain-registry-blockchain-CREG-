#!/usr/bin/env bash
# Install hourly IPFS pin + availability check on the edge VM (IPFS-002).
#
# Run on creg-testnet-vm after ipfs-pin-check.py is deployed to ~/creg-pin-check/.
#
# Usage:
#   bash testnet/gcp/install-ipfs-hourly-cron.sh
#   CREG_API_URL=https://api.testnet.cregnet.dev bash testnet/gcp/install-ipfs-hourly-cron.sh

set -euo pipefail

API_URL="${CREG_API_URL:-https://api.testnet.cregnet.dev}"
IPFS_API="${CREG_IPFS_API:-http://localhost:15001}"
INSTALL_DIR="${CREG_PIN_INSTALL_DIR:-$HOME/creg-pin-check}"
LOG_FILE="${CREG_PIN_LOG:-$HOME/creg-pin-check/pin-check.log}"
REPORT_DIR="${CREG_PIN_REPORT_DIR:-$HOME/creg-pin-check/reports}"

mkdir -p "$INSTALL_DIR/reports"
SCRIPT="$INSTALL_DIR/ipfs-pin-check.py"
if [[ ! -f "$SCRIPT" ]]; then
  echo "Missing $SCRIPT — upload testnet/ipfs-pin-check.py first (run-ipfs-pin-check.ps1)" >&2
  exit 1
fi

CRON_LINE="0 * * * * CREG_API_URL=$API_URL CREG_IPFS_API=$IPFS_API CREG_PIN_REPORT_DIR=$REPORT_DIR /usr/bin/python3 $SCRIPT >> $LOG_FILE 2>&1"

TMP="$(mktemp)"
crontab -l 2>/dev/null | grep -v 'ipfs-pin-check.py' | grep -v 'creg-pin-check' >"$TMP" || true
echo "$CRON_LINE" >>"$TMP"
crontab "$TMP"
rm -f "$TMP"

echo "Installed hourly cron:"
echo "  $CRON_LINE"
crontab -l | grep ipfs-pin-check || true
