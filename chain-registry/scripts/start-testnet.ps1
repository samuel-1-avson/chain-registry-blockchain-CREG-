param(
    [switch]$SkipExplorer,
    [switch]$SkipDeploySync,
    [switch]$SkipCleanup,
    [switch]$RunSmokeTests
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

$EnvFile = Join-Path $RepoRoot ".env.testnet"
$EnvExample = Join-Path $RepoRoot ".env.testnet.example"
$ComposeFile = Join-Path $RepoRoot "docker-compose.testnet.yml"

function Invoke-TestnetCompose {
    param([string[]]$Args)
    & docker compose --env-file $EnvFile -f $ComposeFile @Args
    if ($LASTEXITCODE -ne 0) {
        throw "docker compose failed (exit $LASTEXITCODE)"
    }
}

if (-not (Test-Path $EnvFile)) {
    if (-not (Test-Path $EnvExample)) {
        throw "Missing .env.testnet.example — cannot bootstrap Sepolia testnet env."
    }
    Copy-Item $EnvExample $EnvFile
    Write-Host "Created .env.testnet from .env.testnet.example — fill in <CHANGE_ME> keys before production use."
}

if (-not $SkipCleanup) {
    Write-Host "Stopping any previous bootstrap stack (volumes preserved)..."
    Invoke-TestnetCompose @("down", "--remove-orphans") 2>$null
}

Write-Host "Starting Sepolia bootstrap stack (no Anvil)..."
$upArgs = @("up", "-d", "--build")
if ($SkipExplorer) {
    $upArgs += @("--scale", "web-explorer=0")
}
Invoke-TestnetCompose $upArgs

Write-Host ""
Write-Host "Bootstrap host starting. Sepolia L1 — contracts from testnet/chain-spec.sepolia.json"
Write-Host "  Node API:      http://localhost:8080/v1/health"
Write-Host "  Chain spec:    http://localhost:8888/chain-spec.json"
Write-Host "  Faucet:        http://localhost:8082/health"
Write-Host "  Web Explorer:  http://localhost:3007"
Write-Host ""

if ($RunSmokeTests) {
    $deadline = (Get-Date).AddMinutes(3)
    $healthy = $false
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-RestMethod -Uri "http://localhost:8080/v1/health" -TimeoutSec 5
            if ($r.status -eq "ok" -or $r.healthy -eq $true -or $null -ne $r) {
                $healthy = $true
                break
            }
        } catch {
            Start-Sleep -Seconds 5
        }
    }
    if (-not $healthy) {
        throw "Node health check failed — see: docker compose --env-file .env.testnet -f docker-compose.testnet.yml logs node-1"
    }
    $stats = Invoke-RestMethod -Uri "http://localhost:8080/v1/chain/stats" -TimeoutSec 10
    Write-Host "Smoke OK — tip height: $($stats.tip_height), packages: $($stats.package_count)"
}