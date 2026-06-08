# Patch chain-spec.sepolia.json service URLs for the local public lab stack,
# recompute genesis hash, and re-sign the spec.
#
# Usage:
#   .\testnet\patch-sepolia-chain-spec-services.ps1
#   .\testnet\patch-sepolia-chain-spec-services.ps1 -PublicHost "testnet.example.com"

param(
    [string]$PublicHost = "localhost"
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
$sigPath = Join-Path $scriptDir "chain-spec.sepolia.json.sig"

if (-not (Test-Path $specPath)) {
    throw "Missing $specPath"
}

$spec = Get-Content $specPath -Raw | ConvertFrom-Json

# Bootnodes are overridden by CREG_P2P_SEEDS in the 3-node fleet; empty avoids dialing placeholders.
$spec.bootnodes = @()

$nodeApiPort = if ($env:CREG_3NODE_NODE3_API_PORT) { $env:CREG_3NODE_NODE3_API_PORT } else { "28182" }
$ipfsPort = if ($env:CREG_3NODE_IPFS_HOST_PORT) { $env:CREG_3NODE_IPFS_HOST_PORT } else { "15001" }
$specPort = if ($env:CREG_3NODE_SPEC_HOST_PORT) { $env:CREG_3NODE_SPEC_HOST_PORT } else { "18888" }
$faucetPort = if ($env:CREG_3NODE_FAUCET_PORT) { $env:CREG_3NODE_FAUCET_PORT } else { "8082" }
$explorerPort = if ($env:CREG_3NODE_EXPLORER_PORT) { $env:CREG_3NODE_EXPLORER_PORT } else { "3007" }

$hostBase = if ($PublicHost -eq "localhost") { "http://localhost" } else { "https://$PublicHost" }

$spec.services = [ordered]@{
    ipfs_gateway = "$hostBase`:$ipfsPort"
    ipfs_api     = "$hostBase`:$ipfsPort"
    faucet       = "$hostBase`:$faucetPort"
    explorer     = "$hostBase`:$explorerPort"
    metrics      = "$hostBase`:$nodeApiPort/metrics"
}

$spec.support.discord = "https://github.com/chain-registry/chain-registry/discussions"
$spec.support.security = "security@chain-registry.github.io"
$spec.signing.detached_signature_url = "$hostBase`:$specPort/chain-spec.json.sig"

# Phase must stay a known enum value (alpha | beta | ga).
$spec.phase = "alpha"

$specJson = $spec | ConvertTo-Json -Depth 30 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, [System.Text.UTF8Encoding]::new($false))
Write-Host "Patched services in $specPath (host=$PublicHost)" -ForegroundColor Green

Write-Host "Computing genesis hash..." -ForegroundColor Cyan
$genesisHash = cargo run -q --example compute_genesis_hash --package common -- $specPath 2>&1 | Select-Object -Last 1
if (-not $genesisHash -or $genesisHash -match "error") {
    throw "compute_genesis_hash failed: $genesisHash"
}
$spec = Get-Content $specPath -Raw | ConvertFrom-Json
$spec.genesis_hash = $genesisHash.Trim()
$specJson = $spec | ConvertTo-Json -Depth 30 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, [System.Text.UTF8Encoding]::new($false))
Write-Host "genesis_hash = $($spec.genesis_hash)" -ForegroundColor Green

Write-Host "Signing chain spec..." -ForegroundColor Cyan
$privkey = "9d91e9e0d82a02b7be8c40a522d899eea9eeffad244323be3e568973211f3a6d"
$sig = cargo run -q --example sign_chain_spec --package common -- $specPath $privkey 2>&1 | Select-Object -Last 1
Set-Content -Path $sigPath -Value $sig.Trim() -NoNewline
Write-Host "Wrote $sigPath" -ForegroundColor Green

cargo run -q --example verify_chain_spec --package common -- $specPath $sigPath 2>&1 | Select-Object -Last 3
