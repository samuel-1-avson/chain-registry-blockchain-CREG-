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

function Assert-TlsCerts {
    param(
        [string]$RepoRoot
    )

    $certsDir = Join-Path $RepoRoot "testnet\certs"
    $serverCrt = Join-Path $certsDir "server.crt"
    $serverKey = Join-Path $certsDir "server.key"

    if ((Test-Path $serverCrt) -and (Test-Path $serverKey)) {
        Write-Host "TLS certificates found: $certsDir" -ForegroundColor Green
        return
    }

    Write-Host "" -ForegroundColor Yellow
    Write-Host "WARNING: TLS certificates not found in testnet/certs/" -ForegroundColor Yellow
    Write-Host "  The node API will be served over plain HTTP." -ForegroundColor Yellow
    Write-Host "  For a public testnet, generate certs first:" -ForegroundColor Yellow
    Write-Host "    bash scripts/generate-tls-certs.sh" -ForegroundColor Yellow
    Write-Host "" -ForegroundColor Yellow

    # On Windows we can use PowerShell/certreq to generate a self-signed cert
    # as a fallback when openssl is not available.
    if (Get-Command openssl -ErrorAction SilentlyContinue) {
        Write-Host "openssl found — generating self-signed TLS certs automatically..." -ForegroundColor Cyan
        $genScript = Join-Path $RepoRoot "scripts\generate-tls-certs.sh"
        if (Test-Path $genScript) {
            bash $genScript $certsDir
            if ((Test-Path $serverCrt) -and (Test-Path $serverKey)) {
                Write-Host "TLS certificates generated successfully." -ForegroundColor Green
            } else {
                Write-Host "openssl ran but certs were not created. Continuing without TLS." -ForegroundColor Yellow
            }
        }
    } else {
        Write-Host "  openssl not found. Install Git Bash or WSL to auto-generate certs." -ForegroundColor Yellow
        Write-Host "  Continuing without TLS (HTTP only)." -ForegroundColor Yellow
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
Assert-TlsCerts -RepoRoot $repoRoot
Assert-RequiredEnvValues -Path $envFile -Keys @(
    "NODE1_VALIDATOR_KEY",
    "VALIDATOR_SET_JSON",
    "DEPLOYER_KEY",
    "CREG_BRIDGE_KEY",
    "FAUCET_PRIVATE_KEY",
    "FAUCET_ADDRESS",
    "FAUCET_INITIAL_BALANCE",
    "RELAYER_PRIVATE_KEY"
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