param(
    [switch]$SkipExplorer,
    [switch]$SkipDeploySync,
    [switch]$SkipCleanup,
    [switch]$RunSmokeTests
)

$ErrorActionPreference = "Stop"

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

function Ensure-TestnetEnvFile {
    param(
        [string]$RepoRoot,
        [string]$EnvFile
    )

    if (Test-Path $EnvFile) {
        return
    }

    Write-Host ".env.testnet not found. Generating a bootstrap testnet environment..." -ForegroundColor Yellow
    $generator = Join-Path $RepoRoot "scripts\generate-testnet-keys.ps1"
    & $generator -Nodes 1 -Output $EnvFile

    if (-not (Test-Path $EnvFile)) {
        throw "failed to generate $EnvFile"
    }
}

function Assert-RequiredEnvValues {
    param(
        [string]$Path,
        [string[]]$Keys
    )

    $missing = @()
    foreach ($key in $Keys) {
        $value = Get-DotEnvValue -Path $Path -Key $key
        if ([string]::IsNullOrWhiteSpace($value)) {
            $missing += $key
        }
    }

    if ($missing.Count -gt 0) {
        throw "Missing required entries in ${Path}: $($missing -join ', '). Re-run scripts/generate-testnet-keys.ps1 or update the file manually."
    }
}

function Clear-ComposeEnvOverrides {
    param(
        [string[]]$Keys
    )

    $cleared = @()
    foreach ($key in $Keys) {
        $envPath = "Env:$key"
        if (Test-Path $envPath) {
            Remove-Item $envPath -ErrorAction SilentlyContinue
            $cleared += $key
        }
    }

    if ($cleared.Count -gt 0) {
        Write-Host "Cleared host environment overrides that would shadow .env.testnet: $($cleared -join ', ')" -ForegroundColor Yellow
    }
}

function Fund-TestnetFaucet {
    param(
        [string]$EnvFile,
        [string[]]$ComposeArgs
    )

    $tokenAddress = Get-DotEnvValue -Path $EnvFile -Key "TESTNET_TOKEN_ADDR"
    $faucetAddress = Get-DotEnvValue -Path $EnvFile -Key "FAUCET_ADDRESS"
    $deployerKey = Get-DotEnvValue -Path $EnvFile -Key "DEPLOYER_KEY"
    $targetBalanceRaw = Get-DotEnvValue -Path $EnvFile -Key "FAUCET_INITIAL_BALANCE"

    if ([string]::IsNullOrWhiteSpace($targetBalanceRaw)) {
        $targetBalanceRaw = Get-DotEnvValue -Path $EnvFile -Key "FAUCET_MAX_BALANCE"
    }

    if (
        [string]::IsNullOrWhiteSpace($tokenAddress) -or
        [string]::IsNullOrWhiteSpace($faucetAddress) -or
        [string]::IsNullOrWhiteSpace($deployerKey) -or
        [string]::IsNullOrWhiteSpace($targetBalanceRaw)
    ) {
        throw "missing faucet funding inputs in $EnvFile"
    }

    Write-Host "Funding faucet wallet..." -ForegroundColor Cyan
    $balanceOutput = docker compose @ComposeArgs run --rm --entrypoint sh deploy-contracts -c "cast call '$tokenAddress' 'balanceOf(address)(uint256)' '$faucetAddress' --rpc-url http://anvil:8545"

    if ($LASTEXITCODE -ne 0) {
        throw "failed to read faucet balance"
    }

    $currentBalanceRaw = ($balanceOutput | Select-Object -Last 1).Trim()
    $currentBalance = [System.Numerics.BigInteger]::Parse($currentBalanceRaw)
    $targetBalance = [System.Numerics.BigInteger]::Parse($targetBalanceRaw)

    if ($currentBalance -ge $targetBalance) {
        Write-Host "Faucet already funded to the configured balance." -ForegroundColor Green
        return
    }

    $topUpAmount = $targetBalance - $currentBalance
    docker compose @ComposeArgs run --rm --entrypoint sh deploy-contracts -c "cast send '$tokenAddress' 'transfer(address,uint256)' '$faucetAddress' '$topUpAmount' --private-key '$deployerKey' --rpc-url http://anvil:8545"

    if ($LASTEXITCODE -ne 0) {
        throw "failed to fund faucet wallet"
    }

    Write-Host "Faucet funded with $topUpAmount wei of tCREG." -ForegroundColor Green
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$envFile = Join-Path $repoRoot ".env.testnet"
$composeFile = Join-Path $repoRoot "docker-compose.testnet.yml"
$composeArgs = @("--project-directory", $repoRoot, "--env-file", $envFile, "-f", $composeFile)
$bootstrapServices = @(
    "ipfs",
    "postgres",
    "anvil"
)

$buildServices = @(
    "app-image"
)

$runtimeServices = @(
    "node-1",
    "faucet"
)

if (-not $SkipExplorer) {
    $buildServices += "web-explorer-image"
    $runtimeServices += "web-explorer"
}

Set-Location $repoRoot

Ensure-TestnetEnvFile -RepoRoot $repoRoot -EnvFile $envFile
Assert-RequiredEnvValues -Path $envFile -Keys @(
    "NODE1_VALIDATOR_KEY",
    "VALIDATOR_SET_JSON",
    "DEPLOYER_KEY",
    "CREG_BRIDGE_KEY",
    "FAUCET_ADDRESS",
    "FAUCET_INITIAL_BALANCE"
)
Clear-ComposeEnvOverrides -Keys @(
    "TESTNET_TOKEN_ADDR",
    "TESTNET_STAKING_ADDR",
    "TESTNET_REGISTRY_ADDR",
    "TESTNET_GOVERNANCE_ADDR",
    "TESTNET_ZK_VERIFIER_ADDR",
    "TESTNET_VALIDATOR_REWARDS_ADDR",
    "TESTNET_VALIDATOR_REWARDS_TREASURY",
    "FAUCET_TOKEN_CONTRACT"
)

if (-not $SkipCleanup) {
    Write-Host "Removing stale testnet containers..." -ForegroundColor Cyan
    $staleContainers = @(
        docker ps -a --filter "name=creg-testnet-" --format "{{.Names}}"
        docker ps -a --filter "name=chain-registry-deploy-contracts-run-" --format "{{.Names}}"
    ) | Where-Object { $_ -and $_.Trim().Length -gt 0 } | Select-Object -Unique
    foreach ($container in $staleContainers) {
        docker rm -f $container | Out-Null
    }
}

Write-Host "Starting Chain Registry testnet with .env.testnet" -ForegroundColor Cyan
docker compose @composeArgs up -d --build @bootstrapServices

Write-Host "Deploying contracts..." -ForegroundColor Cyan
docker compose @composeArgs up --build deploy-contracts

if ($LASTEXITCODE -ne 0) {
    throw "deploy-contracts failed; not starting runtime services"
}

if (-not $SkipDeploySync) {
    & (Join-Path $PSScriptRoot "sync-testnet-artifacts.ps1")
    Fund-TestnetFaucet -EnvFile $envFile -ComposeArgs $composeArgs
}

Assert-RequiredEnvValues -Path $envFile -Keys @(
    "TESTNET_TOKEN_ADDR",
    "TESTNET_STAKING_ADDR",
    "TESTNET_REGISTRY_ADDR",
    "TESTNET_GOVERNANCE_ADDR",
    "FAUCET_TOKEN_CONTRACT"
)

Write-Host "Building shared runtime images..." -ForegroundColor Cyan
docker compose @composeArgs build @buildServices

if ($LASTEXITCODE -ne 0) {
    throw "runtime image build failed; not starting runtime services"
}

Write-Host "Starting validator, faucet, and explorer services..." -ForegroundColor Cyan
docker compose @composeArgs up -d --no-build @runtimeServices

Write-Host ""
Write-Host "Current service state:" -ForegroundColor Cyan
docker compose @composeArgs ps

Write-Host ""
Write-Host "Endpoints:" -ForegroundColor Green
Write-Host "  Node API:      http://localhost:8080"
Write-Host "  Faucet:        http://localhost:8082"
Write-Host "  Explorer:      http://localhost:3000"
Write-Host "  IPFS API:      http://localhost:5001"
Write-Host "  IPFS Gateway:  http://localhost:8081"

if ($RunSmokeTests) {
    Write-Host "" 
    Write-Host "Running automated smoke tests..." -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot "smoke-test-testnet.ps1") -SkipExplorer:$SkipExplorer -SkipFaucetRestart
}