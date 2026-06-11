# L2 Public Alpha — local gate verification (no GCP SSH required unless -Live).
#
# Usage:
#   .\testnet\l2-gate-verify.ps1
#   .\testnet\l2-gate-verify.ps1 -Live -BaseDomain testnet.cregnet.dev

param(
    [switch]$Live,
    [string]$BaseDomain = "testnet.cregnet.dev"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$docsRoot = Split-Path -Parent $repoRoot
Set-Location $repoRoot

function Log($m) { Write-Host "[l2-gate] $m" }

$gates = [ordered]@{
    consensus_evidence_script = $false
    malicious_fixtures        = $false
    validator_checklist_doc   = $false
    incident_runbook_doc      = $false
    ipfs_pin_script           = $false
    l2_status_doc             = $false
    live_api_health           = $null
    live_sandbox_reported     = $null
}

if (Test-Path "scripts/validate-consensus-evidence.mjs") {
    node scripts/validate-consensus-evidence.mjs | Out-Null
    if ($LASTEXITCODE -eq 0) { $gates.consensus_evidence_script = $true }
}

& (Join-Path $scriptDir "malicious-fixtures-verify.ps1")
if ($LASTEXITCODE -eq 0) { $gates.malicious_fixtures = $true }

$gates.validator_checklist_doc = Test-Path (Join-Path $docsRoot "docs/VALIDATOR_ONBOARDING_CHECKLIST.md")
$gates.incident_runbook_doc = Test-Path (Join-Path $docsRoot "docs/INCIDENT_RESPONSE_RUNBOOK.md")
$gates.ipfs_pin_script = Test-Path (Join-Path $scriptDir "ipfs-pin-check.py")
$gates.l2_status_doc = Test-Path (Join-Path $docsRoot "docs/L2_PUBLIC_ALPHA_GATE_STATUS.md")

if ($Live) {
    $api = "https://api.$BaseDomain/v1/health"
    try {
        $health = Invoke-RestMethod -Uri $api -TimeoutSec 20
        $gates.live_api_health = ($health.status -eq "ok")
        if ($health.sandbox) {
            $gates.live_sandbox_reported = [bool]$health.sandbox.engine
        }
    } catch {
        $gates.live_api_health = $false
        Log "WARN: could not fetch $api — $($_.Exception.Message)"
    }
}

$failed = @($gates.GetEnumerator() | Where-Object { $_.Value -eq $false })
if ($failed.Count -gt 0) {
    $failed | ForEach-Object { Log "FAIL: $($_.Key)" }
    throw "L2 local gate verify failed ($($failed.Count) checks)"
}

$outDir = Join-Path $scriptDir "l2-gate-logs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir ("l2-gate-{0}.json" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
([ordered]@{
    timestamp = (Get-Date).ToUniversalTime().ToString("o")
    gates     = $gates
} | ConvertTo-Json -Depth 5) | Set-Content -Path $outPath -Encoding utf8

Log "L2 local gates PASSED (evidence: $outPath)"
