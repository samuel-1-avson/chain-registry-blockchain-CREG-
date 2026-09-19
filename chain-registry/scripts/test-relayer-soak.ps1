# scripts/test-relayer-soak.ps1
# Automated contract and state root relayer soak drill
# Anchors finalized PBFT state roots to Sepolia contracts using L1RelayerService

[CmdletBinding()]
param (
    [int]$Count = 100,
    [int]$StartHeight = 1,
    [string]$RpcUrl = "https://ethereum-sepolia-rpc.publicnode.com",
    [string]$ContractAddress = "0xF232cC6F5C6EfC74C7517B209D15baBd57b817eE",
    [switch]$LiveBroadcast,
    [switch]$Json
)

$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "  CREG L1 CONTRACT & STATE ROOT RELAYER SOAK TEST DRILL" -ForegroundColor Cyan -NoNewline
Write-Host " (Sepolia)" -ForegroundColor Yellow
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "  Contract:      $ContractAddress" -ForegroundColor Gray
Write-Host "  Sepolia RPC:   $RpcUrl" -ForegroundColor Gray
Write-Host "  Checkpoints:   $Count batches" -ForegroundColor Green
Write-Host "  Start Height:  #$StartHeight" -ForegroundColor Green
Write-Host "  Mode:          $(if ($LiveBroadcast) { 'Live On-Chain Broadcast' } else { 'Simulated Dry-Run (Cryptographic / Calldata Verification)' })" -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host ""

$cargoArgs = @(
    "run", "--bin", "creg", "--",
    "relayer", "soak",
    "--count", $Count.ToString(),
    "--start-height", $StartHeight.ToString(),
    "--rpc-url", $RpcUrl,
    "--contract", $ContractAddress
)

if ($LiveBroadcast) {
    $cargoArgs += "--broadcast"
}

if ($Json) {
    $cargoArgs += "--json"
}

Write-Host "Executing L1RelayerService soak drill..." -ForegroundColor Yellow
& cargo $cargoArgs

if ($LASTEXITCODE -ne 0) {
    Write-Error "Relayer soak drill failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "[OK] L1 Relayer Soak Drill execution completed successfully." -ForegroundColor Green
