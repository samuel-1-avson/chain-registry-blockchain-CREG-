# Roll observer pool on existing MIG VMs (sync repo + start observer container).
#
# Usage:
#   .\testnet\gcp\deploy-observer-pool.ps1 -Confirm

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$MigName = "",
    [switch]$Confirm
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $MigName) { $MigName = if ($cfg.GCP_OBSERVER_MIG_NAME) { $cfg.GCP_OBSERVER_MIG_NAME } else { "creg-observer-pool" } }

function Log($m) { Write-Host "[observer-deploy] $m" }

$instances = @(gcloud compute instance-groups managed list-instances $MigName `
    --zone=$Zone --project=$ProjectId --format="value(instance.basename())" 2>$null | Where-Object { $_ -and $_ -notmatch '^us-central1' })

if ($instances.Count -eq 0) {
    throw "No instances in MIG $MigName. Run provision-observer-pool.ps1 -Confirm first."
}

if (-not $Confirm) {
    Write-Host "Will SSH to $($instances.Count) observer VM(s) and run start-observer-pool-gcp.sh" -ForegroundColor Yellow
    Write-Host "Re-run with -Confirm." -ForegroundColor Yellow
    exit 0
}

$testnetDir = Split-Path -Parent $gcpDir

foreach ($uri in $instances) {
    $vm = ($uri -split "/")[-1]
    Log "Syncing repo to $vm ..."
    & (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $vm
    & (Join-Path $gcpDir "push-env.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $vm -TunnelThroughIap
    $repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
    Log "Starting observer on $vm ..."
    # Synced local tree may be ahead of GHCR; rebuild observer image from source.
    gcloud compute ssh $vm --zone=$Zone --project=$ProjectId --tunnel-through-iap --quiet --command="set -euo pipefail; cd ~/creg-hosting/$repoSlug/chain-registry; chmod +x testnet/start-observer-pool-gcp.sh; CREG_FLEET_BUILD=1 ./testnet/start-observer-pool-gcp.sh"
}

Log "Done. Verify ILB health and edge Caddy upstream."
