param(
    [Parameter(Mandatory = $true)][string]$Canonical,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [Parameter(Mandatory = $true)][int]$ExpectedHeight,
    [Parameter(Mandatory = $true)][string]$ExpectedHash,
    [Parameter(Mandatory = $true)][string]$ScannerProfileDigest
)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7+ is required.' }
if ($Canonical -notmatch '^npm:[a-z0-9][a-z0-9._-]*@[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
    throw 'Only a canonical npm name@version is accepted.'
}
if ($ExpectedHash -notmatch '^[0-9a-fA-F]{64}$' -or $ScannerProfileDigest -notmatch '^[0-9a-f]{64}$') {
    throw 'Expected block and scanner profile must be SHA-256 digests.'
}
if (Test-Path -LiteralPath $EvidencePath) { throw 'Evidence exists; refusing to overwrite it.' }

$fleet = @(
    @{ slot='v1'; role='validator'; name='operator laptop'; port=18080 },
    @{ slot='v2'; role='validator'; name='creg-validator-1'; project='gen-lang-client-0022105784'; zone='us-east1-b'; port=18080 },
    @{ slot='v3'; role='validator'; name='creg-validator-3'; project='fiffi-500411'; zone='europe-west1-b'; port=8080 },
    @{ slot='v4'; role='validator'; name='creg-validator-4'; project='fiffi-500411'; zone='asia-southeast1-b'; port=8080 },
    @{ slot='o1'; role='observer'; name='creg-testnet-vm'; project='gen-lang-client-0022105784'; zone='us-central1-b'; port=18080 },
    @{ slot='o2'; role='observer'; name='creg-observer-vm'; project='gen-lang-client-0022105784'; zone='us-central1-a'; port=8080 },
    @{ slot='o3'; role='observer'; name='creg-observer-3'; project='fiffi-500411'; zone='us-west1-b'; port=8080 },
    @{ slot='o4'; role='observer'; name='creg-observer-4'; project='fiffi-500411'; zone='africa-south1-a'; port=8080 }
)
$encoded = [Uri]::EscapeDataString($Canonical)
$remoteTemplate = @'
python3 - <<'PY'
import datetime,json,urllib.error,urllib.request
base='http://127.0.0.1:PORT'; record={'slot':'SLOT','captured_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
def get(path):
 limit=65536 if path=='/metrics' else 8192
 with urllib.request.urlopen(base+path,timeout=15) as r:return r.status,r.read(limit).decode('utf8','replace')
for key,path in [('stats','/v1/public/chain/stats'),('health','/v1/public/health'),('metrics','/metrics')]:
 status,body=get(path); record[key]=json.loads(body) if key!='metrics' else body
try:
 status,body=get('/v1/public/packages/ENCODED'); record['package_http_status']=status; record['package_body']=body[:1024]
except urllib.error.HTTPError as e:
 record['package_http_status']=e.code; record['package_body']=e.read(1024).decode('utf8','replace')
print(json.dumps(record))
PY
'@
$records = @()
foreach ($hostInfo in $fleet) {
    if ($hostInfo.slot -eq 'v1') {
        $base = "http://127.0.0.1:$($hostInfo.port)"
        $stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 15
        $health = Invoke-RestMethod -Uri "$base/v1/public/health" -TimeoutSec 15
        $metrics = (Invoke-WebRequest -Uri "$base/metrics" -TimeoutSec 15).Content
        $response = Invoke-WebRequest -Uri "$base/v1/public/packages/$encoded" -TimeoutSec 15 -SkipHttpErrorCheck
        $record = [ordered]@{
            slot='v1'; captured_at_utc=[DateTime]::UtcNow.ToString('o')
            package_http_status=[int]$response.StatusCode
            package_body=([string]$response.Content).Substring(0,[Math]::Min(1024,([string]$response.Content).Length))
            stats=$stats; health=$health; metrics=$metrics
        }
    } else {
        $command = $remoteTemplate.Replace('PORT',[string]$hostInfo.port).Replace('SLOT',[string]$hostInfo.slot).Replace('ENCODED',$encoded).Replace('PROFILE',$ScannerProfileDigest)
        $gcp = @("--project=$($hostInfo.project)","--zone=$($hostInfo.zone)",'--tunnel-through-iap','--quiet')
        $output = (& gcloud compute ssh $hostInfo.name @gcp "--command=$command" 2>&1 | Out-String).Trim()
        $remoteExitCode = $LASTEXITCODE
        try { $record = $output | ConvertFrom-Json -AsHashtable }
        catch { throw "Node $($hostInfo.slot) returned non-JSON lookup evidence (gcloud exit $remoteExitCode): $output" }
        $record.remote_command_exit_code = $remoteExitCode
    }
    $metricsText = [string]$record.metrics
    $record.scanner_profile_matches = $metricsText -match ('profile_digest="' + [regex]::Escape($ScannerProfileDigest) + '"')
    $record.Remove('metrics')
    $expectedNodeId = "$($hostInfo.role)-$($hostInfo.slot.Substring(1))"
    $record.role = $hostInfo.role
    $record.expected_node_id = $expectedNodeId
    $record.checks = [ordered]@{
        package_absent = $record.package_http_status -eq 404
        signed_r7 = $record.stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $record.stats.spec_signature_verified -eq $true
        exact_shared_tip = $record.stats.tip_height -eq $ExpectedHeight -and $record.stats.finalized_height -eq $ExpectedHeight -and ([string]$record.stats.tip_hash).ToLowerInvariant().Replace('0x','') -eq $ExpectedHash.ToLowerInvariant()
        four_validators = $record.stats.validator_count -eq 4 -and $record.stats.active_validators -eq 4
        correct_role = $record.health.node_id -eq $expectedNodeId -and $record.health.is_validator -eq ($hostInfo.role -eq 'validator')
        scanner_profile = $record.scanner_profile_matches -eq $true
    }
    $record.passed = @($record.checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    $records += $record
    Write-Output "$($hostInfo.slot): absent=$($record.checks.package_absent); shared_tip=$($record.checks.exact_shared_tip); profile=$($record.checks.scanner_profile)"
}
$report = [ordered]@{
    schema='creg-r7-real-package-absence-at-tip-v1'
    captured_at_utc=[DateTime]::UtcNow.ToString('o')
    canonical=$Canonical
    expected_height=$ExpectedHeight
    expected_hash=$ExpectedHash.ToLowerInvariant()
    scanner_profile_digest=$ScannerProfileDigest
    all_eight_absent=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.package_absent }).Count -eq 0
    all_eight_same_tip=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.exact_shared_tip }).Count -eq 0
    all_eight_profile_ready=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.scanner_profile }).Count -eq 0
    passed=$records.Count -eq 8 -and @($records | Where-Object { -not $_.passed }).Count -eq 0
    public_cutover=$false
    nodes=$records
}
New-Item -ItemType Directory -Path (Split-Path -Parent $EvidencePath) -Force | Out-Null
$report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $report.passed) { throw "All-eight real-node package absence gate failed; inspect $EvidencePath" }
Write-Output "All eight real nodes confirm $Canonical is absent at shared h$ExpectedHeight. Evidence: $EvidencePath"
