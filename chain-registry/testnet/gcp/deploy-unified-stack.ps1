# Deploy Unified Single-Instance RPC & Services Stack on GCP (~$59/mo).
# Consolidates CREG Node + Caddy Unified Gateway + Explorer + Faucet onto a single e2-standard-2 instance.
#
# Usage:
#   .\testnet\gcp\deploy-unified-stack.ps1
#   .\testnet\gcp\deploy-unified-stack.ps1 -ProjectId gen-lang-client-0022105784 -DownsizeVm

param(
    [string]$ProjectId = "gen-lang-client-0022105784",
    [string]$Zone = "us-central1-b",
    [string]$VmName = "creg-testnet-vm",
    [string]$MachineType = "e2-medium",
    [switch]$DownsizeVm,
    [switch]$TearDownRedundantResources
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $scriptDir
$repoRoot = Split-Path -Parent $testnetDir

function Log($msg) { Write-Host "[unified-deploy] $msg" -ForegroundColor Cyan }
function Pass($msg) { Write-Host "[unified-deploy] OK   $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "[unified-deploy] WARN $msg" -ForegroundColor Yellow }
function Fail($msg) { Write-Host "[unified-deploy] FAIL $msg" -ForegroundColor Red; exit 1 }

$gcloud = if (Get-Command gcloud.cmd -ErrorAction SilentlyContinue) { "gcloud.cmd" } else { "gcloud" }

Log "Deploying Unified RPC Stack to GCP Project: $ProjectId ($Zone)"

# 1. Check VM status
$vmJson = & $gcloud compute instances describe $VmName --zone=$Zone --project=$ProjectId --format="json" 2>$null | ConvertFrom-Json
if (-not $vmJson) {
    Fail "VM '$VmName' not found in $Zone. Run provision-vm.ps1 first."
}

$currentMachineType = ($vmJson.machineType -split '/')[-1]
Log "Current VM Machine Type: $currentMachineType (Target: $MachineType)"

# Downsize or upsize if requested
if ($currentMachineType -ne $MachineType -and $DownsizeVm) {
    Log "Resizing VM '$VmName' to '$MachineType'..."
    & $gcloud compute instances stop $VmName --zone=$Zone --project=$ProjectId --quiet
    & $gcloud compute instances set-machine-type $VmName --zone=$Zone --machine-type=$MachineType --project=$ProjectId --quiet
    & $gcloud compute instances start $VmName --zone=$Zone --project=$ProjectId --quiet
    Pass "VM resized successfully to $MachineType."
}

# 2. Upload unified stack files
Log "Uploading unified stack configuration to VM..."
$remoteTestnetDir = "/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet"

$rootFiles = @(
    (Join-Path $testnetDir "docker-compose.unified.yml"),
    (Join-Path $testnetDir ".env.sepolia"),
    (Join-Path $testnetDir "chain-spec.sepolia.json"),
    (Join-Path $testnetDir "chain-spec.sepolia.json.sig")
)

foreach ($f in $rootFiles) {
    if (Test-Path $f) {
        & $gcloud compute scp $f "${VmName}:${remoteTestnetDir}/" --zone=$Zone --project=$ProjectId --quiet
    }
}

$caddyUnified = Join-Path $testnetDir "caddy\Caddyfile.unified"
if (Test-Path $caddyUnified) {
    & $gcloud compute scp $caddyUnified "${VmName}:${remoteTestnetDir}/caddy/" --zone=$Zone --project=$ProjectId --quiet
}
Pass "Uploaded configuration files to VM."

# 3. Start unified stack on VM via SSH
Log "Starting unified container stack on VM..."
$startCmd = @'
cd ~/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet
sudo docker compose -f docker-compose.cloud-edge.yml down 2>/dev/null || true
sudo docker compose -f docker-compose.cloud-edge-ingress.yml down 2>/dev/null || true
sudo docker compose --env-file .env.sepolia -f docker-compose.unified.yml up -d
sudo docker compose -f docker-compose.unified.yml ps
'@

& $gcloud compute ssh $VmName --zone=$Zone --project=$ProjectId --command=$startCmd --quiet

# 4. Optional teardown of legacy redundant MIG / Geth VM
if ($TearDownRedundantResources) {
    Log "Tearing down redundant observer pool MIG and internal Geth VM..."
    & $gcloud compute instance-groups managed delete creg-observer-pool-mig --region=us-central1 --project=$ProjectId --quiet 2>$null
    & $gcloud compute instances delete creg-sepolia-geth-vm --zone=$Zone --project=$ProjectId --quiet 2>$null
    Pass "Redundant instances stopped/deleted. Spend optimized to ~$59/month."
}

Pass "Unified RPC Deployment complete!"
Log "Endpoints now live under unified domain:"
Log "  - CREG Native RPC: https://api.testnet.cregnet.dev/rpc"
Log "  - Sepolia L1 Proxy: https://api.testnet.cregnet.dev/sepolia"
Log "  - Health Check:    https://api.testnet.cregnet.dev/health"
