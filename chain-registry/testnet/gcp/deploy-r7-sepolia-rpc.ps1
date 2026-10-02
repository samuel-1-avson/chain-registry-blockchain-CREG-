param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$Zone = 'us-west1-b',
    [string]$VmName = 'creg-sepolia-rpc',
    [string]$GethImage = 'ethereum/client-go@sha256:6606aa1c7b6354d57581952604e7ab5a49689aeee9aebf668c81b6d3225395f1',
    [string]$LighthouseImage = 'sigp/lighthouse@sha256:3e5f0609c74b824d3cb1049920169114d99fd7bc9082b996b3adae3d4e9ef8dc'
)

$ErrorActionPreference = 'Stop'
if ($GethImage -notmatch '^ethereum/client-go@sha256:[0-9a-f]{64}$' -or
    $LighthouseImage -notmatch '^sigp/lighthouse@sha256:[0-9a-f]{64}$') {
    throw 'Both execution and consensus images must be pinned by immutable digest.'
}

$address = (& gcloud compute addresses describe creg-sepolia-rpc-ip "--project=$ProjectId" "--region=us-west1" --format='value(address)').Trim()
if ($LASTEXITCODE -ne 0 -or $address -notmatch '^\d{1,3}(\.\d{1,3}){3}$') {
    throw 'The reserved GCP RPC public IPv4 address is unavailable.'
}
$instanceAddress = (& gcloud compute instances describe $VmName "--project=$ProjectId" "--zone=$Zone" --format='value(networkInterfaces[0].accessConfigs[0].natIP)').Trim()
if ($LASTEXITCODE -ne 0 -or $instanceAddress -ne $address) {
    throw 'The RPC VM is not attached to its expected reserved public IPv4 address.'
}

$remoteCommand = @'
set -euo pipefail
sudo test -e /var/lib/creg-sepolia-rpc-bootstrap.done
mountpoint -q /mnt/disks/sepolia-rpc
sudo test -f /mnt/disks/sepolia-rpc/sepolia/geth/jwtsecret
sudo mkdir -p /mnt/disks/sepolia-rpc/lighthouse
sudo docker network inspect creg-rpc >/dev/null 2>&1 || sudo docker network create creg-rpc >/dev/null
sudo docker pull __GETH_IMAGE__
sudo docker pull __LIGHTHOUSE_IMAGE__

# Recreate only these two dedicated client containers. Chain databases and the
# generated execution JWT/P2P identity are on the attached persistent disk.
sudo docker rm -f creg-sepolia-beacon creg-sepolia-rpc >/dev/null 2>&1 || true
sudo docker run -d \
  --name creg-sepolia-rpc \
  --restart unless-stopped \
  --network creg-rpc \
  --publish 8545:8545 \
  --publish 30303:30303/tcp \
  --publish 30303:30303/udp \
  --mount type=bind,src=/mnt/disks/sepolia-rpc,dst=/root/.ethereum \
  __GETH_IMAGE__ \
  --sepolia \
  --syncmode=snap \
  --http \
  --http.addr=0.0.0.0 \
  --http.port=8545 \
  --http.api=eth,net,web3 \
  --http.vhosts='*' \
  --authrpc.addr=0.0.0.0 \
  --authrpc.port=8551 \
  --authrpc.jwtsecret=/root/.ethereum/sepolia/geth/jwtsecret \
  --authrpc.vhosts='*' \
  --port=30303 \
  --nat=extip:__EXTERNAL_IP__ \
  --metrics \
  --metrics.addr=127.0.0.1 \
  --metrics.port=6060

sudo docker run -d \
  --name creg-sepolia-beacon \
  --restart unless-stopped \
  --network creg-rpc \
  --publish 127.0.0.1:5052:5052 \
  --publish 9000:9000/tcp \
  --publish 9000:9000/udp \
  --mount type=bind,src=/mnt/disks/sepolia-rpc/lighthouse,dst=/root/.lighthouse \
  --mount type=bind,src=/mnt/disks/sepolia-rpc/sepolia/geth/jwtsecret,dst=/jwtsecret,readonly \
  --entrypoint lighthouse \
  __LIGHTHOUSE_IMAGE__ \
  beacon_node \
  --network=sepolia \
  --datadir=/root/.lighthouse \
  --execution-endpoint=http://creg-sepolia-rpc:8551 \
  --execution-jwt=/jwtsecret \
  --checkpoint-sync-url=https://checkpoint-sync.sepolia.ethpandaops.io \
  --checkpoint-sync-url-timeout=300 \
  --http \
  --http-address=0.0.0.0 \
  --http-port=5052 \
  --listen-address=0.0.0.0 \
  --port=9000 \
  --discovery-port=9000 \
  --enr-address=__EXTERNAL_IP__ \
  --enr-tcp-port=9000 \
  --enr-udp-port=9000

sudo docker inspect --format='container={{.Name}} image={{.Config.Image}} status={{.State.Status}}' creg-sepolia-rpc creg-sepolia-beacon
for attempt in $(seq 1 30); do
  rpc_result="$(curl -fsS -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
    http://127.0.0.1:8545 || true)"
  if [[ "$rpc_result" == *'"result":"0xaa36a7"'* ]]; then
    echo 'RPC_CHAIN_ID=0xaa36a7'
    break
  fi
  if [[ "$attempt" == 30 ]]; then
    echo 'Sepolia JSON-RPC did not become ready' >&2
    exit 1
  fi
  sleep 2
done
echo 'Lighthouse sync is asynchronous; verify the checkpoint and eth_syncing before fleet use.'
'@ -replace '__GETH_IMAGE__', $GethImage -replace '__LIGHTHOUSE_IMAGE__', $LighthouseImage -replace '__EXTERNAL_IP__', $address

& gcloud compute ssh $VmName "--project=$ProjectId" "--zone=$Zone" --tunnel-through-iap --quiet --ssh-flag=-batch "--command=$remoteCommand"
if ($LASTEXITCODE -ne 0) {
    throw 'Sepolia client deployment failed. The dedicated VM and persistent chain disk remain intact.'
}
Write-Host "Pinned Sepolia Geth and Lighthouse containers started on $address. Sync must complete and match the independent checkpoint providers before fleet configuration."
