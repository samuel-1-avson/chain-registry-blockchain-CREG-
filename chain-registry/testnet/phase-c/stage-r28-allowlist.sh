#!/usr/bin/env bash
# RPC remediation Phase 1, staging (docs/rpc-remediation-plan.md).
#
# Starts a copy of the live explorer (creg-explorer-public-r28) whose /rpc gateway
# points at a real Sepolia RPC instead of the edge, on 127.0.0.1 only, and checks
# the ECA-38 allowlist matrix against it. The live explorer, Caddy and every other
# container are left untouched; the copy receives no public traffic.
#
# Usage (on creg-testnet-vm):
#   bash stage-r28-allowlist.sh UPSTREAM_URL UPSTREAM_HOST [PORT]
#   e.g. bash stage-r28-allowlist.sh https://ethereum-sepolia-rpc.publicnode.com/ ethereum-sepolia-rpc.publicnode.com 3031
#
# Remove the copy with: docker rm -f creg-explorer-public-r28-allowlist-stage
set -uo pipefail

upstream="${1:?upstream URL, including its path (for example a trailing /)}"
upstream_host="${2:?Host header for the upstream}"
port="${3:-3031}"
src=creg-explorer-public-r28
stage=creg-explorer-public-r28-allowlist-stage
network=testnet_creg-unified-net

DOCKER=(docker)
docker info >/dev/null 2>&1 || DOCKER=(sudo docker)

case "$upstream" in
  *api.testnet.cregnet.dev*|*rpc.testnet.cregnet.dev*|*explorer.testnet.cregnet.dev*|*faucet.testnet.cregnet.dev*)
    echo "refusing: the upstream must be a real Sepolia RPC, never the CREG edge (it would loop)" >&2
    exit 2 ;;
esac

image=$("${DOCKER[@]}" inspect -f '{{.Config.Image}}' "$src") || { echo "no $src container" >&2; exit 1; }
if ! "${DOCKER[@]}" run --rm --entrypoint test "$image" -f /etc/nginx/njs/rpc-allowlist.js; then
  echo "refusing: $image has no rpc-allowlist.js" >&2
  exit 1
fi

# Copy the source container's CREG_* runtime settings except the two being replaced.
mapfile -t envs < <("${DOCKER[@]}" inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$src" \
  | grep -E '^CREG_' | grep -vE '^CREG_SEPOLIA_RPC_(UPSTREAM|HOST)=')
args=()
for entry in "${envs[@]}"; do
  [[ -n "$entry" ]] && args+=(-e "$entry")
done

"${DOCKER[@]}" rm -f "$stage" >/dev/null 2>&1 || true
"${DOCKER[@]}" run -d --name "$stage" --restart no --network "$network" \
  -p "127.0.0.1:${port}:80" "${args[@]}" \
  -e "CREG_SEPOLIA_RPC_UPSTREAM=$upstream" -e "CREG_SEPOLIA_RPC_HOST=$upstream_host" \
  "$image" >/dev/null || { echo "could not start $stage" >&2; exit 1; }

echo "image $image"
echo "copied settings: $(printf '%s\n' "${envs[@]}" | cut -d= -f1 | paste -sd' ' -)"
echo "upstream $(printf '%s' "$upstream" | sed -E 's#^(https?://[^/]+).*#\1#')"

base="http://127.0.0.1:${port}"
for _ in $(seq 1 20); do
  curl -fsS -o /dev/null "$base/" 2>/dev/null && break
  sleep 1
done

body=/tmp/creg-stage-body.$$
trap 'rm -f "$body"' EXIT
fail=0
check() {
  local name=$1 want=$2 got=$3 pattern=${4:-}
  if [[ "$got" == "$want" ]] && { [[ -z "$pattern" ]] || grep -q -- "$pattern" "$body"; }; then
    echo "PASS $name ($got)"
  else
    echo "FAIL $name (got $got, want $want${pattern:+ and /$pattern/}) $(head -c 160 "$body" | tr -d '\n')"
    fail=1
  fi
}
post() { curl -s -o "$body" -w '%{http_code}' -X POST "$base/rpc" -H 'content-type: application/json' --data-binary @-; }

check "GET / (explorer page)" 200 "$(curl -s -o "$body" -w '%{http_code}' "$base/")"
check "GET /v1/public/health (node via explorer)" 200 "$(curl -s -o "$body" -w '%{http_code}' "$base/v1/public/health")"
check "GET /rpc refused" 405 "$(curl -s -o "$body" -w '%{http_code}' "$base/rpc")"
check "eth_chainId" 200 "$(printf '%s' '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' | post)" '0xaa36a7'
check "eth_blockNumber" 200 "$(printf '%s' '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":2}' | post)" '"result"'
check "eth_call (allowlisted)" 200 "$(printf '%s' '{"jsonrpc":"2.0","method":"eth_call","params":[{"to":"0x0000000000000000000000000000000000000000","data":"0x"},"latest"],"id":3}' | post)" '"jsonrpc"'
check "eth_getLogs denied" 403 "$(printf '%s' '{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"fromBlock":"latest"}],"id":4}' | post)" 'method not allowed'
check "web3_clientVersion denied" 403 "$(printf '%s' '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":5}' | post)" 'method not allowed'
check "batch with a denied method" 403 "$(printf '%s' '[{"jsonrpc":"2.0","method":"eth_chainId","id":1},{"jsonrpc":"2.0","method":"eth_getLogs","id":2}]' | post)"
batch21=$(printf '{"jsonrpc":"2.0","method":"eth_chainId","id":%d},' $(seq 1 21))
check "batch of 21 refused" 400 "$(printf '[%s]' "${batch21%,}" | post)"
check "malformed body refused" 400 "$(printf '%s' '{"jsonrpc":"2.0",' | post)" 'invalid JSON-RPC request'
check "70 KiB body refused" 413 "$({ printf '{"jsonrpc":"2.0","method":"eth_chainId","params":["'; head -c 71680 /dev/zero | tr '\0' a; printf '"],"id":1}'; } | post)"

echo
if [[ $fail -eq 0 ]]; then
  echo "STAGE-OK: $stage on $base enforces the allowlist"
else
  echo "STAGE-FAILED: fix before any cutover; the live explorer is unchanged"
fi
exit $fail
