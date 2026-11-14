# Start the pinned Nethermind + Teku containers on creg-sepolia-nethermind.
# Chain data, the JWT secret and the Teku initial state live on the persistent
# data disk, so recreating the containers does not resync.
#
# Teku start point, in order of preference:
#   1. an existing Teku database (restart);
#   2. /mnt/disks/sepolia-nethermind/teku/initial-state.ssz from CREG's own
#      Lighthouse (fetch-beacon-initial-state.ps1);
#   3. a public checkpoint service, only with -AllowExternalCheckpoint.
param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$Region = 'europe-west1',
    [string]$Zone = 'europe-west1-c',
    [string]$VmName = 'creg-sepolia-nethermind',
    [string]$AddressName = 'creg-sepolia-nethermind-ip',
    [string]$NethermindImage = 'nethermind/nethermind@sha256:57df9bfaaf4d577919b35a2c7b1582ee5bd1812b36a812bdf8ac88ff7dba524d',
    [string]$TekuImage = 'consensys/teku@sha256:4560398cf9a0b208a4a4a12ebc2051d820b67df51092a54154031dacc57f2ac6',
    # CREG needs receipts from the R7 Staking deployment (11,781,426). 0 = full history.
    [int]$HistoryBarrier = 11780000,
    [switch]$AllowExternalCheckpoint
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_Invoke-RemoteBash.ps1')
if ($NethermindImage -notmatch '^nethermind/nethermind@sha256:[0-9a-f]{64}$' -or
    $TekuImage -notmatch '^consensys/teku@sha256:[0-9a-f]{64}$') {
    throw 'Both client images must be pinned by immutable digest.'
}
if ($HistoryBarrier -lt 0 -or $HistoryBarrier -gt 11781426) {
    throw 'The history barrier must be between 0 and the R7 Staking deployment block (11,781,426).'
}

$address = (& gcloud compute addresses describe $AddressName "--project=$ProjectId" "--region=$Region" --format='value(address)').Trim()
if ($LASTEXITCODE -ne 0 -or $address -notmatch '^\d{1,3}(\.\d{1,3}){3}$') { throw 'The reserved public IPv4 address is unavailable.' }
$instanceAddress = (& gcloud compute instances describe $VmName "--project=$ProjectId" "--zone=$Zone" --format='value(networkInterfaces[0].accessConfigs[0].natIP)').Trim()
if ($LASTEXITCODE -ne 0 -or $instanceAddress -ne $address) { throw 'The VM is not attached to its reserved public IPv4 address.' }

$remoteScript = @'
set -euo pipefail
nethermind_image=$1; teku_image=$2; external_ip=$3; barrier=$4; allow_external=$5
base=/mnt/disks/sepolia-nethermind
sudo test -e /var/lib/creg-sepolia-nethermind-bootstrap.done
mountpoint -q "$base"
sudo test -s "$base/jwtsecret"
sudo docker network inspect creg-rpc >/dev/null 2>&1 || sudo docker network create creg-rpc >/dev/null
sudo docker pull -q "$nethermind_image"
sudo docker pull -q "$teku_image"

if sudo test -d "$base/teku/beacon"; then
  start_args=(); echo 'Teku start point: existing database'
elif sudo test -s "$base/teku/initial-state.ssz"; then
  start_args=(--initial-state=/data/initial-state.ssz); echo 'Teku start point: CREG Lighthouse finalized state'
elif [[ "$allow_external" == true ]]; then
  start_args=(--checkpoint-sync-url=https://checkpoint-sync.sepolia.ethpandaops.io); echo 'Teku start point: external checkpoint (explicitly allowed)'
else
  echo 'No Teku start point. Run fetch-beacon-initial-state.ps1, or pass -AllowExternalCheckpoint.' >&2; exit 1
fi

sudo docker rm -f creg-sepolia-teku creg-sepolia-nethermind >/dev/null 2>&1 || true
sudo docker run -d \
  --name creg-sepolia-nethermind \
  --restart unless-stopped \
  --network creg-rpc \
  --publish 8545:8545 \
  --publish 30303:30303/tcp \
  --publish 30303:30303/udp \
  --mount type=bind,src="$base/nethermind",dst=/data \
  --mount type=bind,src="$base/jwtsecret",dst=/jwtsecret,readonly \
  "$nethermind_image" \
  --config sepolia \
  --data-dir /data \
  --JsonRpc.Enabled true \
  --JsonRpc.Host 0.0.0.0 \
  --JsonRpc.Port 8545 \
  --JsonRpc.EnabledModules '[Eth,Net,Web3]' \
  --JsonRpc.EngineHost 0.0.0.0 \
  --JsonRpc.EnginePort 8551 \
  --JsonRpc.JwtSecretFile /jwtsecret \
  --Network.ExternalIp "$external_ip" \
  --Network.P2PPort 30303 \
  --Network.DiscoveryPort 30303 \
  --Network.MaxActivePeers 25 \
  --Sync.AncientBodiesBarrier "$barrier" \
  --Sync.AncientReceiptsBarrier "$barrier" >/dev/null

sudo docker run -d \
  --name creg-sepolia-teku \
  --restart unless-stopped \
  --network creg-rpc \
  --publish 127.0.0.1:5051:5051 \
  --publish 9000:9000/tcp \
  --publish 9000:9000/udp \
  --mount type=bind,src="$base/teku",dst=/data \
  --mount type=bind,src="$base/jwtsecret",dst=/jwtsecret,readonly \
  "$teku_image" \
  --network=sepolia \
  --data-path=/data \
  --ee-endpoint=http://creg-sepolia-nethermind:8551 \
  --ee-jwt-secret-file=/jwtsecret \
  --rest-api-enabled=true \
  --rest-api-interface=0.0.0.0 \
  --rest-api-port=5051 \
  --rest-api-host-allowlist=localhost,127.0.0.1 \
  --p2p-port=9000 \
  --p2p-advertised-ip="$external_ip" \
  --p2p-peer-lower-bound=40 \
  --p2p-peer-upper-bound=50 \
  "${start_args[@]}" >/dev/null

sudo docker inspect --format='container={{.Name}} image={{.Config.Image}} status={{.State.Status}}' creg-sepolia-nethermind creg-sepolia-teku
for attempt in $(seq 1 60); do
  result="$(curl -fsS -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' http://127.0.0.1:8545 || true)"
  if [[ "$result" == *'"result":"0xaa36a7"'* ]]; then echo 'RPC_CHAIN_ID=0xaa36a7'; break; fi
  if [[ "$attempt" == 60 ]]; then
    echo 'Nethermind JSON-RPC did not become ready; recent log:' >&2
    sudo docker logs --tail 40 creg-sepolia-nethermind >&2 || true
    exit 1
  fi
  sleep 3
done
echo '== nethermind log'; sudo docker logs --tail 12 creg-sepolia-nethermind 2>&1
echo '== teku log'; sudo docker logs --tail 12 creg-sepolia-teku 2>&1
df -h "$base" | tail -n 1
'@

$allow = if ($AllowExternalCheckpoint) { 'true' } else { 'false' }
$lines = Invoke-RemoteBash -ProjectId $ProjectId -Vm $VmName -Zone $Zone -Script $remoteScript `
    -Arguments @($NethermindImage, $TekuImage, $address, [string]$HistoryBarrier, $allow)
$lines | Where-Object { $_ -notmatch '^(The host key|You have no|think it is|The server|  ssh-|If you|cache and|If you want|the key to|If you do not|connection\.|Store key)' } | ForEach-Object { Write-Host $_ }
Write-Host "Nethermind + Teku started on $address. Sync takes hours to days; check progress and disk use before adding it to the fleet."
