# Smoke checks for hub-api and explorer nginx proxy paths.
#
# Usage:
#   .\testnet\hub-explorer-smoke.ps1
#   .\testnet\hub-explorer-smoke.ps1 -BaseDomain testnet.cregnet.dev

param(
    [string]$BaseDomain = "testnet.cregnet.dev"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

function Log($m) { Write-Host "[hub-explorer-smoke] $m" }
function Fail($m) { Log "FAIL $m"; exit 1 }

$hubHealthUrl = "https://$BaseDomain/api/health"
$hubStatusUrl = "https://$BaseDomain/api/status/public"
$explorerHealthUrl = "https://explorer.$BaseDomain/v1/public/health"

$checks = @(
    @{
        label = "hub_api_health"
        url = $hubHealthUrl
        validate = {
            param($json)
            if ($json.service -ne "hub-api") { return "unexpected service" }
            if ($json.phase -ne "1") { return "expected phase 1, got $($json.phase)" }
            if ($json.db -notin @("ready", "not_configured")) {
                return "unexpected db state: $($json.db)"
            }
            return $null
        }
    },
    @{
        label = "hub_api_status_public"
        url = $hubStatusUrl
        validate = {
            param($json)
            if ($json.service -ne "hub-api") { return "unexpected service" }
            if ($json.phase -ne "1") { return "expected phase 1" }
            if (-not $json.upstreams) { return "missing upstreams" }
            foreach ($probe in $json.upstreams) {
                if ($probe.PSObject.Properties.Name -contains "url") {
                    return "public status must not expose upstream URLs"
                }
            }
            return $null
        }
    },
    @{
        label = "explorer_public_health_proxy"
        url = $explorerHealthUrl
        validate = {
            param($json)
            if (-not $json.status) { return "missing status field" }
            return $null
        }
    }
)

$failed = 0
foreach ($check in $checks) {
    try {
        $raw = curl.exe -sS -L --max-time 25 $check.url 2>&1
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
        Log "FAIL $($check.label): $_"
        $failed++
    }
}

if ($failed -gt 0) {
    Fail "$failed check(s) failed"
}

Log "All hub-api and explorer proxy smoke checks passed"
exit 0
