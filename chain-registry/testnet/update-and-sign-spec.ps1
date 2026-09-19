# Multi-Bootnode Chain Spec Update & Signing Script
#
# Adds secondary bootnode to chain-spec.sepolia.json, recomputes genesis hash,
# signs using SPEC_SIGNING_KEY from Google Secret Manager, and deploys to spec server.
#
# Usage:
#   .\testnet\update-and-sign-spec.ps1

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
$sigPath  = Join-Path $scriptDir "chain-spec.sepolia.json.sig"

Write-Host ""
Write-Host "=== CREG Multi-Bootnode Specification Update ===" -ForegroundColor Cyan

# 1. Fetch SPEC_SIGNING_KEY from Google Secret Manager
Write-Host "Step 1: Fetching SPEC_SIGNING_KEY from Google Secret Manager..." -ForegroundColor Cyan
$gcloud = if (Get-Command gcloud.cmd -ErrorAction SilentlyContinue) { "gcloud.cmd" } else { "gcloud" }
$keySecret = & $gcloud secrets versions access latest --secret=creg-testnet-spec-signing-key 2>&1 | Out-String
$keySecret = $keySecret.Trim()
if ($keySecret.Length -ne 64) {
    throw "Invalid SPEC_SIGNING_KEY fetched from Secret Manager (expected 64 hex chars)"
}
$env:SPEC_SIGNING_KEY = $keySecret
Write-Host "  -> Successfully loaded SPEC_SIGNING_KEY from Secret Manager." -ForegroundColor Green

# 2. Update bootnodes in chain-spec.sepolia.json
Write-Host "Step 2: Updating bootnodes array in $specPath..." -ForegroundColor Cyan
$spec = Get-Content $specPath -Raw | ConvertFrom-Json

$spec.bootnodes = @(
    [PSCustomObject]@{
        id = "bootnode-gcp-1"
        operator = "CREG Foundation Seed"
        region = "us-central1-b"
        multiaddr = "/ip4/35.225.225.20/tcp/9000/p2p/12D3KooWDro5WW69YcAhFpqNSx9aGho3NdhU5LShjxSZjCFFE5XJ"
    },
    [PSCustomObject]@{
        id = "bootnode-gcp-2"
        operator = "CREG Secondary Observer Seed"
        region = "us-central1-a"
        multiaddr = "/ip4/35.225.225.20/tcp/9001/p2p/12D3KooWGjQs1eqYHXZaaKzK3x5BaHt89YjRGmRPgwdtwc4Kcjiy"
    }
)

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$specJson = $spec | ConvertTo-Json -Depth 20 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, $utf8NoBom)
Write-Host "  -> Updated bootnodes count: $($spec.bootnodes.Count)" -ForegroundColor Green

# 3. Compute genesis hash
Write-Host "Step 3: Computing genesis hash..." -ForegroundColor Cyan
$genesisHash = cargo run -q --example compute_genesis_hash --package common -- $specPath | Select-Object -Last 1
if (-not $genesisHash) {
    $genesisHash = cargo run -q --package chain-registry-cli -- chain-spec compute-genesis-hash $specPath | Select-Object -Last 1
}
Write-Host "  -> Genesis Hash: $genesisHash" -ForegroundColor Green

$spec = Get-Content $specPath -Raw | ConvertFrom-Json
$spec.genesis_hash = $genesisHash.Trim()
$specJson = $spec | ConvertTo-Json -Depth 20 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, $utf8NoBom)

# 4. Sign spec
Write-Host "Step 4: Cryptographically signing chain spec with Ed25519..." -ForegroundColor Cyan
$sig = cargo run -q --example sign_chain_spec --package common -- $specPath --key-env SPEC_SIGNING_KEY | Select-Object -Last 1
if (-not $sig -or $sig -match "Error") {
    throw "sign_chain_spec failed: $sig"
}
[System.IO.File]::WriteAllText($sigPath, $sig.Trim(), $utf8NoBom)
Write-Host "  -> Signature created: $sig" -ForegroundColor Green

# 5. Verify signature
Write-Host "Step 5: Verifying signature..." -ForegroundColor Cyan
$verify = cargo run -q --example verify_chain_spec --package common -- $specPath $sigPath | Select-Object -Last 1
Write-Host "  -> $verify" -ForegroundColor Green

# 6. Deploy to spec server and upload to GCP Edge VM
Write-Host "Step 6: Syncing to local spec-server and GCP Edge VM..." -ForegroundColor Cyan
$specServerDir = Join-Path $scriptDir "spec-server"
if (Test-Path $specServerDir) {
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.json"), $specJson, $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.json.sig"), $sig.Trim(), $utf8NoBom)
}

$vmName = "creg-testnet-vm"
$zone = "us-central1-b"
$projectId = "gen-lang-client-0022105784"
$remoteDir = "/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet"

Write-Host "Uploading updated signed spec to $vmName..." -ForegroundColor DarkGray
& $gcloud compute scp $specPath "${vmName}:${remoteDir}/chain-spec.sepolia.json" --zone=$zone --project=$projectId --quiet
& $gcloud compute scp $sigPath  "${vmName}:${remoteDir}/chain-spec.sepolia.json.sig" --zone=$zone --project=$projectId --quiet

# Restart spec container so nginx picks up immediately
& $gcloud compute ssh $vmName --zone=$zone --project=$projectId --command="sudo docker restart creg-unified-spec-server" --quiet

Start-Sleep -Seconds 3
Write-Host ""
Write-Host "=== Verification on Public Network ===" -ForegroundColor Cyan
$publicSpec = Invoke-RestMethod -Uri "https://spec.testnet.cregnet.dev/chain-spec.json"
Write-Host "Public Genesis Hash : $($publicSpec.genesis_hash)"
Write-Host "Public Bootnodes    : $($publicSpec.bootnodes.Count)"
foreach ($b in $publicSpec.bootnodes) {
    Write-Host "  - [$($b.id)] ($($b.region)): $($b.multiaddr)" -ForegroundColor Green
}

if ($publicSpec.bootnodes.Count -ge 2) {
    Write-Host ""
    Write-Host "[SUCCESS] Multi-Bootnode topology is now active and publicly verifiable!" -ForegroundColor Green
} else {
    Write-Warning "Spec server has not refreshed yet; check in 10 seconds."
}
