# Deploy Prometheus + Alertmanager on the edge VM and scrape validator fleet /metrics.
#
# Generates testnet/monitoring/prometheus-gcp.yml from the validator fleet internal IP,
# syncs the repo to the edge VM, and starts docker compose.
#
# Usage:
#   .\testnet\gcp\deploy-monitoring.ps1
#   .\testnet\gcp\deploy-monitoring.ps1 -SkipSync
#   .\testnet\gcp\deploy-monitoring.ps1 -ValidatorInternalIp 10.128.0.5

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$ValidatorInternalIp = "",
    [switch]$SkipSync
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$monitoringDir = Join-Path $testnetDir "monitoring"
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

function Log($m) { Write-Host "[deploy-monitoring] $m" }

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }

$envFile = Join-Path $testnetDir "sepolia-3node.env"
$fleetMode = $true
if (Test-Path $envFile) {
    $fleetMode = Select-String -Path $envFile -Pattern '^\s*CREG_VALIDATOR_FLEET_MODE\s*=\s*true' -Quiet
}

if (-not $ValidatorInternalIp) {
    if (Test-Path $envFile) {
        foreach ($line in Get-Content $envFile) {
            if ($line -match '^\s*CREG_VALIDATOR_VM_INTERNAL_IP\s*=\s*(\S+)') {
                $ValidatorInternalIp = $matches[1].Trim()
                break
            }
        }
    }
    if (-not $ValidatorInternalIp) {
        $ipLine = & (Join-Path $gcpDir "get-validator-fleet-internal-ip.ps1") | Where-Object { $_ -match '^CREG_VALIDATOR_VM_INTERNAL_IP=' }
        if ($ipLine) {
            $ValidatorInternalIp = ($ipLine -split '=', 2)[1].Trim()
        }
    }
}

if ($fleetMode -and -not $ValidatorInternalIp) {
    throw "Validator fleet internal IP unknown. Set CREG_VALIDATOR_VM_INTERNAL_IP in sepolia-3node.env or pass -ValidatorInternalIp."
}

$targets = @()
if ($fleetMode) {
    $targets = @(
        "${ValidatorInternalIp}:28180",
        "${ValidatorInternalIp}:28181",
        "${ValidatorInternalIp}:28182"
    )
    Log "Fleet mode: scraping validator VM $ValidatorInternalIp (28180-28182)"
} else {
    $targets = @("127.0.0.1:28180", "127.0.0.1:28181", "127.0.0.1:28182")
    Log "Co-located stack mode: scraping localhost 28180-28182"
}

$targetYaml = ($targets | ForEach-Object { "          - `"$_`"" }) -join "`n"
$prometheusPath = Join-Path $monitoringDir "prometheus-gcp.yml"
$prometheusBody = @"
global:
  scrape_interval: 15s
  evaluation_interval: 15s
  external_labels:
    cluster: creg-testnet-gcp
    environment: public-alpha

alerting:
  alertmanagers:
    - static_configs:
        - targets: ["alertmanager:9093"]

rule_files:
  - "alerts.yml"

scrape_configs:
  - job_name: creg-node
    static_configs:
      - targets:
$targetYaml
        labels:
          role: validator-fleet
    metrics_path: /metrics
    scrape_interval: 15s
    scrape_timeout: 10s

  - job_name: prometheus
    static_configs:
      - targets: ["localhost:9090"]
"@
Set-Content -Path $prometheusPath -Value $prometheusBody -Encoding utf8NoBOM
Log "Wrote $prometheusPath"

if (-not $SkipSync) {
    Log "Syncing repo to edge VM $VmName..."
    & (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName
} else {
    Log "Skipping full repo sync (-SkipSync); uploading monitoring config only."
    $repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
    $sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
    $remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
    $remoteMonitoring = "$remoteHome/creg-hosting/$repoSlug/chain-registry/testnet/monitoring"
    gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$remoteMonitoring'" | Out-Null
    foreach ($f in @("prometheus-gcp.yml", "creg-alerts.yml", "alertmanager-gcp.yml", "docker-compose.monitoring.yml")) {
        $local = Join-Path $monitoringDir $f
        if (-not (Test-Path $local)) { throw "Missing $local" }
        gcloud compute scp $local "${VmName}:${remoteMonitoring}/$f" @sshOpts
    }
}

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteComposeDir = "`$HOME/creg-hosting/$repoSlug/chain-registry/testnet/monitoring"
$remoteCmd = @"
set -euo pipefail
cd $remoteComposeDir
DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  if sudo docker info >/dev/null 2>&1; then
    DOCKER=(sudo docker)
  else
    echo 'Docker not available on edge VM' >&2
    exit 1
  fi
fi
`${DOCKER[@]} compose -f docker-compose.monitoring.yml pull
`${DOCKER[@]} compose -f docker-compose.monitoring.yml up -d
`${DOCKER[@]} compose -f docker-compose.monitoring.yml ps
"@

Log "Starting monitoring stack on $VmName..."
& (Join-Path $gcpDir "ssh-vm.ps1") -Command $remoteCmd
if ($LASTEXITCODE -ne 0) { throw "remote monitoring start failed" }

Log "Done. Verify:"
Log "  .\testnet\gcp\verify-monitoring.ps1"
Log "  .\testnet\gcp\ssh-vm.ps1 -Command 'curl -fsS http://127.0.0.1:9090/-/healthy'"
