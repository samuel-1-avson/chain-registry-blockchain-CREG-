# Redeploy Sepolia using ONLY the authority wallet in testnet/.env.sepolia
# (DEPLOYER_KEY = GOVERNANCE_SIGNER_KEY). Abandons the lost 0xf4c0... deployment.
#
# Prerequisites:
#   .\testnet\setup-sepolia-authority.ps1
#   Sepolia ETH on GOVERNANCE_SIGNER_ADDRESS
#
# Usage:
#   .\testnet\deploy-sepolia-new-authority.ps1
#   .\testnet\deploy-sepolia-new-authority.ps1 -SkipFinalize

param(
    [switch]$SkipFinalize,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$envFile = Join-Path $scriptDir ".env.sepolia"
$legacyDeployer = "0xf4c0bdBB681A61Aa0B123E82C04b0d692F53D58e"

if (-not (Test-Path $envFile)) {
    throw "Missing $envFile — run .\testnet\setup-sepolia-authority.ps1 first"
}

Get-Content $envFile | ForEach-Object {
    if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
        [Environment]::SetEnvironmentVariable($matches[1].Trim(), $matches[2].Trim().Trim('"'), "Process")
    }
}

if (-not $env:GOVERNANCE_SIGNER_KEY -or -not $env:DEPLOYER_KEY) {
    throw "Run .\testnet\setup-sepolia-authority.ps1 to set DEPLOYER_KEY and GOVERNANCE_SIGNER_KEY"
}
if ($env:DEPLOYER_KEY.Trim() -ne $env:GOVERNANCE_SIGNER_KEY.Trim()) {
    throw "DEPLOYER_KEY and GOVERNANCE_SIGNER_KEY must match. Run setup-sepolia-authority.ps1"
}

$cast = Join-Path $scriptDir ".tools\foundry\cast.exe"
if (-not (Test-Path $cast)) { $cast = (Get-Command cast -ErrorAction Stop).Source }
$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"

$addrRaw = & $cast wallet address --private-key $env:DEPLOYER_KEY 2>&1 | Out-String
if ($addrRaw -notmatch '(0x[a-fA-F0-9]{40})') { throw "Invalid DEPLOYER_KEY" }
$authorityAddr = $matches[1]

if ($env:GOVERNANCE_SIGNER_ADDRESS -and ($env:GOVERNANCE_SIGNER_ADDRESS.Trim().ToLower() -ne $authorityAddr.ToLower())) {
    throw "GOVERNANCE_SIGNER_ADDRESS does not match DEPLOYER_KEY. Run setup-sepolia-authority.ps1"
}

$rpc = $env:SEPOLIA_RPC_URL
if (-not $rpc) { $rpc = "https://ethereum-sepolia-rpc.publicnode.com" }

Write-Host ""
Write-Host "=== Redeploy Sepolia with NEW authority ===" -ForegroundColor Cyan
Write-Host "Authority:  $authorityAddr"
Write-Host "Abandons:   $legacyDeployer (old deployer, not used)"
Write-Host ""

$manifestPath = Join-Path $repoRoot "contracts\deployments\sepolia-latest.json"
if ((Test-Path $manifestPath) -and -not $Yes) {
    $old = Get-Content $manifestPath -Raw | ConvertFrom-Json
    Write-Host "Current manifest deployer: $($old.deployer)" -ForegroundColor Yellow
    Write-Host "This will REPLACE sepolia-latest.json and patch chain-spec.sepolia.json." -ForegroundColor Yellow
    $confirm = Read-Host "Type yes to continue"
    if ($confirm -ne "yes") { throw "Cancelled" }
}

# Archive old manifest if it pointed at legacy deployer
if (Test-Path $manifestPath) {
    $old = Get-Content $manifestPath -Raw | ConvertFrom-Json
    if ($old.deployer -eq $legacyDeployer) {
        $archive = Join-Path $repoRoot "contracts\deployments\sepolia-legacy-0xf4c0.json"
        Copy-Item -Force $manifestPath $archive
        Write-Host "Archived old manifest -> $archive" -ForegroundColor DarkGray
    }
}

# Optional: seed faucet from new deployer's minted supply
$faucetEnv = Join-Path $scriptDir ".env.sepolia.faucet"
if (Test-Path $faucetEnv) {
    Get-Content $faucetEnv | ForEach-Object {
        if ($_ -match '^\s*FAUCET_ADDRESS\s*=\s*(.+)\s*$') {
            $env:FAUCET_ADDRESS = $matches[1].Trim()
        }
    }
}

$env:GOVERNANCE_THRESHOLD = "1"
$env:DEPLOYER_KEY = $env:GOVERNANCE_SIGNER_KEY

Set-Location $repoRoot
& (Join-Path $scriptDir "deploy-sepolia.ps1")
if ($LASTEXITCODE -ne 0) { throw "deploy-sepolia.ps1 failed" }

$newManifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
if ($newManifest.deployer.ToLower() -ne $authorityAddr.ToLower()) {
    throw "Deploy manifest deployer $($newManifest.deployer) != authority $authorityAddr"
}

Write-Host ""
Write-Host "Verifying governance signer on new deployment..." -ForegroundColor Cyan
& (Join-Path $scriptDir "check-governance-signer.ps1")
if ($LASTEXITCODE -ne 0) { throw "Authority is not a governance signer after deploy" }

if (-not $SkipFinalize) {
    & (Join-Path $scriptDir "finalize-sepolia-spec.ps1")
}

function Set-EnvLine {
    param([string[]]$Lines, [string]$Name, [string]$Value)
    $out = New-Object System.Collections.Generic.List[string]
    $replaced = $false
    foreach ($line in $Lines) {
        if ($line -match "^\s*#?\s*$([regex]::Escape($Name))\s*=") {
            if (-not $replaced) { $out.Add("$Name=$Value"); $replaced = $true }
            continue
        }
        $out.Add($line)
    }
    if (-not $replaced) { $out.Add("$Name=$Value") }
    return $out
}
$lines = Get-Content $envFile
$lines = Set-EnvLine $lines "CREG_GOVERNANCE_ADDR" $newManifest.governance
$lines = Set-EnvLine $lines "CREG_REGISTRY_ADDR" $newManifest.registry
$lines = Set-EnvLine $lines "CREG_STAKING_ADDR" $newManifest.staking
$lines = Set-EnvLine $lines "CREG_TOKEN_ADDR" $newManifest.cregToken
$lines = Set-EnvLine $lines "CREG_ZK_VERIFIER_ADDR" $newManifest.zkVerifier
$lines | Set-Content -Path $envFile -Encoding utf8

Write-Host ""
Write-Host "Done. Authority wallet controls deploy + governance (threshold 1)." -ForegroundColor Green
Write-Host "  fund publisher: .\testnet\fund-publisher-sepolia.ps1  (uses DEPLOYER_KEY)"
Write-Host "  sync faucet:    .\testnet\sync-sepolia-faucet-env.ps1"
Write-Host "  faucet mint:    .\testnet\fund-sepolia-faucet-governance.ps1"
Write-Host "  start faucet:   .\testnet\start-sepolia-faucet.ps1"
Write-Host "  do NOT use register-governance-signer-sepolia.ps1 for this path"
