# Expand Governance Multisig on Sepolia (1-of-1 -> 2-of-3 Threshold)
#
# Adds two independent co-signers and raises the execution threshold to 2.
# Once executed, no single key can alter governance parameters or execute proposals unilaterally.
#
# Usage:
#   .\testnet\expand-governance-multisig.ps1

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"
$toolsCast = Join-Path $scriptDir ".tools\foundry\cast.exe"
$castCmd = Get-Command cast -ErrorAction SilentlyContinue
$cast = if (Test-Path $toolsCast) { $toolsCast } elseif ($castCmd) { $castCmd.Source } else { $null }
if (-not $cast) { throw "cast.exe not found at $toolsCast" }

function Import-DotEnv {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return }
    Get-Content $Path | ForEach-Object {
        if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
            $k = $matches[1].Trim()
            $v = $matches[2].Trim().Trim('"')
            if ($v -and $v -notmatch 'YOUR_') {
                Set-Item -Path "Env:$k" -Value $v
            }
        }
    }
}

Import-DotEnv (Join-Path $scriptDir ".env.sepolia")

$RpcUrl = $env:SEPOLIA_RPC_URL
if (-not $RpcUrl) { $RpcUrl = "https://ethereum-sepolia-rpc.publicnode.com" }
$GovAddr = $env:CREG_GOVERNANCE_ADDR
if (-not $GovAddr) { $GovAddr = "0x122Bc2C704B966C74004506F98F13bf7b5F75b9D" }
$AuthorityKey = $env:GOVERNANCE_SIGNER_KEY
if (-not $AuthorityKey) { $AuthorityKey = $env:DEPLOYER_KEY }
if (-not $AuthorityKey) { throw "Missing GOVERNANCE_SIGNER_KEY or DEPLOYER_KEY in .env.sepolia" }

$cosignersPath = Join-Path $scriptDir "secrets\governance-cosigners.json"
if (-not (Test-Path $cosignersPath)) { throw "Missing $cosignersPath" }
$cosignersConfig = Get-Content $cosignersPath -Raw | ConvertFrom-Json

$signer1Addr = $cosignersConfig.signers[0].address
$signer2Addr = $cosignersConfig.signers[1].address
$signer3Addr = $cosignersConfig.signers[2].address

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " CREG Governance Decentralization: 1-of-1 -> 2-of-3 Multi-Sig " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "Governance Contract : $GovAddr"
Write-Host "RPC Endpoint        : $RpcUrl"
Write-Host "Founding Signer 1   : $signer1Addr"
Write-Host "New Co-Signer 2     : $signer2Addr"
Write-Host "New Co-Signer 3     : $signer3Addr"
Write-Host ""

function Call-Contract {
    param([string]$Sig, [string[]]$MethodArgs = @())
    $cmdArgs = @("call", $GovAddr, $Sig) + $MethodArgs + @("--rpc-url", $RpcUrl)
    $res = & $cast @cmdArgs 2>&1 | Out-String
    return $res.Trim()
}

function Send-Tx {
    param([string]$Sig, [string[]]$MethodArgs = @())
    $cmdArgs = @("send", $GovAddr, $Sig) + $MethodArgs + @("--private-key", $AuthorityKey, "--rpc-url", $RpcUrl)
    $res = & $cast @cmdArgs 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "Transaction failed: $res"
    }
    return $res
}

function Submit-And-Vote {
    param([string]$Target, [string]$Calldata, [string]$Description)
    $propCountRaw = Call-Contract "proposalCount()(uint256)"
    $propId = [uint64]($propCountRaw -replace '\D', '')
    Write-Host "Submitting Proposal #$propId : $Description ..." -ForegroundColor Yellow
    
    Send-Tx "submit(address,bytes,string)" @($Target, $Calldata, $Description) | Out-Null
    Write-Host "  -> Proposal #$propId submitted. Voting with Authority..." -ForegroundColor DarkGray
    
    Send-Tx "vote(uint256,bool)" @($propId.ToString(), "true") | Out-Null
    Write-Host "  -> Vote cast & executed for Proposal #$propId!" -ForegroundColor Green
    Start-Sleep -Seconds 3
}

$curThresholdRaw = Call-Contract "threshold()(uint256)"
$curThreshold = [uint64]($curThresholdRaw -replace '\D', '')
$curSignersRaw = Call-Contract "signerCount()(uint256)"
$curSigners = [uint64]($curSignersRaw -replace '\D', '')

Write-Host "Initial Status:"
Write-Host "  Signer Count: $curSigners"
Write-Host "  Threshold   : $curThreshold"
Write-Host ""

# 1. Add Signer 2 if needed
$isSigner2Raw = Call-Contract "isSigner(address)(bool)" -MethodArgs @($signer2Addr)
if ($isSigner2Raw -notmatch "true") {
    Write-Host "Action: Adding Signer 2 ($signer2Addr)..." -ForegroundColor Cyan
    $calldata2 = (& $cast calldata "addSigner(address)" $signer2Addr 2>&1 | Out-String).Trim()
    Submit-And-Vote -Target $GovAddr -Calldata $calldata2 -Description "Add Co-Signer Alpha ($signer2Addr)"
} else {
    Write-Host "Signer 2 ($signer2Addr) is already registered." -ForegroundColor Green
}

# 2. Add Signer 3 if needed
$isSigner3Raw = Call-Contract "isSigner(address)(bool)" -MethodArgs @($signer3Addr)
if ($isSigner3Raw -notmatch "true") {
    Write-Host "Action: Adding Signer 3 ($signer3Addr)..." -ForegroundColor Cyan
    $calldata3 = (& $cast calldata "addSigner(address)" $signer3Addr 2>&1 | Out-String).Trim()
    Submit-And-Vote -Target $GovAddr -Calldata $calldata3 -Description "Add Co-Signer Beta ($signer3Addr)"
} else {
    Write-Host "Signer 3 ($signer3Addr) is already registered." -ForegroundColor Green
}

# 3. Update Threshold to 2 if needed
$curThresholdRaw = Call-Contract "threshold()(uint256)"
$curThreshold = [uint64]($curThresholdRaw -replace '\D', '')
if ($curThreshold -lt 2) {
    Write-Host "Action: Updating Threshold to 2..." -ForegroundColor Cyan
    $calldataThresh = (& $cast calldata "updateThreshold(uint256)" 2 2>&1 | Out-String).Trim()
    Submit-And-Vote -Target $GovAddr -Calldata $calldataThresh -Description "Set Multi-Sig Threshold to 2-of-3"
} else {
    Write-Host "Threshold is already $curThreshold." -ForegroundColor Green
}

# Final Verification
Write-Host ""
Write-Host "=== Final Multisig Verification ===" -ForegroundColor Cyan
$finalThresholdRaw = Call-Contract "threshold()(uint256)"
$finalThreshold = [uint64]($finalThresholdRaw -replace '\D', '')
$finalSignersRaw = Call-Contract "signerCount()(uint256)"
$finalSigners = [uint64]($finalSignersRaw -replace '\D', '')

Write-Host "Final Signer Count: $finalSigners (Expected: 3)"
Write-Host "Final Threshold   : $finalThreshold (Expected: 2)"

$ok1 = (Call-Contract "isSigner(address)(bool)" -MethodArgs @($signer1Addr)) -match "true"
$ok2 = (Call-Contract "isSigner(address)(bool)" -MethodArgs @($signer2Addr)) -match "true"
$ok3 = (Call-Contract "isSigner(address)(bool)" -MethodArgs @($signer3Addr)) -match "true"

Write-Host "  Signer 1 Verified: $ok1"
Write-Host "  Signer 2 Verified: $ok2"
Write-Host "  Signer 3 Verified: $ok3"

if ($finalSigners -eq 3 -and $finalThreshold -eq 2 -and $ok1 -and $ok2 -and $ok3) {
    Write-Host ""
    Write-Host "[SUCCESS] Governance contract is now a true decentralized 2-of-3 Multi-Sig!" -ForegroundColor Green
} else {
    throw "Multisig expansion did not meet all verification criteria"
}
