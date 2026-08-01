# Chain Registry — Start Single Local Node (Windows PowerShell)
# Usage: ./start-local-node.ps1 [-ValidatorKey <hex>] [-NodeBinary <path>]
#
# This starts a single chain-registry node against Sepolia using the signed chain spec.
# No Docker required. Prefers the packaged release binaries; cargo is a
# developer-only fallback when no release binary is available.

param(
    [string]$ValidatorKey = "",
    [string]$NodeId = "local-test",
    [string]$DataDir = "./node-data",
    [string]$RpcUrl = "https://ethereum-sepolia-rpc.publicnode.com",
    [string]$NodeBinary = "",
    [switch]$Validator
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path "$PSScriptRoot/.."

Write-Host "═══════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "  Chain Registry — Local Node Starter" -ForegroundColor Cyan
Write-Host "═══════════════════════════════════════════════════════" -ForegroundColor Cyan

# Resolve release binaries when available. Source builds remain a developer
# fallback, but a clean operator machine should use the packaged binaries.
$nodeCommand = $null
$cliCommand = $null
if ($NodeBinary) {
    if (-not (Test-Path $NodeBinary)) { throw "Node binary not found: $NodeBinary" }
    $nodeCommand = (Resolve-Path $NodeBinary).Path
    $cliCandidate = Join-Path (Split-Path $nodeCommand -Parent) "creg.exe"
    if (Test-Path $cliCandidate) { $cliCommand = (Resolve-Path $cliCandidate).Path }
} else {
    $nodeOnPath = Get-Command creg-node -ErrorAction SilentlyContinue
    $cliOnPath = Get-Command creg -ErrorAction SilentlyContinue
    if ($nodeOnPath) { $nodeCommand = $nodeOnPath.Source }
    if ($cliOnPath) { $cliCommand = $cliOnPath.Source }
}

# Check prerequisites
Write-Host "`n[1/5] Checking prerequisites..." -ForegroundColor Yellow

$hasCargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $hasCargo -and -not $nodeCommand) {
    Write-Error "Rust/Cargo not found. Install from https://rustup.rs/"
    exit 1
}
if ($hasCargo) { Write-Host "  ✓ Cargo found: $(cargo --version)" }
if ($nodeCommand) { Write-Host "  ✓ Release node: $nodeCommand" }
if ($cliCommand) { Write-Host "  ✓ Release CLI:  $cliCommand" }

# Check chain spec exists
$specPath = "$repoRoot/testnet/chain-spec.sepolia.json"
if (-not (Test-Path $specPath)) {
    Write-Error "Chain spec not found: $specPath"
    exit 1
}
Write-Host "  ✓ Chain spec found"

# Check signature exists
$sigPath = "$repoRoot/testnet/chain-spec.sepolia.json.sig"
if (-not (Test-Path $sigPath)) {
    throw "Signature file not found: $sigPath. Refusing to start without a signed chain spec."
}

# Create data directory
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
Write-Host "  ✓ Data directory: $(Resolve-Path $DataDir)"

# Set environment variables
Write-Host "`n[2/5] Configuring environment..." -ForegroundColor Yellow

$env:CARGO_MANIFEST_DIR = "$repoRoot"
$env:CREG_CHAIN_SPEC_URL = "file:///$($specPath -replace '\\','/')"
$env:CREG_SPEC_SIGNING_PUBKEY = "3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01"
$env:CREG_ETH_RPC_URL = $RpcUrl
$env:CREG_NODE_ID = $NodeId
$env:CREG_DATA_DIR = (Resolve-Path $DataDir).Path
$env:CREG_LISTEN = "0.0.0.0:8080"
$env:CREG_P2P_LISTEN = "/ip4/0.0.0.0/tcp/9000"
$env:CREG_P2P_SEEDS = ""   # Empty = you are the bootstrap
$env:CREG_IS_VALIDATOR = if ($Validator) { "true" } else { "false" }
$env:RUST_LOG = "info,chain_registry_node=debug"

if ($Validator -and $ValidatorKey) {
    $env:CREG_VALIDATOR_KEY = $ValidatorKey
    Write-Host "  ✓ Validator mode ENABLED"
} elseif ($Validator) {
    throw "Validator mode requires -ValidatorKey matching an identity in the signed chain spec."
} else {
    Write-Host "  ✓ Full node mode (non-validator)"
}

Write-Host "  ✓ Chain spec: $($env:CREG_CHAIN_SPEC_URL)"
Write-Host "  ✓ L1 RPC: $RpcUrl"
Write-Host "  ✓ Node ID: $NodeId"

# Verify chain spec signature
Write-Host "`n[3/5] Verifying chain spec..." -ForegroundColor Yellow
try {
    $signature = (Get-Content $sigPath -Raw).Trim()
    if ($signature -notmatch '^[0-9a-fA-F]{128}$') {
        throw "Detached signature is not 64-byte hexadecimal"
    }
    if ($cliCommand) {
        $verifyOutput = & $cliCommand chain-spec validate "$specPath" --signature "$sigPath" --pubkey $env:CREG_SPEC_SIGNING_PUBKEY --require-signature 2>&1
    } else {
        $verifyOutput = & cargo run --locked --example verify_chain_spec --package common --quiet -- "$specPath" $signature 2>&1
    }
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  ✓ Chain spec signature VALID" -ForegroundColor Green
    } else {
        throw "Chain spec signature verification failed: $($verifyOutput -join ' ')"
    }
} catch {
    throw "Refusing to start: chain spec signature verification failed. $_"
}

# Compute genesis hash
Write-Host "`n[4/5] Computing genesis hash..." -ForegroundColor Yellow
try {
    if ($cliCommand) {
        $genesisHash = & $cliCommand chain-spec compute-genesis-hash "$specPath" 2>$null
    } else {
        $genesisHash = & cargo run --locked --example compute_genesis_hash --package common --quiet -- "$specPath" 2>$null
    }
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace(($genesisHash -join ''))) {
        throw "genesis hash computation failed"
    }
    $env:CREG_GENESIS_HASH = ($genesisHash | Select-Object -Last 1).ToString().Trim()
    Write-Host "  ✓ Genesis hash: $genesisHash"
} catch {
    throw "Refusing to start: could not compute genesis hash. $_"
}

# Start node
Write-Host "`n[5/5] Starting node..." -ForegroundColor Yellow
Write-Host "  Press Ctrl+C to stop`n" -ForegroundColor Gray
Write-Host "  API:     http://localhost:8080/v1/health" -ForegroundColor Cyan
Write-Host "  Metrics: http://localhost:8080/metrics" -ForegroundColor Cyan
Write-Host "  gRPC:    localhost:50051" -ForegroundColor Cyan
Write-Host "  P2P:     localhost:9000`n" -ForegroundColor Cyan

Set-Location $repoRoot
if ($nodeCommand) {
    & $nodeCommand
} else {
    cargo run --locked --bin creg-node --package chain-registry-node
}
