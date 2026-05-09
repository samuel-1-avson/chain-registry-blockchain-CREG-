# Chain Registry Sepolia Deployment Script (PowerShell)
# Deploys Token and Staking contracts to Ethereum Sepolia.

$envFile = "testnet/.env.sepolia"
if (-not (Test-Path $envFile)) {
    Write-Error "Error: $envFile not found."
    return
}

# Parse .env.sepolia manually to avoid issues with complex values
$envVars = @{}
Get-Content $envFile | Where-Object { $_ -match "^[^#].+=.+" } | ForEach-Object {
    $parts = $_ -split "=", 2
    $envVars[$parts[0].Trim()] = $parts[1].Trim()
}

$rpcUrl = $envVars["SEPOLIA_RPC_URL"]
$deployerKey = $envVars["DEPLOYER_KEY"]

if (-not $rpcUrl -or -not $deployerKey) {
    Write-Error "Error: SEPOLIA_RPC_URL and DEPLOYER_KEY must be set in $envFile"
    return
}

Write-Host "🚀 Deploying to Sepolia..." -ForegroundColor Cyan
Write-Host "RPC URL: $rpcUrl"

# Run deployment using forge inside a dedicated deployment container
# Note: We override the entrypoint to ensure arguments are passed correctly to the shell
docker compose --env-file .env.local-testnet -f docker-compose.local-testnet.yml run --rm --entrypoint "" -e DEPLOYER_KEY=$deployerKey deploy-contracts /bin/sh -c "forge script testnet/Deploy.s.sol:DeployScript --rpc-url $rpcUrl --broadcast --slow"

$manifestPath = "testnet/artifacts/testnet-contracts.json"
if (Test-Path $manifestPath) {
    $manifest = Get-Content $manifestPath | ConvertFrom-Json
    $tokenAddr = $manifest.token
    $stakingAddr = $manifest.staking
    
    Write-Host "✅ Deployment successful!" -ForegroundColor Green
    Write-Host "Token: $tokenAddr"
    Write-Host "Staking: $stakingAddr"
    
    Write-Host "`nTo finish the connection, run these commands to update your explorer image:" -ForegroundColor Yellow
    Write-Host "docker compose --env-file .env.local-testnet -f docker-compose.local-testnet.yml build web-explorer"
    Write-Host "docker compose --env-file .env.local-testnet -f docker-compose.local-testnet.yml up -d web-explorer"
} else {
    Write-Error "❌ Deployment failed or manifest not found."
}
