# Smoke checks for public testnet chain finalization and consensus observability.
#
# Environment variables (all optional):
#   CREG_SMOKE_BASE_DOMAIN   — API host base domain (default: testnet.cregnet.dev)
#   CREG_SMOKE_API_URL       — Full chain API base URL (overrides BASE_DOMAIN)
#   CREG_SMOKE_PACKAGE       — Canonical package to check for verified status
#   CREG_SMOKE_MIN_TIP       — Minimum acceptable tip_height (default: 0)
#   CREG_SMOKE_TIMEOUT_SECS  — Per-request curl timeout (default: 25)
#
# Usage:
#   .\testnet\consensus-finalization-smoke.ps1
#   $env:CREG_SMOKE_BASE_DOMAIN = "testnet.cregnet.dev"; .\testnet\consensus-finalization-smoke.ps1

param(
    [string]$BaseDomain = $(if ($env:CREG_SMOKE_BASE_DOMAIN) { $env:CREG_SMOKE_BASE_DOMAIN } else { "testnet.cregnet.dev" }),
    [string]$ApiUrl = $env:CREG_SMOKE_API_URL,
    [string]$PackageCanonical = $env:CREG_SMOKE_PACKAGE,
    [int]$MinTipHeight = $(if ($env:CREG_SMOKE_MIN_TIP) { [int]$env:CREG_SMOKE_MIN_TIP } else { 0 }),
    [int]$TimeoutSecs = $(if ($env:CREG_SMOKE_TIMEOUT_SECS) { [int]$env:CREG_SMOKE_TIMEOUT_SECS } else { 25 })
)

$ErrorActionPreference = "Stop"

function Log($m) { Write-Host "[consensus-finalization-smoke] $m" }

if ([string]::IsNullOrWhiteSpace($ApiUrl)) {
    $ApiUrl = "https://api.$BaseDomain"
}
$ApiUrl = $ApiUrl.TrimEnd("/")

$checks = @(
    @{
        label = "public_health"
        url = "$ApiUrl/v1/public/health"
        validate = {
            param($json)
            if ($json.status -ne "ok") { return "expected status ok" }
            if ($null -eq $json.sync) { return "missing sync section" }
            if ($null -eq $json.sync.lag_blocks) { return "missing sync.lag_blocks" }
            return $null
        }
    },
    @{
        label = "chain_stats"
        url = "$ApiUrl/v1/public/chain/stats"
        validate = {
            param($json)
            if ($null -eq $json.tip_height) { return "missing tip_height" }
            if ($json.tip_height -lt $MinTipHeight) {
                return "tip_height $($json.tip_height) below minimum $MinTipHeight"
            }
            return $null
        }
    },
    @{
        label = "consensus_state"
        url = "$ApiUrl/v1/consensus/state"
        validate = {
            param($json)
            if ($null -eq $json.quorum) { return "missing quorum" }
            if ($null -eq $json.active_rounds) { return "missing active_rounds" }
            return $null
        }
    },
    @{
        label = "consensus_pbft"
        url = "$ApiUrl/v1/consensus/pbft"
        validate = {
            param($json)
            if ($null -eq $json.active_round_count) { return "missing active_round_count" }
            if ($null -eq $json.rounds) { return "missing rounds array" }
            return $null
        }
    }
)

if (-not [string]::IsNullOrWhiteSpace($PackageCanonical)) {
    $encoded = [uri]::EscapeDataString($PackageCanonical)
    $checks += @{
        label = "package_status"
        url = "$ApiUrl/v1/public/packages/$encoded"
        validate = {
            param($json)
            if ($json.status -ne "verified") {
                return "package status is $($json.status), expected verified"
            }
            return $null
        }
    }
}

$failed = 0
foreach ($check in $checks) {
    try {
        $raw = curl.exe -sS -L --max-time $TimeoutSecs $check.url 2>&1
        if ($LASTEXITCODE -ne 0) {
            Log "FAIL $($check.label): curl error: $raw"
            $failed++
            continue
        }
        $json = $raw | ConvertFrom-Json
        $err = & $check.validate $json
        if ($err) {
            Log "FAIL $($check.label): $err"
            $failed++
        } else {
            Log "PASS $($check.label)"
        }
    } catch {
        Log "FAIL $($check.label): $($_.Exception.Message)"
        $failed++
    }
}

if ($failed -gt 0) {
    Log "FAILED $failed check(s)"
    exit 1
}

Log "All checks passed"
exit 0
