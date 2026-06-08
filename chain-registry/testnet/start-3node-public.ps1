# Start 3-node Sepolia fleet + public lab services (faucet + explorer).
#
# Usage:
#   .\testnet\start-3node-public.ps1
#   .\testnet\start-3node-public.ps1 -PatchChainSpec
#   .\testnet\start-3node-public.ps1 -WithFaucet   # requires FAUCET_PRIVATE_KEY + FAUCET_ADDRESS in sepolia-3node.env

param(
    [switch]$PatchChainSpec,
    [switch]$WithFaucet,
    [switch]$FreshVolumes
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

if ($PatchChainSpec) {
    & (Join-Path $scriptDir "patch-sepolia-chain-spec-services.ps1")
}

$startArgs = @()
if ($FreshVolumes) { $startArgs += "-FreshVolumes" }
& (Join-Path $scriptDir "start-3node-test.ps1") @startArgs

Set-Location $repoRoot

$composeBase = Join-Path $scriptDir "docker-compose.3node.yml"
$composeServices = Join-Path $scriptDir "docker-compose.3node-services.yml"
$envFile = Join-Path $scriptDir "sepolia-3node.env"

$services = @("web-explorer")
if ($WithFaucet) {
    $services = @("faucet") + $services
}

Write-Host ""
Write-Host "=== Starting public lab services: $($services -join ', ') ===" -ForegroundColor Cyan
docker compose -f $composeBase -f $composeServices --env-file $envFile up -d --build @services

Write-Host ""
Write-Host "Public lab endpoints:" -ForegroundColor Green
Write-Host "  Node API (observer): http://localhost:28182"
Write-Host "  Explorer:            http://localhost:3007"
Write-Host "  Faucet:              http://localhost:8082  (if -WithFaucet and keys configured)"
Write-Host "  Chain spec:          http://localhost:18888/chain-spec.json"
Write-Host ""
Write-Host "Set before publishing:" -ForegroundColor Yellow
Write-Host "  `$env:CREG_NODE_URL = 'http://localhost:28182'"
