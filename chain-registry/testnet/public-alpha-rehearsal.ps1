# MAIN-006 — public alpha rehearsal orchestrator.
#
# Dry-run checks local evidence artifacts; -Execute runs live probes against testnet.
#
# Usage:
#   .\testnet\public-alpha-rehearsal.ps1
#   .\testnet\public-alpha-rehearsal.ps1 -Execute -BaseDomain testnet.cregnet.dev

param(
    [switch]$Execute,
    [string]$BaseDomain = "testnet.cregnet.dev"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Log($m) { Write-Host "[rehearsal] $m" }

$steps = [ordered]@{
    l2_gates           = $false
    malicious_fixtures = $false
    hosting_verify     = $null
    fleet_sandbox      = $null
}

& (Join-Path $scriptDir "l2-gate-verify.ps1")
$steps.l2_gates = ($LASTEXITCODE -eq 0)

& (Join-Path $scriptDir "malicious-fixtures-verify.ps1")
$steps.malicious_fixtures = ($LASTEXITCODE -eq 0)

if ($Execute) {
    $hostScript = Join-Path $scriptDir "hosting-301-verify.ps1"
    if (Test-Path $hostScript) {
        & $hostScript -BaseDomain $BaseDomain
        $steps.hosting_verify = ($LASTEXITCODE -eq 0)
    }
    $fleetScript = Join-Path $scriptDir "gcp\verify-fleet-sandbox.ps1"
    if (Test-Path $fleetScript) {
        & $fleetScript
        $steps.fleet_sandbox = ($LASTEXITCODE -eq 0)
    }
}

Log "Rehearsal summary:"
foreach ($key in $steps.Keys) {
    $val = $steps[$key]
    if ($null -eq $val) {
        Log "  $key : skipped (dry-run)"
    } elseif ($val) {
        Log "  $key : PASS"
    } else {
        Log "  $key : FAIL"
    }
}

$failed = @($steps.Values | Where-Object { $_ -eq $false })
if ($failed.Count -gt 0) {
    Log "FAILED — $($failed.Count) step(s) did not pass"
    exit 1
}

Log "PASSED — rehearsal checks complete"
exit 0
