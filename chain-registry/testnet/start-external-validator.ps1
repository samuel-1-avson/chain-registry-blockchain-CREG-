# ==============================================================================
# Chain Registry (CREG) -- Independent External Validator Launch Script (Windows)
# ==============================================================================
# This script enables ANY independent operator to join the decentralized CREG
# testnet as a full validator on Windows with Docker Desktop.
# ==============================================================================

param(
    [string]$NodeId = "",
    [string]$EthRpc = "https://ethereum-sepolia-rpc.publicnode.com",
    [int]$HttpPort = 8080,
    [int]$P2pPort = 9000,
    [string]$CustomKey = ""
)

$ErrorActionPreference = "Stop"

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "   Chain Registry (CREG) -- External Validator Onboarding    " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# Check Docker
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Error "Docker is not found. Please install and start Docker Desktop."
    exit 1
}

# Resolve Node ID and isolated Data Directory (allows running multiple nodes on one host)
if (-not $NodeId) {
    $NodeId = "ext-val-primary"
    $dataDir = Join-Path $env:USERPROFILE ".creg-validator"
} else {
    $dataDir = Join-Path $env:USERPROFILE ".creg-validator-$NodeId"
}

if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir -Force | Out-Null }
# Pre-create the chain data dir: if the Docker daemon creates it on first mount it
# can land with ownership the node user cannot write, breaking the RocksDB store.
if (-not (Test-Path (Join-Path $dataDir "chain-data"))) { New-Item -ItemType Directory -Path (Join-Path $dataDir "chain-data") -Force | Out-Null }
$keyFile = Join-Path $dataDir "validator.key"
$envFile = Join-Path $dataDir "validator.env"

# Generate, load, or use custom Validator Key
if ($CustomKey) {
    $validatorKey = $CustomKey.Trim().ToLower()
    [System.IO.File]::WriteAllText($keyFile, $validatorKey)
    Write-Host "[OK] Using custom validator key (saved to $keyFile)" -ForegroundColor Green
} elseif (-not (Test-Path $keyFile)) {
    Write-Host "Generating new Ed25519 validator consensus key..." -ForegroundColor Yellow
    $bytes = New-Object byte[] 32
    (New-Object System.Security.Cryptography.RNGCryptoServiceProvider).GetBytes($bytes)
    $validatorKey = [System.BitConverter]::ToString($bytes).Replace("-", "").ToLower()
    [System.IO.File]::WriteAllText($keyFile, $validatorKey)
    Write-Host "[OK] Generated new validator key: $keyFile" -ForegroundColor Green
} else {
    $validatorKey = (Get-Content $keyFile -Raw).Trim()
    Write-Host "[OK] Loaded existing validator key from $keyFile" -ForegroundColor Green
}

$operatorKeyFile = Join-Path $dataDir "operator_api.key"
if (-not (Test-Path $operatorKeyFile)) {
    $opBytes = New-Object byte[] 32
    (New-Object System.Security.Cryptography.RNGCryptoServiceProvider).GetBytes($opBytes)
    $opKey = [System.BitConverter]::ToString($opBytes).Replace("-", "").ToLower()
    [System.IO.File]::WriteAllText($operatorKeyFile, $opKey)
} else {
    $opKey = (Get-Content $operatorKeyFile -Raw).Trim()
}

# Bootnode seed: override with $env:BOOTNODE — the default below is only correct
# while the published chain spec lists that peer. A stale ID means the node can
# never join the mesh (handshake rejected on peer-ID mismatch).
$bootnode = if ($env:BOOTNODE) { $env:BOOTNODE } else { "/ip4/35.225.225.20/tcp/9000/p2p/12D3KooWSUsWkwq8vhF7UyfGPoTAJkt6ESXuDaLKZNcR9myF8UXx" }

$envLines = @(
    "CREG_TESTNET=true",
    "CREG_NODE_ID=$NodeId",
    "CREG_DATA_DIR=/data",
    "CREG_IS_VALIDATOR=true",
    "CREG_NODE_ROLE=validator",
    "CREG_VALIDATOR_KEY=$validatorKey",
    "CREG_OPERATOR_API_KEY=$opKey",
    "CREG_ETH_RPC=$EthRpc",
    "CREG_EXPECTED_L1_CHAIN_ID=11155111",
    "CREG_CHAIN_ID=creg-testnet-1",
    "CREG_STAKING_ADDR=0x30604Aa6f763fbde6aDCffbd8960af3031037331",
    "CREG_REGISTRY_ADDR=0xD0F465ca185a99E268be0C4FF5A135Bdf89FAF3B",
    "CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/chain-spec.json",
    "CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/chain-spec.json.sig",
    "CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev",
    "CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000",
    "CREG_P2P_SEEDS=$bootnode",
    "CREG_PEERS=https://api.testnet.cregnet.dev",
    "CREG_LISTEN=0.0.0.0:8080",
    "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=$(if ($env:PBFT_SMALL_QUORUM) { $env:PBFT_SMALL_QUORUM } else { 'true' })",
    "RUST_LOG=info,chain_registry_node=debug,consensus=info"
)
$envContent = $envLines -join [Environment]::NewLine
[System.IO.File]::WriteAllText($envFile, $envContent)

# Auto-detect available Docker image
$imageCandidates = @("creg-node:genesis2", "chain-registry-node-secure:latest", "creg-node:fleet", "creg-node:latest", "creg-node:local-3node", "chain-registry-app:latest")
$chosenImage = $null
foreach ($cand in $imageCandidates) {
    $imgId = docker images -q $cand 2>$null
    if ($imgId) {
        $chosenImage = $cand
        break
    }
}

if (-not $chosenImage) {
    $chainRegistryDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $dockerfile = Join-Path $chainRegistryDir "Dockerfile"
    Write-Host "Local validator Docker image not found. Building from $dockerfile (multi-stage, no host toolchain needed; takes a while on first run)..." -ForegroundColor Yellow
    docker build -f "$dockerfile" -t chain-registry-node-secure:latest "$chainRegistryDir"
    $chosenImage = "chain-registry-node-secure:latest"
}

Write-Host ""
Write-Host "Starting independent validator container using $chosenImage..." -ForegroundColor Yellow

# Remove old container if it exists
docker rm -f "creg-validator-$NodeId" 2>$null | Out-Null

$dockerArgs = @(
    "run",
    "-d",
    "--name", "creg-validator-$NodeId",
    "--restart", "unless-stopped",
    "--env-file", "$envFile",
    "-p", "$($HttpPort):8080",
    "-p", "$($P2pPort):9000",
    "-v", "$($dataDir)/chain-data:/data",
    $chosenImage
)
& docker @dockerArgs | Out-Null

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host "   Validator Node Successfully Launched!                    " -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host "Node ID:         $NodeId" -ForegroundColor Cyan
Write-Host "Data Directory:  $dataDir" -ForegroundColor Cyan
Write-Host "Key Location:    $keyFile" -ForegroundColor Cyan
Write-Host "Docker Image:    $chosenImage" -ForegroundColor Cyan
Write-Host "P2P Port:        $P2pPort (Firewall: OPEN TCP/UDP $P2pPort)" -ForegroundColor Cyan
Write-Host "HTTP RPC Port:   $HttpPort (Keep private or proxy with TLS)" -ForegroundColor Cyan
Write-Host "Bootnode Peer:   $bootnode" -ForegroundColor Cyan
Write-Host ""
Write-Host "--- KEY BACKUP NOTICE ---" -ForegroundColor Yellow
Write-Host "Validator Key:   $validatorKey" -ForegroundColor Yellow
Write-Host "Save this key string securely! To restore this node anytime, pass:" -ForegroundColor Gray
Write-Host "  .\testnet\start-external-validator.cmd -CustomKey $validatorKey" -ForegroundColor Gray
Write-Host ""
Write-Host "--- REMOTE VERIFICATION ---" -ForegroundColor Yellow
Write-Host "Local query:     curl.exe http://localhost:$($HttpPort)/v1/health" -ForegroundColor Gray
Write-Host "Remote query:    curl.exe http://<YOUR_SERVER_IP>:$($HttpPort)/v1/health" -ForegroundColor Gray
Write-Host "Global Explorer: https://explorer.testnet.cregnet.dev (Validators Tab)" -ForegroundColor Gray
Write-Host ""
Write-Host "--- STAKING FOR REWARDS ---" -ForegroundColor Yellow
Write-Host "1. Drip testnet tokens:  creg testnet drip" -ForegroundColor Cyan
Write-Host "2. Deposit stake (100):  creg stake 100" -ForegroundColor Cyan
Write-Host "3. Monitor live logs:    docker logs -f creg-validator-$NodeId" -ForegroundColor Cyan
Write-Host ""
