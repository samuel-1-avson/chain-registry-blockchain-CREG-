# Print SEPOLIA_RPC_URL / CREG_ETH_RPC for sepolia-3node.env (internal Geth VM).
#
# Usage:
#   .\testnet\gcp\get-sepolia-geth-rpc-url.ps1

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$statePath = Join-Path $gcpDir "sepolia-geth-state.json"
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

$rpcUrl = $null
if (Test-Path $statePath) {
    $state = Get-Content $statePath -Raw | ConvertFrom-Json
    $rpcUrl = $state.rpcUrl
}

if (-not $rpcUrl) {
    $ipName = if ($cfg.GCP_SEPOLIA_GETH_INTERNAL_IP_NAME) { $cfg.GCP_SEPOLIA_GETH_INTERNAL_IP_NAME } else { "creg-sepolia-geth-internal-ip" }
    $region = $cfg.GCP_REGION
    $project = $cfg.GCP_PROJECT
    $internalIp = (gcloud compute addresses describe $ipName --region=$region --project=$project --format="get(address)").Trim()
    $rpcUrl = "http://${internalIp}:8545"
}

Write-Host "Add to testnet/sepolia-3node.env:"
Write-Host ""
Write-Host "SEPOLIA_RPC_URL=$rpcUrl"
Write-Host "CREG_ETH_RPC=$rpcUrl"
Write-Host ""
Write-Host "Then: .\testnet\gcp\push-env.ps1 ; .\testnet\gcp\deploy-stack.ps1"
