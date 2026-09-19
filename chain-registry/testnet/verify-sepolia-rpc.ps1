# Verify Sepolia L1 RPC endpoint connectivity and readiness.
# Supports Google Cloud Blockchain RPC, Infura, Alchemy, or any EIP-1474 provider.
#
# Usage:
#   .\testnet\verify-sepolia-rpc.ps1
#   .\testnet\verify-sepolia-rpc.ps1 -RpcUrl "https://us-central1-ethereum-sepolia.blockchain.goog/v1/projects/.../endpoints/...?key=..."
#   .\testnet\verify-sepolia-rpc.ps1 -CheckDeployer

param(
    [string]$RpcUrl,
    [switch]$CheckDeployer
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

function Log($msg) { Write-Host "[sepolia-rpc] $msg" }
function Pass($msg) { Write-Host "[sepolia-rpc] OK   $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "[sepolia-rpc] WARN $msg" -ForegroundColor Yellow }
function Fail($msg) { Write-Host "[sepolia-rpc] FAIL $msg" -ForegroundColor Red; exit 1 }

# If RpcUrl is not explicitly passed, try loading from .env.sepolia or sepolia-fleet.env
if (-not $RpcUrl) {
    $envSepolia = Join-Path $scriptDir ".env.sepolia"
    $envFleet = Join-Path $scriptDir "sepolia-fleet.env"
    
    if (Test-Path $envSepolia) {
        Get-Content $envSepolia | ForEach-Object {
            if ($_ -match '^\s*SEPOLIA_RPC_URL\s*=\s*(.*)\s*$') {
                $candidate = $matches[1].Trim().Trim('"').Trim("'")
                if ($candidate -and $candidate -notmatch 'YOUR_') {
                    $RpcUrl = $candidate
                }
            }
        }
    }
    
    if (-not $RpcUrl -and (Test-Path $envFleet)) {
        Get-Content $envFleet | ForEach-Object {
            if ($_ -match '^\s*SEPOLIA_RPC_URL\s*=\s*(.*)\s*$') {
                $candidate = $matches[1].Trim().Trim('"').Trim("'")
                if ($candidate -and $candidate -notmatch 'YOUR_') {
                    $RpcUrl = $candidate
                }
            }
        }
    }
}

if (-not $RpcUrl) {
    Fail "No RPC URL provided. Pass -RpcUrl or set SEPOLIA_RPC_URL in testnet/.env.sepolia"
}

# Obfuscate API key / secret in log output
$maskedUrl = $RpcUrl -replace 'key=[^&]+', 'key=***' -replace 'v3/[a-fA-F0-9]{20,}', 'v3/***' -replace 'v2/[a-fA-F0-9]{20,}', 'v2/***'
Log "Testing Sepolia RPC endpoint: $maskedUrl"

# 1. Test eth_chainId (must be 0xaa36a7 / 11155111 for Sepolia)
$bodyChainId = '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
try {
    $resp = Invoke-RestMethod -Uri $RpcUrl -Method POST -ContentType "application/json" -Body $bodyChainId -TimeoutSec 15
    $sw.Stop()
    if (-not $resp.result) {
        Fail "eth_chainId missing result: $($resp | ConvertTo-Json -Compress)"
    }
    $chainIdDec = [Convert]::ToInt64($resp.result, 16)
    if ($chainIdDec -ne 11155111) {
        Fail "Expected Sepolia chain ID 11155111 (0xaa36a7), got $chainIdDec ($($resp.result))"
    }
    Pass "Chain ID: 11155111 (Sepolia) [latency: $($sw.ElapsedMilliseconds)ms]"
} catch {
    Fail "Failed to call eth_chainId: $_"
}

# 2. Test eth_blockNumber
$bodyBlock = '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":2}'
try {
    $respBlock = Invoke-RestMethod -Uri $RpcUrl -Method POST -ContentType "application/json" -Body $bodyBlock -TimeoutSec 15
    if (-not $respBlock.result) {
        Fail "eth_blockNumber missing result"
    }
    $blockNum = [Convert]::ToInt64($respBlock.result, 16)
    Pass "Latest block: $blockNum ($($respBlock.result))"
} catch {
    Fail "Failed to call eth_blockNumber: $_"
}

# 3. Test eth_gasPrice
$bodyGas = '{"jsonrpc":"2.0","method":"eth_gasPrice","params":[],"id":3}'
try {
    $respGas = Invoke-RestMethod -Uri $RpcUrl -Method POST -ContentType "application/json" -Body $bodyGas -TimeoutSec 15
    if ($respGas.result) {
        $gasPriceGwei = [Math]::Round(([Convert]::ToInt64($respGas.result, 16) / 1e9), 2)
        Pass "Current gas price: $gasPriceGwei Gwei"
    }
} catch {
    Warn "Could not fetch eth_gasPrice (non-fatal)"
}

# 4. Check deployer balance if available
$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"
$toolsCast = Join-Path $scriptDir ".tools\foundry\cast.exe"
$cast = if (Test-Path $toolsCast) { $toolsCast } elseif (Get-Command cast -ErrorAction SilentlyContinue) { (Get-Command cast).Source } else { $null }

$deployerAddr = $null
$envSepolia = Join-Path $scriptDir ".env.sepolia"
if (Test-Path $envSepolia) {
    Get-Content $envSepolia | ForEach-Object {
        if ($_ -match '^\s*GOVERNANCE_SIGNER_ADDRESS\s*=\s*(0x[a-fA-F0-9]{40})') {
            $deployerAddr = $matches[1]
        }
    }
}

if ($deployerAddr -and $cast) {
    try {
        $balRaw = (& $cast balance $deployerAddr --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
        $balWei = if ($balRaw -match '(\d+)') { $matches[1] } else { "0" }
        $balEth = [Math]::Round(([decimal]$balWei / 1e18), 6)
        if ([decimal]$balWei -gt 0) {
            Pass "Deployer ($deployerAddr) balance: $balEth Sepolia ETH ($balWei wei)"
        } else {
            Warn "Deployer ($deployerAddr) balance is 0. Fund with Google Cloud Faucet before deploying."
        }
    } catch {
        Warn "Could not check deployer balance: $_"
    }
}

Log "Sepolia RPC endpoint verification successful!"
