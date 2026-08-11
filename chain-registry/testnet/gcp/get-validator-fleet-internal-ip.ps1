# LEGACY LAB ONLY — print the retired validator-fleet VM IP.
# Production edge/observer config must use independently operated endpoints.
#
# Usage:
#   .\testnet\gcp\get-validator-fleet-internal-ip.ps1 -AllowLegacyMaintainerSeedFleet

param(
    [switch]$AllowLegacyMaintainerSeedFleet
)

$ErrorActionPreference = "Stop"
if (-not $AllowLegacyMaintainerSeedFleet) {
    throw "The GCP validator fleet is retired. Configure CREG_OBSERVER_API_UPSTREAM and external validator peers instead."
}
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

$project = $cfg.GCP_PROJECT
$region = $cfg.GCP_REGION
$zone = $cfg.GCP_ZONE
$vmName = if ($cfg.GCP_VALIDATOR_VM_NAME) { $cfg.GCP_VALIDATOR_VM_NAME } else { "creg-validator-vm" }
$ipName = if ($cfg.GCP_VALIDATOR_INTERNAL_IP_NAME) { $cfg.GCP_VALIDATOR_INTERNAL_IP_NAME } else { "creg-validator-internal-ip" }
$statePath = Join-Path $gcpDir "validator-fleet-state.json"

$internalIp = ""
if (Test-Path $statePath) {
    $state = Get-Content $statePath -Raw | ConvertFrom-Json
    if ($state.internalIp) { $internalIp = $state.internalIp.Trim() }
}

if (-not $internalIp) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $internalIp = (gcloud compute addresses describe $ipName --region=$region --project=$project --format="get(address)" 2>$null).Trim()
    $ErrorActionPreference = $prev
}

if (-not $internalIp) {
    $internalIp = (gcloud compute instances describe $vmName --zone=$zone --project=$project --format="get(networkInterfaces[0].networkIP)").Trim()
}

Write-Host "CREG_VALIDATOR_VM_INTERNAL_IP=$internalIp"
Write-Host "CREG_VALIDATOR_FLEET_MODE=true"
Write-Host "# Node API ports on fleet VM: 28180 (node1), 28181 (node2), 28182 (node3 observer)"
