# Print creg-testnet-vm VPC internal IP for sepolia-3node.env (CREG_EDGE_INTERNAL_IP).
#
# Usage:
#   .\testnet\gcp\get-edge-internal-ip.ps1

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

$project = $cfg.GCP_PROJECT
$zone = $cfg.GCP_ZONE
$vmName = $cfg.GCP_VM_NAME

$internalIp = (gcloud compute instances describe $vmName --zone=$zone --project=$project --format="get(networkInterfaces[0].networkIP)").Trim()

Write-Host "CREG_EDGE_INTERNAL_IP=$internalIp"
Write-Host "CREG_IPFS_URL=http://${internalIp}:15001"
Write-Host "CREG_CHAIN_SPEC_URL=http://${internalIp}:18888/chain-spec.json"
