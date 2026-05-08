param(
    [switch]$SkipExplorer,
    [switch]$SkipFaucetRestart,
    [string]$RecipientAddress
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Get-DotEnvValue {
    param(
        [string]$Path,
        [string]$Key
    )

    $prefix = "$Key="
    foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
        if ($line.StartsWith($prefix)) {
            return $line.Substring($prefix.Length).Trim()
        }
    }

    return $null
}

function New-RandomEthereumAddress {
    while ($true) {
        $bytes = [byte[]]::new(20)
        [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
        $address = "0x" + (($bytes | ForEach-Object { $_.ToString("x2") }) -join "")
        if ($address -ne "0x0000000000000000000000000000000000000000") {
            return $address
        }
    }
}

function Wait-ForHttpEndpoint {
    param(
        [string]$Uri,
        [string]$Description,
        [int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $lastError = $null

    while ((Get-Date) -lt $deadline) {
        try {
            return Invoke-WebRequest -Uri $Uri
        } catch {
            $lastError = $_
            Start-Sleep -Seconds 1
        }
    }

    throw "$Description did not become reachable at $Uri. Last error: $lastError"
}

function Get-Erc20Balance {
    param(
        [string]$Address,
        [string]$TokenAddress,
        [string[]]$ComposeArgs
    )

    $output = docker compose @ComposeArgs exec -T -e FOUNDRY_DISABLE_NIGHTLY_WARNING=1 anvil cast call $TokenAddress "balanceOf(address)(uint256)" $Address

    if ($LASTEXITCODE -ne 0) {
        throw "failed to read ERC20 balance for $Address"
    }

    $line = ($output | Select-Object -Last 1).Trim()
    if ($line -match "^([0-9]+)") {
        return [System.Numerics.BigInteger]::Parse($matches[1])
    }

    throw "unexpected ERC20 balance output: $line"
}

function Solve-PowChallenge {
    param(
        [string]$Challenge,
        [int]$Difficulty
    )

    if (-not ("ChainRegistry.PowSolver" -as [type])) {
        Add-Type -TypeDefinition @"
using System;
using System.Security.Cryptography;
using System.Text;

namespace ChainRegistry {
    public static class PowSolver {
        public static long Solve(string challenge, int difficulty) {
            using var sha = SHA256.Create();
            var prefix = Encoding.UTF8.GetBytes(challenge);
            var buffer = new byte[prefix.Length + 32];
            Buffer.BlockCopy(prefix, 0, buffer, 0, prefix.Length);

            for (long nonce = 0; nonce < long.MaxValue; nonce++) {
                var nonceBytes = Encoding.UTF8.GetBytes(nonce.ToString());
                Buffer.BlockCopy(nonceBytes, 0, buffer, prefix.Length, nonceBytes.Length);
                var hash = sha.ComputeHash(buffer, 0, prefix.Length + nonceBytes.Length);
                if (HasLeadingZeroBits(hash, difficulty)) {
                    return nonce;
                }
            }

            throw new InvalidOperationException("Unable to solve faucet PoW challenge.");
        }

        private static bool HasLeadingZeroBits(byte[] hash, int difficulty) {
            var leadingZeros = 0;
            foreach (var b in hash) {
                if (b == 0) {
                    leadingZeros += 8;
                } else {
                    leadingZeros += CountLeadingZeroBits(b);
                    break;
                }

                if (leadingZeros >= difficulty) {
                    break;
                }
            }

            return leadingZeros >= difficulty;
        }

        private static int CountLeadingZeroBits(byte value) {
            var count = 0;
            for (var mask = 0x80; mask > 0; mask >>= 1) {
                if ((value & mask) == 0) {
                    count++;
                } else {
                    break;
                }
            }

            return count;
        }
    }
}
"@
    }

    return [ChainRegistry.PowSolver]::Solve($Challenge, $Difficulty)
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$envFile = Join-Path $repoRoot ".env.testnet"
$composeFile = Join-Path $repoRoot "docker-compose.testnet.yml"
$composeArgs = @("--project-directory", $repoRoot, "--env-file", $envFile, "-f", $composeFile)

$tokenAddress = Get-DotEnvValue -Path $envFile -Key "TESTNET_TOKEN_ADDR"
$dripAmountRaw = Get-DotEnvValue -Path $envFile -Key "FAUCET_DRIP_AMOUNT"
$faucetAddress = Get-DotEnvValue -Path $envFile -Key "FAUCET_ADDRESS"

if (
    [string]::IsNullOrWhiteSpace($tokenAddress) -or
    [string]::IsNullOrWhiteSpace($dripAmountRaw) -or
    [string]::IsNullOrWhiteSpace($faucetAddress)
) {
    throw "missing required faucet or token configuration in $envFile"
}

$dripAmount = [System.Numerics.BigInteger]::Parse($dripAmountRaw)

Set-Location $repoRoot

if (-not $SkipFaucetRestart) {
    Write-Host "Restarting faucet to clear in-memory rate limiter..." -ForegroundColor Cyan
    docker compose @composeArgs restart faucet | Out-Null

    if ($LASTEXITCODE -ne 0) {
        throw "failed to restart faucet before smoke test"
    }
}

Write-Host "Checking node health..." -ForegroundColor Cyan
$nodeHealthResponse = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8080/v1/health" -Description "Node health endpoint"
$nodeHealth = $nodeHealthResponse.Content | ConvertFrom-Json

Write-Host "Checking chain stats..." -ForegroundColor Cyan
$chainStatsResponse = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8080/v1/chain/stats" -Description "Chain stats endpoint"
$chainStats = $chainStatsResponse.Content | ConvertFrom-Json

Write-Host "Checking faucet health..." -ForegroundColor Cyan
$faucetHealthResponse = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8082/health" -Description "Faucet health endpoint"
$faucetHealth = $faucetHealthResponse.Content | ConvertFrom-Json

Write-Host "Checking IPFS service..." -ForegroundColor Cyan
$ipfsVersionOutput = docker compose @composeArgs exec -T ipfs ipfs version --number

if ($LASTEXITCODE -ne 0) {
    throw "failed to read IPFS version"
}

$ipfsIdOutput = docker compose @composeArgs exec -T ipfs ipfs id -f="<id>"

if ($LASTEXITCODE -ne 0) {
    throw "failed to read IPFS identity"
}

$ipfsVersion = ($ipfsVersionOutput | Select-Object -Last 1).Trim()
$ipfsIdentity = ($ipfsIdOutput | Select-Object -Last 1).Trim()

if (-not $SkipExplorer) {
    Write-Host "Checking explorer..." -ForegroundColor Cyan
    $explorerResponse = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:3007" -Description "Explorer endpoint"
}

$recipient = $RecipientAddress
if ([string]::IsNullOrWhiteSpace($recipient)) {
    $recipient = New-RandomEthereumAddress
}

Write-Host "Executing faucet drip to $recipient..." -ForegroundColor Cyan
$beforeBalance = Get-Erc20Balance -Address $recipient -TokenAddress $tokenAddress -ComposeArgs $composeArgs

# Solve PoW challenge before drip (N-02 fix)
Write-Host "  Fetching PoW challenge..." -ForegroundColor Gray
$challengeResponse = Invoke-RestMethod -Uri "http://127.0.0.1:8082/api/challenge" -Method Get
$challenge = $challengeResponse.challenge
$difficulty = $challengeResponse.difficulty

Write-Host "  Solving PoW (difficulty=$difficulty)..." -ForegroundColor Gray
$nonce = Solve-PowChallenge -Challenge $challenge -Difficulty $difficulty
Write-Host "  PoW solved (nonce=$nonce)" -ForegroundColor Gray

$dripBody = @{ address = $recipient; challenge = $challenge; nonce = "$nonce" } | ConvertTo-Json -Compress
$dripResponse = Invoke-RestMethod -Uri "http://127.0.0.1:8082/api/drip" -Method Post -ContentType "application/json" -Body $dripBody

if (-not $dripResponse.success) {
    throw "faucet drip failed: $($dripResponse.message)"
}

$afterBalance = Get-Erc20Balance -Address $recipient -TokenAddress $tokenAddress -ComposeArgs $composeArgs
$expectedBalance = $beforeBalance + $dripAmount

if ($afterBalance -ne $expectedBalance) {
    throw "unexpected recipient balance after drip. expected $expectedBalance, got $afterBalance"
}

$faucetBalance = Get-Erc20Balance -Address $faucetAddress -TokenAddress $tokenAddress -ComposeArgs $composeArgs

$summary = [ordered]@{
    node = [ordered]@{
        status = $nodeHealth.status
        version = $nodeHealth.version
        tip_height = $chainStats.tip_height
        block_count = $chainStats.block_count
        package_count = $chainStats.package_count
    }
    faucet = [ordered]@{
        status = $faucetHealth.status
        mode = $faucetHealth.mode
        faucet_balance = $faucetHealth.faucet_balance
        recipient = $recipient
        tx_hash = $dripResponse.tx_hash
        recipient_balance_before = $beforeBalance.ToString()
        recipient_balance_after = $afterBalance.ToString()
        faucet_balance_on_chain = $faucetBalance.ToString()
    }
    ipfs = [ordered]@{
        version = $ipfsVersion
        peer_id = $ipfsIdentity
    }
    explorer = if ($SkipExplorer) {
        [ordered]@{ skipped = $true }
    } else {
        [ordered]@{ status_code = $explorerResponse.StatusCode }
    }
}

Write-Host "Smoke test passed." -ForegroundColor Green
$summary | ConvertTo-Json -Depth 5