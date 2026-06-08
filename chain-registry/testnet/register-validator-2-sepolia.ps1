# On-chain validator application for node 2 of the 3-node Sepolia fleet.
#
# Node 2 uses CREG_NODE_ID=validator-2 and CREG_VALIDATOR_KEY_2 from sepolia-3node.env.
# L1 currently has only core-1 active; this script stakes/applies the second wallet so
# core-1's consensus-admission worker can approve via approveByConsensus (1-of-1 quorum).
#
# Prerequisites:
#   - testnet/sepolia-3node.env with CREG_ETH_RPC and CREG_VALIDATOR_KEY_2
#   - VALIDATOR_2_ETH_PRIVATE_KEY in the environment (or pass -EthPrivateKey)
#   - Wallet funded with Sepolia ETH and >= 100 tCREG on CREG_TOKEN_ADDR
#   - 3-node fleet running (node1 must observe admission and submit L1 tx)
#
# Usage:
#   $env:VALIDATOR_2_ETH_PRIVATE_KEY = "0x..."
#   .\testnet\register-validator-2-sepolia.ps1
#   .\testnet\register-validator-2-sepolia.ps1 -ApplyOnly
#   .\testnet\register-validator-2-sepolia.ps1 -CheckOnly

param(
    [string]$RpcUrl = "",
    [string]$EthPrivateKey = "",
    [ValidateSet("validator-2")]
    [string]$NodeId = "validator-2",
    [int]$StakeCreg = 100,
    [string]$Node2Api = "http://localhost:28181",
    [switch]$ApplyOnly,
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Import-DotEnv {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return }
    Get-Content $Path | ForEach-Object {
        if ($_ -match '^\s*([^#\s][^=]*)\s*=\s*(.*)\s*$') {
            [Environment]::SetEnvironmentVariable($matches[1].Trim(), $matches[2].Trim().Trim('"'), "Process")
        }
    }
}

$fleetEnv = Join-Path $scriptDir "sepolia-3node.env"
Import-DotEnv $fleetEnv

if (-not $RpcUrl) {
    $RpcUrl = $env:CREG_ETH_RPC
    if (-not $RpcUrl) { $RpcUrl = $env:SEPOLIA_RPC_URL }
}
if (-not $RpcUrl) {
    throw "Set CREG_ETH_RPC in testnet/sepolia-3node.env or pass -RpcUrl"
}

if (-not $EthPrivateKey) {
    $EthPrivateKey = $env:VALIDATOR_2_ETH_PRIVATE_KEY
}
if (-not $EthPrivateKey) {
    throw @"
Set VALIDATOR_2_ETH_PRIVATE_KEY (Sepolia EVM wallet for validator-2) or pass -EthPrivateKey.
Never commit this key. Fund the wallet with Sepolia ETH and >= $StakeCreg tCREG before applying.
"@
}

$specPath = Join-Path $scriptDir "chain-spec.sepolia.json"
$spec = Get-Content $specPath -Raw | ConvertFrom-Json
$staking = $spec.contracts.staking
$token = $spec.contracts.creg_token

$key2 = $env:CREG_VALIDATOR_KEY_2
if (-not $key2) { throw "CREG_VALIDATOR_KEY_2 missing from testnet/sepolia-3node.env" }
$key2 = $key2.Trim().TrimStart("0x")

$pub2 = (cargo run -q --example ed25519_pubkey_from_secret -p common -- $key2 2>&1 | Select-Object -Last 1).Trim()
if (-not $pub2) { throw "Failed to derive Ed25519 pubkey from CREG_VALIDATOR_KEY_2" }

$toolsCast = Join-Path $scriptDir ".tools\foundry\cast.exe"
$castCmd = Get-Command cast -ErrorAction SilentlyContinue
$cast = if (Test-Path $toolsCast) { $toolsCast } elseif ($castCmd) { $castCmd.Source } else { $null }
if (-not $cast) { throw "cast not found. Run .\testnet\install-foundry.ps1" }

$env:FOUNDRY_DISABLE_NIGHTLY_WARNING = "1"

$addrRaw = & $cast wallet address --private-key $EthPrivateKey 2>&1 | Out-String
if ($addrRaw -notmatch '(0x[a-fA-F0-9]{40})') { throw "Invalid EthPrivateKey" }
$validatorAddr = $matches[1]

$wei = [bigint]($StakeCreg * [decimal]1e18)

function Invoke-CastSend {
    param([string[]]$Args)
    & $cast @Args
    if ($LASTEXITCODE -ne 0) { throw "cast send failed: $($Args -join ' ')" }
}

function Get-ValidatorState {
    $raw = & $cast call $staking "validators(address)(uint256,uint8,uint256,uint256,uint256,uint256)" $validatorAddr --rpc-url $RpcUrl 2>&1 | Out-String
    if ($raw -match '(\d+)\s*\n\s*(\d+)') {
        return @{ stake = $matches[1]; state = [int]$matches[2] }
    }
    return $null
}

# ValidatorState: 0=None 1=Pending 2=Active 3=Unbonding 4=Rejected
$stateNames = @{ 0 = "None"; 1 = "Pending"; 2 = "Active"; 3 = "Unbonding"; 4 = "Rejected" }

Write-Host ""
Write-Host "=== Validator-2 Sepolia registration ===" -ForegroundColor Cyan
Write-Host "EVM address:  $validatorAddr"
Write-Host "Node ID:      $NodeId"
Write-Host "Ed25519 pub:  $pub2"
Write-Host "Staking:      $staking"
Write-Host "RPC:          $RpcUrl"
Write-Host ""

$vs = Get-ValidatorState
if ($vs) {
    $name = $stateNames[$vs.state]
    if (-not $name) { $name = "state=$($vs.state)" }
    Write-Host "On-chain:     $name (stake wei $($vs.stake))" -ForegroundColor $(if ($vs.state -eq 2) { "Green" } else { "Yellow" })
}

if ($CheckOnly) { exit 0 }

if ($vs -and $vs.state -eq 2) {
    Write-Host "Already Active on L1. Register identity on node2 if not done yet." -ForegroundColor Green
} elseif (-not $ApplyOnly -or -not $vs -or $vs.state -eq 0) {
    if (-not $vs -or $vs.state -eq 0 -or $vs.state -eq 4) {
        Write-Host "Approving $StakeCreg tCREG for staking contract..." -ForegroundColor Cyan
        Invoke-CastSend @(
            "send", $token, "approve(address,uint256)", $staking, $wei.ToString(),
            "--private-key", $EthPrivateKey, "--rpc-url", $RpcUrl
        )
        Write-Host "Submitting applyToBeValidator($StakeCreg tCREG)..." -ForegroundColor Cyan
        Invoke-CastSend @(
            "send", $staking, "applyToBeValidator(uint256)", $wei.ToString(),
            "--private-key", $EthPrivateKey, "--rpc-url", $RpcUrl
        )
        Write-Host "Application submitted (Pending)." -ForegroundColor Green
    } elseif ($vs.state -eq 1) {
        Write-Host "Application already Pending; waiting for core-1 admission quorum." -ForegroundColor Yellow
    }
}

Write-Host ""
Write-Host "=== Next steps (manual) ===" -ForegroundColor Cyan
Write-Host @"
1. Register validator identity on node 2 (must match keys above):
   POST $Node2Api/v1/validators/register
   Body: evm_address, node_id ($NodeId), ed25519_pubkey, nonce, evm_signature, ed25519_signature
   Easiest: Explorer wallet UI on the testnet explorer, or copy the flow from crates/node API tests.

2. Ensure node 1 (core-1) is running with CREG_STAKING_ADDR set — its consensus-admission
   worker signs EIP-712 attestations and submits approveByConsensus on L1.
   With one active validator, a single core-1 signature satisfies the 2/3 quorum rule.

3. Poll until Active:
   .\testnet\register-validator-2-sepolia.ps1 -CheckOnly

4. Optional: add validator-2 to chain-spec.sepolia.json, re-sign, restart spec-server
   (bootstrap metadata). Runtime can also merge from /v1/validators/register once L1 is Active.

5. If admission stalls > APPLICATION_TIMEOUT, re-run this script after expireApplication
   or fund more tCREG and re-apply.
"@
