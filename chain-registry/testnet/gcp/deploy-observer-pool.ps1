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
    --zone=$Zone --project=$ProjectId --format="value(instance)" 2>$null | Where-Object { $_ })

if ($instances.Count -eq 0) {
    throw "No instances in MIG $MigName. Run provision-observer-pool.ps1 -Confirm first."
}

if (-not $Confirm) {
    Write-Host "Will SSH to $($instances.Count) observer VM(s) and run start-observer-pool-gcp.sh" -ForegroundColor Yellow
    Write-Host "Re-run with -Confirm." -ForegroundColor Yellow
    exit 0
}

$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
& (Join-Path $gcpDir "push-env.ps1") -ProjectId $ProjectId -Zone $Zone -VmName ($instances[0] -replace ".*/instances/", "")

foreach ($uri in $instances) {
    $vm = ($uri -split "/")[-1]
    Log "Deploying on $vm ..."
    gcloud compute scp (Join-Path $testnetDir "sepolia-3node.env") "${vm}:/tmp/sepolia-3node.env" `
        --zone=$Zone --project=$ProjectId --quiet
    gcloud compute ssh $vm --zone=$Zone --project=$ProjectId --quiet --command=@"
sudo mkdir -p /opt/chain-registry/chain-registry/testnet
sudo cp /tmp/sepolia-3node.env /opt/chain-registry/chain-registry/testnet/sepolia-3node.env 2>/dev/null || true
cd /opt/chain-registry/chain-registry && git pull --ff-only 2>/dev/null || true
./testnet/start-observer-pool-gcp.sh
"@
}

Log "Done. Verify ILB health and edge Caddy upstream."
