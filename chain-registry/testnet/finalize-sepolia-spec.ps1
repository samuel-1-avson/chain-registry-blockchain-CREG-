# Chain Registry - Finalize Sepolia Chain Spec
# Run this AFTER deploy-sepolia.ps1 completes successfully.
#
# Usage:
#   .\testnet\finalize-sepolia-spec.ps1

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Write-Step($msg) {
    Write-Host ""
    Write-Host "=== $msg ===" -ForegroundColor Cyan
}

function Write-Success($msg) {
    Write-Host "[OK] $msg" -ForegroundColor Green
}

# Step 1: Read deployment manifest
$manifestPath = Join-Path $repoRoot "contracts\deployments\sepolia-latest.json"
if (-not (Test-Path $manifestPath)) {
    Write-Error "Deployment manifest not found at $manifestPath. Run deploy-sepolia.ps1 first."
}

$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
Write-Success "Loaded deployment manifest"

# Step 2: Patch chain-spec.sepolia.json
$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
$signingKeyEnvName = & (Join-Path $scriptDir "require-spec-signing-key.ps1") -SpecPath $specPath
$spec = Get-Content $specPath -Raw | ConvertFrom-Json

$spec.contracts.governance        = $manifest.governance
$spec.contracts.registry          = $manifest.registry
$spec.contracts.staking           = $manifest.staking
$spec.contracts.reputation        = $manifest.reputation
$spec.contracts.creg_token        = $manifest.cregToken
$spec.contracts.zk_verifier       = $manifest.zkVerifier
$spec.contracts.appeal            = $manifest.appeal
$spec.contracts.validator_rewards = $manifest.validatorRewards
$spec.contracts.vrf               = $manifest.vrf

if ($manifest.stakingDeployBlock) {
    $spec.validator_set.epoch_block_height = [int64]$manifest.stakingDeployBlock
    Write-Success "Set validator_set.epoch_block_height to $($manifest.stakingDeployBlock)"
}

# Update genesis time to now
$spec.genesis_time = (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")

# Save patched spec without UTF-8 BOM
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$specJson = $spec | ConvertTo-Json -Depth 20 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, $utf8NoBom)
Write-Success "Patched contract addresses into chain-spec.sepolia.json"

$env:CARGO_TERM_QUIET = "true"

# Step 3: Compute genesis hash
Write-Step "Computing genesis hash"
$genesisHash = cargo run -q --example compute_genesis_hash --package common -- $specPath | Select-Object -Last 1
if (-not $genesisHash) {
    Write-Warning "compute_genesis_hash example not found. Using CLI fallback."
    $genesisHash = cargo run -q --package chain-registry-cli -- chain-spec compute-genesis-hash $specPath | Select-Object -Last 1
}
Write-Success "Genesis hash: $genesisHash"

# Update genesis hash in spec
$spec = Get-Content $specPath -Raw | ConvertFrom-Json
$spec.genesis_hash = $genesisHash
$specJson = $spec | ConvertTo-Json -Depth 20 -Compress
[System.IO.File]::WriteAllText($specPath, $specJson, $utf8NoBom)
Write-Success "Updated genesis_hash in spec"

# Step 4: Sign the spec
Write-Step "Signing chain spec"
$sig = cargo run -q --example sign_chain_spec --package common -- $specPath --key-env $signingKeyEnvName | Select-Object -Last 1
if (-not $sig -or $sig -match "Error") {
    throw "sign_chain_spec failed: $sig"
}

$sigPath = Join-Path $scriptDir "chain-spec.sepolia.json.sig"
[System.IO.File]::WriteAllText($sigPath, $sig.Trim(), $utf8NoBom)
Write-Success "Signature saved to chain-spec.sepolia.json.sig"

# Step 5: Verify signature
Write-Step "Verifying signature"
cargo run -q --example verify_chain_spec --package common -- $specPath $sigPath | Select-Object -Last 1
Write-Success "Signature verified"

# Step 6: Sync to spec-server directory
$specServerDir = Join-Path $scriptDir "spec-server"
if (Test-Path $specServerDir) {
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.sepolia.json"), $specJson, $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.sepolia.json.sig"), $sig.Trim(), $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.json"), $specJson, $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $specServerDir "chain-spec.json.sig"), $sig.Trim(), $utf8NoBom)
    Write-Success "Synced chain spec and signature to spec-server"
}

# Step 7: Summary
Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host "  Sepolia Chain Spec Ready for Publication                  " -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host ""
Write-Host "Files:"
Write-Host "  Spec:      $specPath"
Write-Host "  Signature: $sigPath"
Write-Host ""
Write-Host "Genesis Hash: $genesisHash"
