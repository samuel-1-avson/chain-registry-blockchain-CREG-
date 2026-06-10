# Deploy Sepolia Geth docker-compose on creg-sepolia-geth-vm.
#
# Usage:
#   .\testnet\gcp\deploy-sepolia-geth.ps1

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$composeDir = Join-Path $testnetDir "sepolia-geth"
$composeFile = Join-Path $composeDir "docker-compose.yml"

function Log($m) { Write-Host "[deploy-sepolia-geth] $m" }

if (-not (Test-Path $composeFile)) {
    throw "Missing $composeFile"
}

$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
$vmName = if ($cfg.GCP_SEPOLIA_GETH_VM_NAME) { $cfg.GCP_SEPOLIA_GETH_VM_NAME } else { "creg-sepolia-geth-vm" }
$zone = $cfg.GCP_ZONE
$project = $cfg.GCP_PROJECT
$remoteDir = "/opt/creg-sepolia-geth"

Log "Packaging compose to $vmName..."
$tarPath = Join-Path $env:TEMP "creg-sepolia-geth.tgz"
if (Test-Path $tarPath) { Remove-Item $tarPath -Force }
Push-Location $composeDir
tar -czf $tarPath docker-compose.yml
Pop-Location

$remoteTar = "/tmp/creg-sepolia-geth.tgz"
gcloud compute scp $tarPath "${vmName}:${remoteTar}" `
    --zone=$zone --project=$project --tunnel-through-iap `
    --strict-host-key-checking=no --quiet
if ($LASTEXITCODE -ne 0) { throw "scp failed" }

$remoteCmd = @"
set -euo pipefail
sudo mkdir -p $remoteDir
sudo tar -xzf $remoteTar -C $remoteDir
rm -f $remoteTar
cd $remoteDir
sudo docker compose pull
sudo docker compose up -d
sudo docker compose ps
"@ -replace "`r", ""

& (Join-Path $gcpDir "ssh-sepolia-geth.ps1") -Command $remoteCmd
if ($LASTEXITCODE -ne 0) { throw "remote deploy failed" }

Log "Geth starting. Sync may take hours - check eth_syncing via ssh-sepolia-geth.ps1"
