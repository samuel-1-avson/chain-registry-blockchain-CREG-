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
    "anvil",
    "deploy-contracts"
)

$buildServices = @(
    "app-image"
)

$runtimeServices = @(
    "node-1",
    "node-2",
    "node-3",
    "node-4",
    "node-5",
    "node-6",
    "node-7",
    "node-8",
    "node-9",
    "node-10",
    "faucet"
)

if (-not $SkipExplorer) {
    $buildServices += "web-explorer-image"
    $runtimeServices += "web-explorer"
}

Set-Location $repoRoot

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

if (-not $SkipDeploySync) {
    Write-Host "Deploying contracts..." -ForegroundColor Cyan
    docker compose @composeArgs run --rm --entrypoint sh deploy-contracts -c "echo 'Waiting for Anvil...' && sleep 3 && mkdir -p contracts/deployments testnet/artifacts && forge build && echo 'Deploying contracts...' && forge script contracts/script/Deploy.s.sol:DeployChainRegistry --rpc-url http://anvil:8545 --broadcast -vvvv"

    if ($LASTEXITCODE -ne 0) {
        throw "deploy-contracts failed; not starting runtime services"
    }

    & (Join-Path $PSScriptRoot "sync-testnet-artifacts.ps1")
    Fund-TestnetFaucet -EnvFile $envFile -ComposeArgs $composeArgs
} else {
    docker compose @composeArgs run --rm --entrypoint sh deploy-contracts -c "echo 'Waiting for Anvil...' && sleep 3 && mkdir -p contracts/deployments testnet/artifacts && forge build && echo 'Deploying contracts...' && forge script contracts/script/Deploy.s.sol:DeployChainRegistry --rpc-url http://anvil:8545 --broadcast -vvvv"
}

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