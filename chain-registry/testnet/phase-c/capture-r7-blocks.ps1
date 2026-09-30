param(
    [Parameter(Mandatory = $true)] [string]$EvidenceDirectory,
    [Parameter(Mandatory = $true)] [ValidatePattern('^[a-z0-9-]+$')] [string]$CaptureId,
    [int[]]$Heights = @(0, 1, 2),
    [string]$CanonicalPackage = 'npm:is-number@7.0.0',
    [switch]$OmitPackageRecord,
    [string]$FleetPath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9/fleet-runtime-readiness.json')
)

$ErrorActionPreference = 'Stop'
$encodedPackage = [Uri]::EscapeDataString($CanonicalPackage)
if ($Heights.Count -eq 0 -or @($Heights | Where-Object { $_ -lt 0 }).Count -gt 0) { throw 'Invalid real block capture heights.' }
$fleet = Get-Content -LiteralPath $FleetPath -Raw | ConvertFrom-Json
if ($fleet.nodes.Count -ne 8) { throw 'Expected eight real host identities.' }
New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$remotePath = "/var/tmp/creg-r7-blocks-$CaptureId.json"
$template = @'
python3 - <<'PY'
import datetime, json, os, urllib.request
base = 'http://127.0.0.1:PORT'
record = {'captured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'blocks': {}}
def get(path):
    with urllib.request.urlopen(base + path, timeout=15) as response:
        return json.loads(response.read())
record['stats'] = get('/v1/public/chain/stats')
if INCLUDE_PACKAGE:
    record['package'] = get('/v1/public/packages/CANONICAL_PACKAGE')
record['pbft'] = get('/v1/consensus/pbft')
for height in HEIGHTS:
    record['blocks'][str(height)] = get('/v1/public/blocks/' + str(height))
with open('REMOTE_PATH', 'w', encoding='utf8') as output:
    json.dump(record, output)
os.chmod('REMOTE_PATH', 0o644)
print('Public block evidence captured')
PY
'@
foreach ($item in $fleet.nodes | Sort-Object slot) {
    $destination = Join-Path $EvidenceDirectory "$($item.slot).json"
    if (Test-Path -LiteralPath $destination) { throw 'Public block evidence already exists; preserve it and select a new capture ID/directory.' }
    if ($item.slot -eq 'v1') {
        $base = 'http://127.0.0.1:18080'
        $record = [ordered]@{ captured_at_utc = [DateTime]::UtcNow.ToString('o'); blocks = [ordered]@{} }
        $record.stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 15
        if (-not $OmitPackageRecord) {
            $record.package = Invoke-RestMethod -Uri "$base/v1/public/packages/$encodedPackage" -TimeoutSec 15
        }
        $record.pbft = Invoke-RestMethod -Uri "$base/v1/consensus/pbft" -TimeoutSec 15
        foreach ($height in $Heights) { $record.blocks[[string]$height] = Invoke-RestMethod -Uri "$base/v1/public/blocks/$height" -TimeoutSec 15 }
        $record | ConvertTo-Json -Depth 80 | Set-Content -LiteralPath $destination -Encoding utf8
    } else {
        $includePackage = if ($OmitPackageRecord) { 'False' } else { 'True' }
        $command = $template.Replace('PORT', [string]$item.port).Replace('HEIGHTS', (ConvertTo-Json -InputObject @($Heights) -Compress)).Replace('REMOTE_PATH', $remotePath).Replace('CANONICAL_PACKAGE', $encodedPackage).Replace('INCLUDE_PACKAGE', $includePackage)
        & gcloud compute ssh $item.name "--project=$($item.project)" "--zone=$($item.zone)" --tunnel-through-iap --quiet "--command=$command"
        if ($LASTEXITCODE -ne 0) { throw "Remote public block capture failed for $($item.slot)." }
        & gcloud compute scp "$($item.name):$remotePath" $destination "--project=$($item.project)" "--zone=$($item.zone)" --tunnel-through-iap --quiet --scp-flag=-q
        if ($LASTEXITCODE -ne 0) { throw "Public block evidence transfer failed for $($item.slot)." }
    }
    "$($item.slot): real block evidence saved"
}
