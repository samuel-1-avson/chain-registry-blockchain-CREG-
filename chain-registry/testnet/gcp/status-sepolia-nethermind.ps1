# Read-only status of creg-sepolia-nethermind: genesis identity, execution and
# consensus sync progress, peers, disk use and recent warnings. Changes nothing.
param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$Zone = 'europe-west1-c',
    [string]$VmName = 'creg-sepolia-nethermind'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_Invoke-RemoteBash.ps1')
$script = @'
set -uo pipefail
rpc() {
  curl -fsS -m 10 -H 'content-type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}" http://127.0.0.1:8545
}
echo "== time $(date -u +%FT%TZ)"
echo "== execution (Nethermind)"
echo "chain_id $(rpc eth_chainId '[]')"
echo "genesis $(rpc eth_getBlockByNumber '["0x0",false]' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["hash"])')"
echo "block_number $(rpc eth_blockNumber '[]')"
echo "syncing $(rpc eth_syncing '[]')"
echo "peers $(rpc net_peerCount '[]')"
echo "== consensus (Teku)"
curl -fsS -m 10 http://127.0.0.1:5051/eth/v1/node/syncing; echo
curl -fsS -m 10 http://127.0.0.1:5051/eth/v1/node/peer_count; echo
curl -fsS -m 10 http://127.0.0.1:5051/eth/v1/beacon/headers/finalized | python3 -c 'import json,sys; d=json.load(sys.stdin)["data"]; print("finalized", d["header"]["message"]["slot"], d["root"])'
echo "== containers"
sudo docker ps --format '{{.Names}} {{.Status}}'
echo "== disk"
df -h /mnt/disks/sepolia-nethermind | tail -n 1
sudo du -sh /mnt/disks/sepolia-nethermind/nethermind /mnt/disks/sepolia-nethermind/teku 2>/dev/null
echo "== recent nethermind log"
sudo docker logs --since 10m creg-sepolia-nethermind 2>&1 | tail -n 8
echo "== recent teku log"
sudo docker logs --since 10m creg-sepolia-teku 2>&1 | tail -n 6
echo "== warnings in the last 30 min"
echo "nethermind_warn_or_error $(sudo docker logs --since 30m creg-sepolia-nethermind 2>&1 | grep -cE 'WARN|ERROR|Error')"
echo "teku_warn_or_error $(sudo docker logs --since 30m creg-sepolia-teku 2>&1 | grep -cE 'WARN|ERROR')"
'@
Invoke-RemoteBash -ProjectId $ProjectId -Vm $VmName -Zone $Zone -Script $script |
    Where-Object { $_ -notmatch '^(The host key|You have no|think it is|The server|  ssh-|  compute\.|If you|cache and|If you want|the key to|If you do not|connection\.|Store key)' } |
    ForEach-Object { Write-Host $_ }
