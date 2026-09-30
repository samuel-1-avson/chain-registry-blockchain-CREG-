param(
    [Parameter(Mandatory = $true)][string]$Canonical,
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [string]$ScannerProfileDigest = '8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e'
)
$ErrorActionPreference = 'Stop'
if ($Canonical -notmatch '^npm:[a-z0-9][a-z0-9._-]*@[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
    throw 'Only a canonical npm name@version is accepted by this read-only gate.'
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
import datetime,json,re,urllib.error,urllib.request
base='http://127.0.0.1:PORT'
record={'slot':'SLOT','captured_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
with urllib.request.urlopen(base+'/v1/public/chain/stats',timeout=12) as r: stats=json.loads(r.read().decode())
with urllib.request.urlopen(base+'/v1/public/health',timeout=12) as r: health=json.loads(r.read().decode())
with urllib.request.urlopen(base+'/metrics',timeout=12) as r: metrics=r.read().decode()
try:
 with urllib.request.urlopen(base+'/v1/public/packages/ENCODED',timeout=12) as r:
  record['package_http_status']=r.status; record['package_body']=r.read(1024).decode('utf8','replace')
except urllib.error.HTTPError as error:
 record['package_http_status']=error.code; record['package_body']=error.read(1024).decode('utf8','replace')
record.update({'stats':stats,'health':health,'scanner_profile_matches':f'profile_digest="PROFILE"' in metrics})
print(json.dumps(record))
PY
'@
$records = @()
foreach ($hostInfo in $fleet) {
    if ($hostInfo.slot -eq 'v1') {
        $base = "http://127.0.0.1:$($hostInfo.port)"
        $stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 12
        $health = Invoke-RestMethod -Uri "$base/v1/public/health" -TimeoutSec 12
        $metrics = (Invoke-WebRequest -Uri "$base/metrics" -TimeoutSec 12).Content
        $response = Invoke-WebRequest -Uri "$base/v1/public/packages/$encoded" -TimeoutSec 12 -SkipHttpErrorCheck
        $record = [ordered]@{ slot='v1'; captured_at_utc=[DateTime]::UtcNow.ToString('o'); package_http_status=[int]$response.StatusCode; package_body=([string]$response.Content).Substring(0,[Math]::Min(1024,([string]$response.Content).Length)); stats=$stats; health=$health; scanner_profile_matches=$metrics.Contains("profile_digest=`"$ScannerProfileDigest`"") }
    } else {
        $command = $remoteTemplate.Replace('PORT',[string]$hostInfo.port).Replace('SLOT',[string]$hostInfo.slot).Replace('ENCODED',$encoded).Replace('PROFILE',$ScannerProfileDigest)
        $gcp = @("--project=$($hostInfo.project)","--zone=$($hostInfo.zone)",'--tunnel-through-iap','--quiet')
        $output = (& gcloud compute ssh $hostInfo.name @gcp "--command=$command" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Read-only package lookup failed on $($hostInfo.slot)." }
        try { $record = $output | ConvertFrom-Json -AsHashtable } catch { throw "Node $($hostInfo.slot) returned non-JSON lookup evidence." }
    }
    $record.slot = $hostInfo.slot
    $record.role = $hostInfo.role
    $record.expected_node_id = "$($hostInfo.role)-$($hostInfo.slot.Substring(1))"
    $record.checks = [ordered]@{
        package_absent = $record.package_http_status -eq 404
        signed_r7 = $record.stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $record.stats.spec_signature_verified -eq $true
        shared_h16 = $record.stats.tip_height -eq 16 -and $record.stats.finalized_height -eq 16 -and $record.stats.tip_hash -eq '4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260'
        correct_role = $record.health.node_id -eq $record.expected_node_id -and $record.health.is_validator -eq ($hostInfo.role -eq 'validator')
        r13_scanner = $record.scanner_profile_matches -eq $true
    }
    $record.passed = @($record.checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    $records += $record
    Write-Output "$($hostInfo.slot): package_absent=$($record.checks.package_absent); shared_h16=$($record.checks.shared_h16); R13=$($record.checks.r13_scanner)"
}
$report = [ordered]@{
    schema='creg-r7-real-package-absence-v1'
    captured_at_utc=[DateTime]::UtcNow.ToString('o')
    canonical=$Canonical
    scanner_profile_digest=$ScannerProfileDigest
    all_eight_absent=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.package_absent }).Count -eq 0
    all_eight_same_h16=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.shared_h16 }).Count -eq 0
    all_eight_r13_ready=$records.Count -eq 8 -and @($records | Where-Object { -not $_.checks.r13_scanner }).Count -eq 0
    passed=$records.Count -eq 8 -and @($records | Where-Object { -not $_.passed }).Count -eq 0
    public_cutover=$false
    nodes=$records
}
New-Item -ItemType Directory -Path (Split-Path -Parent $EvidencePath) -Force | Out-Null
$report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $report.passed) { throw "Real-node package absence gate failed; inspect $EvidencePath" }
Write-Output "All eight real nodes confirm $Canonical is absent on shared h16. Evidence: $EvidencePath"
