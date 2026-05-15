param(
    [switch]$SkipExplorer,
    [switch]$SkipCleanup,
    [switch]$RunSmokeTests,
    [switch]$SkipPublish
)

$ErrorActionPreference = "Stop"

function Set-Or-AddEnvValue {
    param(
        [System.Collections.Generic.List[string]]$Lines,
        [string]$Key,
        [string]$Value
    )

    $prefix = "$Key="
    $index = -1
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i].StartsWith($prefix)) {
            $index = $i
            break
        }
    }

    $nextLine = "$Key=$Value"
    if ($index -ge 0) {
        $Lines[$index] = $nextLine
    } else {
        $Lines.Add($nextLine) | Out-Null
    }
}

function Get-DotEnvValue {
    param(
        [string]$Path,
        [string]$Key
    )

    if (-not (Test-Path $Path)) {
        return $null
    }

    $prefix = "$Key="
    foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
        if ($line.StartsWith($prefix)) {
            return $line.Substring($prefix.Length).Trim()
        }
    }

    return $null
}

function Ensure-LocalTestnetEnvFile {
    param([string]$EnvFile)

    $lines = [System.Collections.Generic.List[string]]::new()
    if (Test-Path $EnvFile) {
        foreach ($line in [System.IO.File]::ReadAllLines((Resolve-Path $EnvFile))) {
            $lines.Add($line) | Out-Null
        }
    }

    $defaults = [ordered]@{
        DEPLOYER_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        CREG_BRIDGE_KEY = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
        CREG_DEV_SANDBOX = "true"
        CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM = "true"
        FAUCET_PRIVATE_KEY = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
        FAUCET_ADDRESS = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        RELAYER_PRIVATE_KEY = "0x2a871d0798f97d79848a013d4936a73bf4cc922c325d5dc0003f279c7aa26f8f"
        NODE1_VALIDATOR_KEY = "848019e3bdb66143723dd68e60e62c03b8d42e010d70638691eb2631b3637691"
        NODE2_VALIDATOR_KEY = "5e6f4d3c2b1a09876543210fedcba9876543210fedcba9876543210fedcba987"
        NODE3_VALIDATOR_KEY = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100"
        LOCAL_GENESIS_VALIDATOR_EVM_ADDRESS = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
        LOCAL_TESTNET_VALIDATOR2_EVM_ADDRESS = "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
        LOCAL_TESTNET_VALIDATOR3_EVM_ADDRESS = "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"
        POSTGRES_USER = "creg"
        POSTGRES_PASSWORD = "creg"
        POSTGRES_DB = "chain_registry"
    }

    foreach ($entry in $defaults.GetEnumerator()) {
        if ([string]::IsNullOrWhiteSpace((Get-DotEnvValue -Path $EnvFile -Key $entry.Key))) {
            Set-Or-AddEnvValue -Lines $lines -Key $entry.Key -Value $entry.Value
        }
    }

    Set-Or-AddEnvValue -Lines $lines -Key "CREG_DEV_SANDBOX" -Value "true"
    Set-Or-AddEnvValue -Lines $lines -Key "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM" -Value "true"

    [System.IO.File]::WriteAllLines($EnvFile, $lines)
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
        throw "Missing required entries in ${Path}: $($missing -join ', ')"
    }
}

function Clear-ComposeEnvOverrides {
    param([string[]]$Keys)

    $cleared = @()
    foreach ($key in $Keys) {
        $envPath = "Env:$key"
        if (Test-Path $envPath) {
            Remove-Item $envPath -ErrorAction SilentlyContinue
            $cleared += $key
        }
    }

    if ($cleared.Count -gt 0) {
        Write-Host "Cleared host environment overrides that would shadow .env.local-testnet: $($cleared -join ', ')" -ForegroundColor Yellow
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$envFile = Join-Path $repoRoot ".env.local-testnet"
$composeFile = Join-Path $repoRoot "docker-compose.local-testnet.yml"
$composeArgs = @("--project-directory", $repoRoot, "--env-file", $envFile, "-f", $composeFile)
$bootstrapServices = @("ipfs", "postgres", "anvil")
$buildServices = @("app-image")
$runtimeServices = @("node-1", "node-2", "node-3", "observer", "indexer", "faucet", "relayer")

if (-not $SkipExplorer) {
    $buildServices += "web-explorer-image"
    $runtimeServices += "web-explorer"
}

Set-Location $repoRoot

Ensure-LocalTestnetEnvFile -EnvFile $envFile
Clear-ComposeEnvOverrides -Keys @(
    "LOCAL_TESTNET_GOVERNANCE_ADDR",
    "LOCAL_TESTNET_STAKING_ADDR",
    "LOCAL_TESTNET_REPUTATION_ADDR",
    "LOCAL_TESTNET_VRF_ADDR",
    "LOCAL_TESTNET_REGISTRY_ADDR",
    "LOCAL_TESTNET_APPEAL_ADDR",
    "LOCAL_TESTNET_TOKEN_ADDR",
    "LOCAL_TESTNET_ZK_VERIFIER_ADDR",
    "LOCAL_TESTNET_VALIDATOR_REWARDS_ADDR",
    "LOCAL_TESTNET_VALIDATOR_REWARDS_TREASURY",
    "LOCAL_GENESIS_VALIDATOR_EVM_ADDRESS",
    "LOCAL_TESTNET_VALIDATOR2_EVM_ADDRESS",
    "LOCAL_TESTNET_VALIDATOR3_EVM_ADDRESS",
    "NODE1_VALIDATOR_KEY",
    "NODE2_VALIDATOR_KEY",
    "NODE3_VALIDATOR_KEY",
    "DEPLOYER_KEY",
    "CREG_DEV_SANDBOX",
    "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM",
    "CREG_BRIDGE_KEY",
    "FAUCET_PRIVATE_KEY",
    "FAUCET_ADDRESS",
    "RELAYER_PRIVATE_KEY"
)

if (-not $SkipCleanup) {
    Write-Host "Stopping existing local testnet services..." -ForegroundColor Cyan
    docker compose @composeArgs down --remove-orphans
}

Write-Host "Starting shared local testnet services..." -ForegroundColor Cyan
docker compose @composeArgs up -d @bootstrapServices
if ($LASTEXITCODE -ne 0) {
    throw "bootstrap service startup failed"
}

Write-Host "Deploying local contracts..." -ForegroundColor Cyan
docker compose @composeArgs up deploy-contracts
if ($LASTEXITCODE -ne 0) {
    throw "deploy-contracts failed; not starting runtime services"
}

& (Join-Path $PSScriptRoot "sync-local-testnet-artifacts.ps1") -EnvPath $envFile

Assert-RequiredEnvValues -Path $envFile -Keys @(
    "LOCAL_TESTNET_GOVERNANCE_ADDR",
    "LOCAL_TESTNET_STAKING_ADDR",
    "LOCAL_TESTNET_REPUTATION_ADDR",
    "LOCAL_TESTNET_VRF_ADDR",
    "LOCAL_TESTNET_REGISTRY_ADDR",
    "LOCAL_TESTNET_TOKEN_ADDR"
)

Write-Host "Building shared runtime images..." -ForegroundColor Cyan
docker compose @composeArgs build @buildServices
if ($LASTEXITCODE -ne 0) {
    throw "runtime image build failed; not starting distributed services"
}

Write-Host "Starting distributed local runtime services..." -ForegroundColor Cyan
docker compose @composeArgs up -d --no-build @runtimeServices
if ($LASTEXITCODE -ne 0) {
    throw "runtime service startup failed"
}

Write-Host "" 
Write-Host "Current service state:" -ForegroundColor Cyan
docker compose @composeArgs ps

Write-Host ""
Write-Host "Endpoints:" -ForegroundColor Green
Write-Host "  Node 1 API:    http://localhost:8080"
Write-Host "  Node 2 API:    http://localhost:8085"
Write-Host "  Node 3 API:    http://localhost:8086"
Write-Host "  Observer API:  http://localhost:8087"
Write-Host "  Relayer:       http://localhost:8083"
Write-Host "  Indexer:       http://localhost:8084"
Write-Host "  Faucet:        http://localhost:8082"
Write-Host "  Explorer:      http://localhost:3007"
Write-Host "  IPFS API:      http://localhost:5001"
Write-Host "  IPFS Gateway:  http://localhost:8081"

if ($RunSmokeTests) {
    Write-Host ""
    Write-Host "Running distributed local smoke tests..." -ForegroundColor Cyan
    & (Join-Path $PSScriptRoot "smoke-test-local-testnet.ps1") -EnvFile $envFile -SkipExplorer:$SkipExplorer -SkipPublish:$SkipPublish
}