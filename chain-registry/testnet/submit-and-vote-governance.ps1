# Submit and Co-Sign Governance Proposal (2-of-3 Multisig)
#
# Enables decentralized execution under the 2-of-3 threshold:
# 1. Signer A submits the proposal and votes 'true' (1/2 approvals)
# 2. Signer B votes 'true' (2/2 approvals -> triggers automatic execution)
#
# Usage:
#   .\testnet\submit-and-vote-governance.ps1 `
#       -Target "0x30604Aa6f763fbde6aDCffbd8960af3031037331" `
#       -Calldata "0x..." `
#       -Description "Update Staking Parameter" `
#       -ProposerKey "0x..." `
#       -CoSignerKey "0x..."

param(
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$Calldata,
    [Parameter(Mandatory = $true)]
    [string]$Description,
    [string]$ProposerKey = "",
    [string]$CoSignerKey = "",
    [string]$RpcUrl = ""
)

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

if (-not $RpcUrl) {
    $RpcUrl = $env:SEPOLIA_RPC_URL
    if (-not $RpcUrl) { $RpcUrl = "https://ethereum-sepolia-rpc.publicnode.com" }
}
$GovAddr = $env:CREG_GOVERNANCE_ADDR
if (-not $GovAddr) { $GovAddr = "0x122Bc2C704B966C74004506F98F13bf7b5F75b9D" }

$cosignersPath = Join-Path $scriptDir "secrets\governance-cosigners.json"
if (-not (Test-Path $cosignersPath)) { throw "Missing $cosignersPath" }
$cosignersConfig = Get-Content $cosignersPath -Raw | ConvertFrom-Json

if (-not $ProposerKey) {
    $ProposerKey = $env:GOVERNANCE_SIGNER_KEY
    if (-not $ProposerKey) { $ProposerKey = $env:DEPLOYER_KEY }
}
if (-not $CoSignerKey) {
    # Default to Co-Signer Alpha
    $CoSignerKey = $cosignersConfig.signers[1].privateKey
}

$proposerAddr = (& $cast wallet address --private-key $ProposerKey 2>&1 | Out-String).Trim()
$cosignerAddr = (& $cast wallet address --private-key $CoSignerKey 2>&1 | Out-String).Trim()

Write-Host ""
Write-Host "=== Governance 2-of-3 Proposal Execution ===" -ForegroundColor Cyan
Write-Host "Governance Contract : $GovAddr"
Write-Host "Target Contract     : $Target"
Write-Host "Description         : $Description"
Write-Host "Proposer (Signer A) : $proposerAddr"
Write-Host "Co-Signer (Signer B): $cosignerAddr"
Write-Host ""

# Verify both are signers
$isA = (& $cast call $GovAddr "isSigner(address)(bool)" $proposerAddr --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
$isB = (& $cast call $GovAddr "isSigner(address)(bool)" $cosignerAddr --rpc-url $RpcUrl 2>&1 | Out-String).Trim()

if ($isA -notmatch "true") { throw "Proposer $proposerAddr is not an active governance signer!" }
if ($isB -notmatch "true") { throw "Co-Signer $cosignerAddr is not an active governance signer!" }

# 1. Get next proposal ID
$propCountRaw = (& $cast call $GovAddr "proposalCount()(uint256)" --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
$propId = [uint64]($propCountRaw -replace '\D', '')
Write-Host "Next Proposal ID: #$propId" -ForegroundColor Yellow

# 2. Submit proposal by Signer A
Write-Host "Step 1: Signer A submitting proposal #$propId..." -ForegroundColor Cyan
& $cast send $GovAddr "submit(address,bytes,string)" $Target $Calldata $Description `
    --private-key $ProposerKey --rpc-url $RpcUrl | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Proposal submission failed" }

# 3. Vote from Signer A (Approval: 1)
Write-Host "Step 2: Signer A voting 'true' on proposal #$propId..." -ForegroundColor Cyan
& $cast send $GovAddr "vote(uint256,bool)" $propId.ToString() true `
    --private-key $ProposerKey --rpc-url $RpcUrl | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Signer A vote failed" }
Write-Host "  -> Signer A approved (1/2 threshold reached)" -ForegroundColor DarkGray

Start-Sleep -Seconds 3

# 4. Vote from Signer B (Approval: 2 -> Auto-Execute)
Write-Host "Step 3: Signer B co-signing & voting 'true' on proposal #$propId..." -ForegroundColor Cyan
& $cast send $GovAddr "vote(uint256,bool)" $propId.ToString() true `
    --private-key $CoSignerKey --rpc-url $RpcUrl | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Signer B co-sign failed" }
Write-Host "  -> Signer B approved (2/2 threshold reached -> EXECUTED!)" -ForegroundColor Green

Start-Sleep -Seconds 3

# 5. Check proposal status
$propInfo = (& $cast call $GovAddr "getProposal(uint256)(address,string,uint256,uint8,uint256,uint256)" $propId.ToString() --rpc-url $RpcUrl 2>&1 | Out-String).Trim()
Write-Host ""
Write-Host "Proposal Execution Result:" -ForegroundColor Cyan
Write-Host $propInfo
Write-Host "[SUCCESS] Proposal #$propId successfully co-signed and executed under 2-of-3 threshold!" -ForegroundColor Green
