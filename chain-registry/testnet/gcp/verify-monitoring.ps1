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

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$localScript = Join-Path $gcpDir "verify-monitoring-remote.sh"
$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
$remotePath = "$remoteHome/creg-hosting/$repoSlug/chain-registry/testnet/gcp/verify-monitoring-remote.sh"
gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$(Split-Path $remotePath -Parent)'" | Out-Null
gcloud compute scp $localScript "${VmName}:${remotePath}" @sshOpts
$remoteCmd = "sed -i 's/\r$//' '$remotePath' && chmod +x '$remotePath' && bash '$remotePath'"

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
    Fail "creg_sandbox_dev_bypass not scraped yet - redeploy fleet with MAL-001 metrics build"
}
Pass "MAL-001 sandbox metrics present"

if ($text -notmatch 'ALERT_RULES_OK') { Fail "creg-alerts.yml not loaded" }
Pass "Alert rules loaded (incl. CregSandboxDevBypass)"

if ($text -match 'ALERT_RECEIVERS_OK') {
    Pass "Alertmanager receivers wired (Slack and/or PagerDuty)"
} elseif ($text -match 'ALERT_RECEIVERS_NTFY') {
    Pass "Alertmanager receivers wired (ntfy push)"
} elseif ($text -match 'ALERT_RECEIVERS_UNCONFIGURED') {
    Write-Host "  WARN  Alertmanager has no external receivers - run setup-alert-receiver.ps1" -ForegroundColor Yellow
} else {
    Fail "Could not determine Alertmanager receiver status"
}

Log "All monitoring checks passed."
