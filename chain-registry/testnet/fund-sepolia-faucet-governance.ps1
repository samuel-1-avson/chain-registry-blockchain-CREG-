# Prepare or execute one separately approved faucet allocation from Governance custody.
# The default is read-only. The Governance reserve must already be funded; this script
# never mints tokens and never uses a bridge or faucet key as a governance signer.
#
# Read-only proposal preview:
#   .\testnet\fund-sepolia-faucet-governance.ps1 -FaucetAllocationCreg 10000
# Submit a proposal only after the amount is approved in the tokenomics policy and the
# protected environment contains FAUCET_APPROVED_TRANCHE_CREG with that exact amount:
#   .\testnet\fund-sepolia-faucet-governance.ps1 -FaucetAllocationCreg 10000 -SubmitProposal
# This script does not cast approval votes. Signers review the exact proposal separately.

param(
    [Parameter(Mandatory = $true)]
    [UInt64]$FaucetAllocationCreg,
    [string]$RpcUrl = "https://ethereum-sepolia-rpc.publicnode.com",
    [string]$FaucetAddress = "",
    [switch]$SubmitProposal
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$faucetEnv = Join-Path $scriptDir ".env.sepolia.faucet"
$sepoliaEnv = Join-Path $scriptDir ".env.sepolia"

function Import-DotEnv {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return }
    Get-Content $Path | ForEach-Object {
        if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
            [Environment]::SetEnvironmentVariable($matches[1].Trim(), $matches[2].Trim().Trim('"'), "Process")
        }
    }
}

function Invoke-CastCall {
    param(
        [string]$Target,
        [string]$Signature,
        [string[]]$Arguments = @()
    )
    $blockArgs = @()
    if ($script:SnapshotBlock) { $blockArgs = @("--block", $script:SnapshotBlock) }
    $result = & $cast call $Target $Signature @Arguments @blockArgs --rpc-url $RpcUrl 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Read-only call failed: $Signature on $Target`n$result" }
    return $result.Trim()
}

function Convert-CastUInt {
    param([string]$Raw, [string]$Name)
    if ($Raw -notmatch '^\s*(\d+)\s*$') { throw "Could not parse $Name from RPC response: $Raw" }
    return [bigint]::Parse($matches[1])
}

if ($FaucetAllocationCreg -eq 0) { throw "Faucet allocation must be greater than zero." }

Import-DotEnv $faucetEnv
Import-DotEnv $sepoliaEnv

if ($FaucetAddress) { $env:FAUCET_ADDRESS = $FaucetAddress.Trim() }
if (-not $env:FAUCET_ADDRESS) {
    throw "Run .\testnet\setup-sepolia-faucet.ps1 first or pass -FaucetAddress."
}
if ($env:FAUCET_ADDRESS -notmatch '^0x[a-fA-F0-9]{40}$') {
    throw "FAUCET_ADDRESS must be a 20-byte Ethereum address."
}
$faucet = $env:FAUCET_ADDRESS

$repoRoot = Split-Path -Parent $scriptDir
$manifestPath = Join-Path $repoRoot "testnet\phase-c\deployment-manifest.json"
if (-not (Test-Path $manifestPath)) { throw "R7 deployment manifest not found: $manifestPath" }
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
$gov = [string]$manifest.governance
$token = [string]$manifest.cregToken
if ($gov -notmatch '^0x[a-fA-F0-9]{40}$' -or $token -notmatch '^0x[a-fA-F0-9]{40}$') {
    throw "R7 manifest has an invalid Governance or CREG token address."
}

$cast = Join-Path $scriptDir ".tools\foundry\cast.exe"
if (-not (Test-Path $cast)) { $cast = (Get-Command cast -ErrorAction Stop).Source }
$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"

$chainId = (& $cast chain-id --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $chainId -ne "11155111") {
    throw "Expected Sepolia chain ID 11155111, got '$chainId'."
}

$snapshotBlock = (& $cast block-number --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "Could not read the current Sepolia block number." }
$script:SnapshotBlock = $snapshotBlock
$owner = Invoke-CastCall -Target $token -Signature "owner()(address)"
if ($owner.Trim().ToLowerInvariant() -ne $gov.ToLowerInvariant()) {
    throw "R7 token owner ($owner) is not the R7 Governance contract ($gov)."
}

$totalSupply = Convert-CastUInt (Invoke-CastCall $token "totalSupply()(uint256)") "total supply"
$maxSupply = Convert-CastUInt (Invoke-CastCall $token "MAX_SUPPLY()(uint256)") "maximum supply"
$governanceBalance = Convert-CastUInt (Invoke-CastCall $token "balanceOf(address)(uint256)" @($gov)) "Governance token balance"
$faucetBalance = Convert-CastUInt (Invoke-CastCall $token "balanceOf(address)(uint256)" @($faucet)) "faucet token balance"
$threshold = Convert-CastUInt (Invoke-CastCall $gov "threshold()(uint256)") "governance threshold"
$systemStatus = Convert-CastUInt (Invoke-CastCall $gov "systemStatus()(uint8)") "governance status"
$amountWei = [bigint]$FaucetAllocationCreg * [bigint]::Pow(10, 18)

$calldata = & $cast calldata "transfer(address,uint256)" $faucet $amountWei 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Could not encode the ERC-20 transfer calldata: $calldata" }
$calldata = $calldata.Trim()
if ($calldata -notmatch '^0x') { throw "Invalid ERC-20 transfer calldata: $calldata" }
$description = "Release $FaucetAllocationCreg tCREG from governance reserve to approved faucet wallet"

Write-Host "Read-only Sepolia faucet allocation package" -ForegroundColor Cyan
Write-Host "Snapshot block:          $snapshotBlock"
Write-Host "Chain ID:                $chainId"
Write-Host "Governance:              $gov"
Write-Host "Token:                   $token"
Write-Host "Token owner:             $owner"
Write-Host "Governance threshold:    $threshold"
Write-Host "Governance status:       $(if ($systemStatus -eq 0) { 'active' } else { 'paused' })"
Write-Host "Total supply (wei):      $totalSupply"
Write-Host "Maximum supply (wei):    $maxSupply"
Write-Host "Governance reserve (wei):$governanceBalance"
Write-Host "Current faucet (wei):    $faucetBalance"
Write-Host "Recipient:               $faucet"
Write-Host "Allocation:              $FaucetAllocationCreg tCREG ($amountWei wei)"
Write-Host "Target call:             CREG token transfer(address,uint256)"
Write-Host "Calldata:                $calldata"
Write-Host "Description:             $description"

if (-not $SubmitProposal) {
    if ($governanceBalance -lt $amountWei) {
        Write-Host "Governance does not yet hold enough tCREG for this tranche; preview only, not executable." -ForegroundColor Yellow
    }
    Write-Host "Preview only. No transaction was sent. Review the policy and package before using -SubmitProposal." -ForegroundColor Yellow
    return
}

$approvedTranche = 0UL
if (-not [UInt64]::TryParse($env:FAUCET_APPROVED_TRANCHE_CREG, [ref]$approvedTranche) -or
    $approvedTranche -ne $FaucetAllocationCreg) {
    throw "-SubmitProposal requires FAUCET_APPROVED_TRANCHE_CREG to equal the approved tranche ($FaucetAllocationCreg). No transaction was sent."
}

$snapshotBlock = (& $cast block-number --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "Could not refresh the Sepolia block before proposal submission." }
$script:SnapshotBlock = $snapshotBlock
$owner = Invoke-CastCall -Target $token -Signature "owner()(address)"
if ($owner.Trim().ToLowerInvariant() -ne $gov.ToLowerInvariant()) {
    throw "Token ownership changed before submission. No transaction was sent."
}
$totalSupply = Convert-CastUInt (Invoke-CastCall $token "totalSupply()(uint256)") "current total supply"
$governanceBalance = Convert-CastUInt (Invoke-CastCall $token "balanceOf(address)(uint256)" @($gov)) "current Governance token balance"
$faucetBalance = Convert-CastUInt (Invoke-CastCall $token "balanceOf(address)(uint256)" @($faucet)) "current faucet token balance"
$threshold = Convert-CastUInt (Invoke-CastCall $gov "threshold()(uint256)") "current governance threshold"
$systemStatus = Convert-CastUInt (Invoke-CastCall $gov "systemStatus()(uint8)") "current governance status"
if ($governanceBalance -lt $amountWei) {
    throw "Governance holds insufficient tCREG at block $snapshotBlock. No transaction was sent."
}
if ($systemStatus -ne 0) { throw "Governance is paused at block $snapshotBlock. No proposal was submitted." }
Write-Host "Fresh submission preflight block: $snapshotBlock"
Write-Host "Current Governance reserve:      $governanceBalance wei"
Write-Host "Current faucet balance:          $faucetBalance wei"
Write-Host "Current total supply:            $totalSupply wei"

$signerKeys = @()
foreach ($name in @("GOVERNANCE_SIGNER_KEY", "GOVERNANCE_SIGNER_KEY_2", "GOVERNANCE_SIGNER_KEY_3")) {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ($value) { $signerKeys += $value.Trim() }
}
if ($signerKeys.Count -eq 0) { throw "No governance signer keys are configured. No transaction was sent." }

$validSigners = @()
foreach ($key in $signerKeys) {
    $addressRaw = (& $cast wallet address --private-key $key 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $addressRaw -notmatch '(0x[a-fA-F0-9]{40})') { continue }
    $signerAddress = $matches[1]
    if ($validSigners.Address -contains $signerAddress) { continue }
    $isSigner = Invoke-CastCall $gov "isSigner(address)(bool)" @($signerAddress)
    if ($isSigner.Trim().ToLowerInvariant() -eq "true") {
        $validSigners += @{ Key = $key; Address = $signerAddress }
    }
}
if ($validSigners.Count -lt 1) {
    throw "No configured key belongs to a current Governance signer. No transaction was sent."
}

$confirmation = "TRANSFER $FaucetAllocationCreg TCREG TO $($faucet.ToLowerInvariant())"
Write-Host "`nThis submits an on-chain proposal only. It does not cast approval votes or execute the transfer." -ForegroundColor Yellow
if ((Read-Host "Type exactly: $confirmation") -cne $confirmation) {
    throw "Confirmation did not match. No transaction was sent."
}

$submitter = $validSigners[0]
$snapshotBlock = (& $cast block-number --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "Could not refresh Sepolia state after confirmation. No transaction was sent." }
$script:SnapshotBlock = $snapshotBlock
$owner = Invoke-CastCall -Target $token -Signature "owner()(address)"
$governanceBalance = Convert-CastUInt (Invoke-CastCall $token "balanceOf(address)(uint256)" @($gov)) "pre-submit Governance balance"
$systemStatus = Convert-CastUInt (Invoke-CastCall $gov "systemStatus()(uint8)") "pre-submit Governance status"
$submitterIsSigner = Invoke-CastCall $gov "isSigner(address)(bool)" @($submitter.Address)
if ($owner.Trim().ToLowerInvariant() -ne $gov.ToLowerInvariant() -or
    $governanceBalance -lt $amountWei -or
    $systemStatus -ne 0 -or
    $submitterIsSigner.Trim().ToLowerInvariant() -ne "true") {
    throw "Token ownership, Governance reserve/status, or signer role changed after review. No proposal was submitted."
}

$submitOutput = & $cast send $gov "submit(address,bytes,string)" $token $calldata $description `
    --private-key $submitter.Key --rpc-url $RpcUrl 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "Proposal submission failed; inspect chain state before retrying.`n$submitOutput" }
$txHashMatch = [regex]::Match($submitOutput, '(?im)^\s*(?:transactionHash:\s*)?(0x[a-fA-F0-9]{64})\s*$')
if (-not $txHashMatch.Success) {
    throw "Proposal command returned without a recognizable transaction hash. Inspect the output before retrying.`n$submitOutput"
}
Write-Host "Proposal transaction submitted: $($txHashMatch.Groups[1].Value)" -ForegroundColor Green
Write-Host "The Governance threshold is $threshold. Review the exact target, calldata, and description from this transaction before any signer votes."
