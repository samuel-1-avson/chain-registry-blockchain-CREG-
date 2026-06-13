# Verify GCP Prometheus is scraping CREG nodes and MAL-001 sandbox metrics are present.
#
# Usage:
#   .\testnet\gcp\verify-monitoring.ps1

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = ""
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

function Log($m) { Write-Host "[verify-monitoring] $m" }
function Pass($m) { Write-Host "  PASS  $m" -ForegroundColor Green }
function Fail($m) { Write-Host "  FAIL  $m" -ForegroundColor Red; throw $m }

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }

$remoteCmd = @'
set -euo pipefail
prom_ok=0
if curl -fsS http://127.0.0.1:9090/-/healthy >/dev/null 2>&1; then prom_ok=1; fi
if [ "$prom_ok" -ne 1 ]; then
  echo "PROMETHEUS_UNHEALTHY"
  exit 2
fi
echo "PROMETHEUS_OK"
targets_json=$(curl -fsS 'http://127.0.0.1:9090/api/v1/targets')
echo "$targets_json" | grep -q '"health":"up"' || { echo "NO_UP_TARGETS"; exit 3; }
echo "TARGETS_UP"
metrics=$(curl -fsS 'http://127.0.0.1:9090/api/v1/query?query=creg_chain_tip_height')
echo "$metrics" | grep -q '"status":"success"' || { echo "METRICS_QUERY_FAILED"; exit 4; }
echo "METRICS_OK"
sandbox=$(curl -fsS 'http://127.0.0.1:9090/api/v1/query?query=creg_sandbox_dev_bypass')
echo "$sandbox" | grep -q '"status":"success"' || { echo "SANDBOX_METRICS_MISSING"; exit 5; }
echo "SANDBOX_METRICS_OK"
alerts=$(curl -fsS 'http://127.0.0.1:9090/api/v1/rules' )
echo "$alerts" | grep -q 'CregSandboxDevBypass' || { echo "ALERT_RULES_MISSING"; exit 6; }
echo "ALERT_RULES_OK"
'@

Log "Checking Prometheus on $VmName (IAP)..."
$out = & (Join-Path $gcpDir "ssh-vm.ps1") -Command $remoteCmd 2>&1
$text = ($out | Out-String).Trim()
Write-Host $text

if ($text -notmatch 'PROMETHEUS_OK') { Fail "Prometheus not healthy on edge VM" }
Pass "Prometheus healthy"

if ($text -notmatch 'TARGETS_UP') { Fail "No scrape targets in UP state" }
Pass "At least one creg-node target UP"

if ($text -notmatch 'METRICS_OK') { Fail "creg_chain_tip_height query failed" }
Pass "Chain metrics present"

if ($text -notmatch 'SANDBOX_METRICS_OK') {
    Fail "creg_sandbox_dev_bypass not scraped yet — redeploy fleet with MAL-001 metrics build"
}
Pass "MAL-001 sandbox metrics present"

if ($text -notmatch 'ALERT_RULES_OK') { Fail "creg-alerts.yml not loaded" }
Pass "Alert rules loaded (incl. CregSandboxDevBypass)"

Log "All monitoring checks passed."
