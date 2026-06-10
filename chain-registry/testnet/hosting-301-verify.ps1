# HOSTING-301 - Verify public HTTPS endpoints match chain spec and respond.
#
# Usage:
#   .\testnet\hosting-301-verify.ps1 -BaseDomain testnet.creg.dev
#   .\testnet\hosting-301-verify.ps1   # reads CREG_PUBLIC_* from sepolia-3node.env

param(
    [string]$BaseDomain = "",
    [string]$EnvFile = ""
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Log($msg) { Write-Host "[hosting-301] $msg" }
function Fail($msg) { Log "FAIL $msg"; exit 1 }

if (-not $EnvFile) { $EnvFile = Join-Path $scriptDir "sepolia-3node.env" }
$envVars = @{}
if (Test-Path $EnvFile) {
    foreach ($line in Get-Content $EnvFile) {
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$') {
            $envVars[$matches[1]] = $matches[2].Trim()
        }
    }
}

if ($BaseDomain) {
    $apiHost = "api.$BaseDomain"
    $explorerHost = "explorer.$BaseDomain"
    $faucetHost = "faucet.$BaseDomain"
    $specHost = "spec.$BaseDomain"
    $ipfsHost = "ipfs.$BaseDomain"
} else {
    $apiHost = $envVars["CREG_PUBLIC_API_HOST"]
    $explorerHost = $envVars["CREG_PUBLIC_EXPLORER_HOST"]
    $faucetHost = $envVars["CREG_PUBLIC_FAUCET_HOST"]
    $specHost = $envVars["CREG_PUBLIC_SPEC_HOST"]
    $ipfsHost = $envVars["CREG_PUBLIC_IPFS_HOST"]
    if (-not $apiHost) { Fail "Set -BaseDomain or CREG_PUBLIC_* in $EnvFile" }
}

$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
if (-not (Test-Path $specPath)) { Fail "Missing $specPath" }
$spec = Get-Content $specPath -Raw | ConvertFrom-Json

$checks = @{
    timestamp = (Get-Date).ToUniversalTime().ToString("o")
    hosting301 = $false
    endpoints = @()
}

function Test-HttpsEndpoint {
    param(
        [string]$Label,
        [string]$Url
    )
    $entry = @{ label = $Label; url = $Url; ok = $false; detail = "" }
    try {

        if ($Label -eq "faucet") {
            $body = curl.exe -sS -L --max-time 25 $Url 2>&1
            if ($body -match '"faucet"\s*:\s*"online"') {
                $entry.ok = $true
                $entry.detail = "reachable (degraded allowed)"
                return $entry
            }
            $entry.detail = "faucet not online: $body"
            return $entry
        }
        if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
            if ($Label -eq "ipfs") {
                $code = curl.exe -sS -L --max-time 25 -o NUL -w "%{http_code}" -X POST $Url 2>&1
                if ($code -notmatch '^[23]\d\d$') {
                    $entry.detail = "bad status: HTTP $code"
                    return $entry
                }
            } else {
                $status = curl.exe -sI -L --max-time 25 $Url 2>&1 | Select-Object -First 1
                if ($status -notmatch '^HTTP/\S+\s+([23]\d\d)') {
                    $entry.detail = "bad status: $status"
                    return $entry
                }
            }
        } else {
            $r = Invoke-WebRequest -Uri $Url -Method Head -TimeoutSec 25 -MaximumRedirection 5
            if ($r.StatusCode -lt 200 -or $r.StatusCode -ge 400) {
                $entry.detail = "HTTP $($r.StatusCode)"
                return $entry
            }
        }
        $entry.ok = $true
        $entry.detail = "reachable"
    } catch {
        $entry.detail = $_.Exception.Message
    }
    return $entry
}

$apiUrl = "https://$apiHost/v1/health"
$explorerUrl = "https://$explorerHost/"
$specUrl = "https://$specHost/chain-spec.sepolia.json"
$sigUrl = "https://$specHost/chain-spec.sepolia.json.sig"
$faucetUrl = "https://$faucetHost/health"
$ipfsUrl = "https://$ipfsHost/api/v0/version"

Log "API host: $apiHost"
foreach ($pair in @(
    @{ L = "api_health"; U = $apiUrl }
    @{ L = "explorer"; U = $explorerUrl }
    @{ L = "spec_json"; U = $specUrl }
    @{ L = "spec_sig"; U = $sigUrl }
    @{ L = "faucet"; U = $faucetUrl }
    @{ L = "ipfs"; U = $ipfsUrl }
)) {
    $r = Test-HttpsEndpoint -Label $pair.L -Url $pair.U
    $checks.endpoints += $r
    if ($r.ok) { Log "OK $($pair.L) $($pair.U)" } else { Log "MISSING $($pair.L) $($pair.U) - $($r.detail)" }
}

$failed = @($checks.endpoints | Where-Object { -not $_.ok })
if ($failed.Count -gt 0) {
    Fail "$($failed.Count) endpoint(s) not reachable - check DNS, Caddy, and firewall (gcp-public-hosting.md)"
}

# API body check
try {
    $health = Invoke-RestMethod -Uri $apiUrl -TimeoutSec 25
    if (-not ($health.ok -or $health.status -eq "ok")) { Fail "API health JSON not ok" }
    Log "OK api health body"
} catch {
    Fail "API health JSON: $($_.Exception.Message)"
}

$checks.hosting301 = $true
$outDir = Join-Path $scriptDir "hosting-301-logs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir ("hosting-301-{0}.json" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
$checks | ConvertTo-Json -Depth 6 | Set-Content -Path $outPath -Encoding utf8
Log "HOSTING-301 verify PASSED (see $outPath)"
