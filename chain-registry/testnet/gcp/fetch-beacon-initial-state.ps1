# Copy the finalized Sepolia beacon state from CREG's own Lighthouse (on
# creg-sepolia-rpc) to the Teku data directory on creg-sepolia-nethermind, so
# Teku starts from a self-hosted checkpoint instead of a public checkpoint
# service. Read-only on the source node; it only reads its local Beacon API.
param(
    [string]$ProjectId = 'fiffi-500411',
    [string]$SourceVm = 'creg-sepolia-rpc',
    [string]$SourceZone = 'us-west1-b',
    [string]$TargetVm = 'creg-sepolia-nethermind',
    [string]$TargetZone = 'europe-west1-c',
    [string]$WorkDir = 'C:\Users\samue\.cargo-targets\sepolia-initial-state'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '_Invoke-RemoteBash.ps1')
$scp = @("--project=$ProjectId", '--tunnel-through-iap', '--strict-host-key-checking=no', '--quiet')
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
$local = Join-Path $WorkDir 'sepolia-finalized-state.ssz'
$record = Join-Path $WorkDir 'initial-state-record.json'

$sourceScript = @'
set -euo pipefail
api=http://127.0.0.1:5052
curl -fsS "$api/eth/v1/beacon/headers/finalized" > /tmp/creg-finalized-header.json
curl -fsS -H 'Accept: application/octet-stream' "$api/eth/v2/debug/beacon/states/finalized" -o /tmp/creg-sepolia-finalized-state.ssz
python3 - <<'PY'
import hashlib, json
h = json.load(open('/tmp/creg-finalized-header.json'))
data = open('/tmp/creg-sepolia-finalized-state.ssz', 'rb').read()
print(json.dumps({'root': h['data']['root'], 'slot': h['data']['header']['message']['slot'],
                  'execution_optimistic': h.get('execution_optimistic'), 'finalized': h.get('finalized'),
                  'state_bytes': len(data), 'state_sha256': hashlib.sha256(data).hexdigest()}))
PY
'@
$sourceLines = Invoke-RemoteBash -ProjectId $ProjectId -Vm $SourceVm -Zone $SourceZone -Script $sourceScript
$sourceInfo = ($sourceLines | Where-Object { $_ -match '^\{"root"' } | Select-Object -Last 1) | ConvertFrom-Json
if (-not $sourceInfo -or $sourceInfo.state_bytes -lt 1000000) { throw 'Lighthouse did not return a plausible finalized state.' }
Write-Host ("Lighthouse finalized checkpoint: slot {0}, root {1}, {2} bytes, execution_optimistic={3}" -f $sourceInfo.slot, $sourceInfo.root, $sourceInfo.state_bytes, $sourceInfo.execution_optimistic)
if ($sourceInfo.execution_optimistic) {
    Write-Host 'Note: Geth is still syncing, so Lighthouse has not yet verified this block''s execution payload. The checkpoint is finalized by the network; re-check the root once Geth is at head.'
}

& gcloud compute scp "${SourceVm}:/tmp/creg-sepolia-finalized-state.ssz" $local "--zone=$SourceZone" @scp
if ($LASTEXITCODE -ne 0) { throw 'Copy from the source VM failed.' }
$localSha = (Get-FileHash -LiteralPath $local -Algorithm SHA256).Hash.ToLowerInvariant()
if ($localSha -ne $sourceInfo.state_sha256) { throw 'State checksum changed in transit (source to laptop).' }

& gcloud compute scp $local "${TargetVm}:/tmp/creg-sepolia-finalized-state.ssz" "--zone=$TargetZone" @scp
if ($LASTEXITCODE -ne 0) { throw 'Copy to the target VM failed.' }
$targetScript = @'
set -euo pipefail
expected=$1
actual=$(sha256sum /tmp/creg-sepolia-finalized-state.ssz | cut -d' ' -f1)
if [[ "$actual" != "$expected" ]]; then echo 'State checksum mismatch on the target VM.' >&2; exit 1; fi
if sudo test -d /mnt/disks/sepolia-nethermind/teku/beacon; then
  echo 'Teku already has a database; leaving it untouched.' >&2; exit 1
fi
sudo install -o 1000 -g 1000 -m 0640 /tmp/creg-sepolia-finalized-state.ssz /mnt/disks/sepolia-nethermind/teku/initial-state.ssz
rm -f /tmp/creg-sepolia-finalized-state.ssz
echo INSTALLED
'@
$installed = Invoke-RemoteBash -ProjectId $ProjectId -Vm $TargetVm -Zone $TargetZone -Script $targetScript -Arguments @($sourceInfo.state_sha256)
if (-not ($installed -contains 'INSTALLED')) { throw 'Initial state was not installed on the target VM.' }

$sourceInfo | Add-Member -NotePropertyName source_vm -NotePropertyValue $SourceVm
$sourceInfo | Add-Member -NotePropertyName captured_utc -NotePropertyValue ([DateTime]::UtcNow.ToString('o'))
$sourceInfo | ConvertTo-Json | Set-Content -LiteralPath $record -Encoding UTF8
Write-Host "Installed the finalized state on $TargetVm. Record: $record"
