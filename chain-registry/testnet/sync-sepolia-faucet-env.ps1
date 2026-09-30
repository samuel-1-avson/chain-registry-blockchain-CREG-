# Point .env.sepolia.faucet at the current Sepolia CREG token (after redeploy).
#
# Usage:
#   .\testnet\sync-sepolia-faucet-env.ps1

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$envFile = Join-Path $scriptDir ".env.sepolia.faucet"
$manifestPath = Join-Path $repoRoot "testnet\phase-c\deployment-manifest.json"
$legacyManifestPath = Join-Path $repoRoot "contracts\deployments\sepolia-latest.json"
$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"

if (-not (Test-Path $envFile)) {
    throw "Missing $envFile. Run .\testnet\setup-sepolia-faucet.ps1"
}

$token = $null
$deploymentTxHash = $null
$deploymentBlock = $null
if (Test-Path $manifestPath) {
    $manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
    $token = $manifest.cregToken
    $deploymentTxHash = $manifest.cregTokenDeploymentTxHash
    $deploymentBlock = $manifest.cregTokenDeploymentBlock
} elseif (Test-Path $legacyManifestPath) {
    $manifest = Get-Content $legacyManifestPath -Raw | ConvertFrom-Json
    $token = $manifest.cregToken
    $deploymentTxHash = $manifest.cregTokenDeploymentTxHash
    $deploymentBlock = $manifest.cregTokenDeploymentBlock
} elseif (Test-Path $specPath) {
    $token = (Get-Content $specPath -Raw | ConvertFrom-Json).contracts.creg_token
} else {
    throw "No sepolia-latest.json or chain-spec.sepolia.json"
}

if ($token -notmatch '^0x[a-fA-F0-9]{40}$') { throw "Selected deployment manifest has no valid CREG token address" }
if ($deploymentTxHash -notmatch '^0x[a-fA-F0-9]{64}$') {
    throw "Selected deployment manifest must include cregTokenDeploymentTxHash before faucet reconciliation can run"
}
if ($deploymentBlock -notmatch '^\d+$') {
    throw "Selected deployment manifest must include cregTokenDeploymentBlock before faucet reconciliation can run"
}

$values = [ordered]@{
    FAUCET_TOKEN_CONTRACT = $token
    FAUCET_TOKEN_DEPLOYMENT_TX_HASH = $deploymentTxHash
    FAUCET_TOKEN_DEPLOYMENT_BLOCK = $deploymentBlock
    FAUCET_RECONCILIATION_RPC_URL = 'https://ethereum-sepolia-rpc.publicnode.com'
    FAUCET_DEPLOYMENT_RECEIPT_RPC_URL = 'https://1rpc.io/sepolia'
    FAUCET_RECONCILIATION_CONFIRMATIONS = '64'
}

$lines = Get-Content $envFile
$oldToken = $null
foreach ($name in $values.Keys) {
    $out = New-Object System.Collections.Generic.List[string]
    $replaced = $false
    foreach ($line in $lines) {
        if ($line -match "^\s*$([regex]::Escape($name))\s*=") {
            if (-not $replaced) {
                if ($name -eq 'FAUCET_TOKEN_CONTRACT') {
                    $oldToken = ($line -replace '^\s*FAUCET_TOKEN_CONTRACT\s*=\s*', '').Trim()
                }
                $out.Add("$name=$($values[$name])")
                $replaced = $true
            }
        } else {
            $out.Add($line)
        }
    }
    if (-not $replaced) { $out.Add("$name=$($values[$name])") }
    $lines = $out
}
$lines | Set-Content -Path $envFile -Encoding utf8

if ($oldToken -and ($oldToken.ToLower() -ne $token.ToLower())) {
    Write-Host "Updated FAUCET_TOKEN_CONTRACT" -ForegroundColor Green
    Write-Host "  was: $oldToken"
    Write-Host "  now: $token"
} else {
    Write-Host "FAUCET_TOKEN_CONTRACT already current: $token" -ForegroundColor Green
}
Write-Host "Faucet recovery source pinned to token deployment block $deploymentBlock" -ForegroundColor Green
