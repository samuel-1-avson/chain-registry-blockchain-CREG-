# Publisher quickstart sign-off — live probes for PUBLIC_TESTNET_QUICKSTART.md prerequisites.
#
# Usage:
#   .\testnet\publisher-quickstart-verify.ps1
#   .\testnet\publisher-quickstart-verify.ps1 -BaseDomain testnet.cregnet.dev -ReleaseTag v0.1.1-testnet

param(
    [string]$BaseDomain = "testnet.cregnet.dev",
    [string]$ReleaseTag = "v0.1.1-testnet",
    [string]$GithubRepo = "samuel-1-avson/chain-registry-blockchain-CREG-"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Log($m) { Write-Host "[publisher-quickstart] $m" }
function Fail($m) { Log "FAIL $m"; exit 1 }

$logDir = Join-Path $scriptDir "publisher-quickstart-logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$evidence = Join-Path $logDir ("publisher-quickstart-{0:yyyyMMdd-HHmmss}.json" -f (Get-Date))

$report = @{
    timestamp = (Get-Date).ToUniversalTime().ToString("o")
    base_domain = $BaseDomain
    release_tag = $ReleaseTag
    checks = @{}
}

# 1) Public endpoints (HOSTING-301 subset)
& (Join-Path $scriptDir "hosting-301-verify.ps1") -BaseDomain $BaseDomain
$report.checks.hosting_301 = ($LASTEXITCODE -eq 0)
if (-not $report.checks.hosting_301) { Fail "hosting-301-verify failed" }

# 2) Chain spec service URLs match public fleet
$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
$spec = Get-Content $specPath -Raw | ConvertFrom-Json
$serviceChecks = @{
    explorer    = "https://explorer.$BaseDomain"
    faucet      = "https://faucet.$BaseDomain"
    ipfs_gateway = "https://ipfs.$BaseDomain"
}
foreach ($key in $serviceChecks.Keys) {
    $got = $spec.services.$key
    $want = $serviceChecks[$key]
    if ($got -ne $want) {
        Fail "chain-spec services.$key=$got expected $want"
    }
}
$metrics = [string]$spec.services.metrics
if ($metrics -notmatch "api\.$([regex]::Escape($BaseDomain))") {
    Fail "chain-spec services.metrics=$metrics expected api host on $BaseDomain"
}
$report.checks.chain_spec_services = $true
Log "OK chain-spec public service URLs ($BaseDomain)"

# 3) Release binaries exist for install path in quickstart
try {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$GithubRepo/releases/tags/$ReleaseTag" -TimeoutSec 30
    $assetNames = @($rel.assets | ForEach-Object { $_.name })
    $need = @(
        "chain-registry-$ReleaseTag-linux-amd64.tar.gz",
        "chain-registry-$ReleaseTag-windows-amd64.zip",
        "chain-registry-$ReleaseTag-macos-amd64.tar.gz"
    )
    foreach ($n in $need) {
        if ($assetNames -notcontains $n) {
            Fail "release $ReleaseTag missing asset: $n"
        }
    }
    $report.checks.release_assets = $true
    Log "OK release $ReleaseTag assets ($($assetNames.Count) files)"
} catch {
    Fail "GitHub release $ReleaseTag not found: $_"
}

# 4) API health — publisher-facing fields
$healthUrl = "https://api.$BaseDomain/v1/health"
$healthRaw = curl.exe -fsS $healthUrl
$health = $healthRaw | ConvertFrom-Json
if ($health.status -ne "ok") { Fail "api health status not ok" }
if ($health.validator_set_sync.state -ne "synced") {
    Fail "validator_set_sync not synced: $($health.validator_set_sync.state)"
}
$report.checks.api_health = $true
$report.checks.validator_set_sync = $health.validator_set_sync.state
Log "OK $healthUrl synced block $($health.validator_set_sync.cursor_block)"

# 5) Quickstart doc present + alpha disclaimer section
$quickstart = Join-Path (Split-Path $repoRoot -Parent) "docs\PUBLIC_TESTNET_QUICKSTART.md"
if (-not (Test-Path $quickstart)) { Fail "missing $quickstart" }
$qsText = Get-Content $quickstart -Raw
if ($qsText -notmatch 'SEC-401' -or $qsText -notmatch 'public alpha') {
    Fail "PUBLIC_TESTNET_QUICKSTART.md missing alpha/SEC-401 disclaimer"
}
if ($qsText -notmatch [regex]::Escape($ReleaseTag) -and $qsText -notmatch 'v0\.1\.\d+-testnet') {
    Log "WARN quickstart install section may need release tag bump to $ReleaseTag"
}
$report.checks.quickstart_doc = $true

$report.publisher_quickstart = $true
$report | ConvertTo-Json -Depth 6 | Set-Content -Path $evidence -Encoding utf8
Log "PUBLISHER quickstart verify PASSED (evidence: $evidence)"
exit 0
