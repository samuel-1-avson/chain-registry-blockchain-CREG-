# CREG Transparency Log Witness Daemon (Windows PowerShell)
#
# Runs an independent transparency log witness.
# Continuously fetches signed tree heads, checks Merkle consistency proofs,
# and signs WitnessedTreeHead for client split-view protection.
#
# Usage:
#   .\testnet\start-witness-daemon.ps1
#   .\testnet\start-witness-daemon.ps1 -Once

param(
    [string]$NodeUrl = "https://api.testnet.cregnet.dev",
    [string]$LogId = "creg-transparency-log-1",
    [string]$LogPubkey = "3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01",
    [string]$WitnessId = "witness-alpha-independent",
    [string]$KeyPath = "",
    [switch]$Once
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

$userHome = [Environment]::GetFolderPath("UserProfile")
$dataDir = Join-Path $userHome ".creg\witness"
if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir -Force | Out-Null }

if (-not $KeyPath) { $KeyPath = Join-Path $dataDir "witness.key" }
$statePath = Join-Path $dataDir "witness-state.json"
$outPath   = Join-Path $dataDir "witnessed-head.json"

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " CREG Transparency Witness Daemon (Track B)" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "Witness ID       : $WitnessId"
Write-Host "Node URL         : $NodeUrl"
Write-Host "Log ID           : $LogId"
Write-Host "Log Public Key   : $LogPubkey"
Write-Host "Key Path         : $KeyPath"
Write-Host "State File       : $statePath"
Write-Host "Output Head      : $outPath"
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host ""

if (-not (Test-Path $KeyPath)) {
    Write-Host "[INFO] Witness key not found. Generating fresh Ed25519 witness key..." -ForegroundColor Yellow
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $rawBytes = New-Object byte[] 32
    $rng.GetBytes($rawBytes)
    $hexKey = -join ($rawBytes | ForEach-Object { $_.ToString("x2") })
    [System.IO.File]::WriteAllText($KeyPath, $hexKey)
    Write-Host "[OK] Generated witness key at $KeyPath" -ForegroundColor Green
}

$cliArgs = @(
    "run", "--quiet", "--package", "chain-registry-cli", "--bin", "creg", "--",
    "witness", "run",
    "--node-url", $NodeUrl,
    "--log-id", $LogId,
    "--log-pubkey", $LogPubkey,
    "--witness-id", $WitnessId,
    "--key-path", $KeyPath,
    "--state-path", $statePath,
    "--out-path", $outPath,
    "--interval-secs", "30"
)

if ($Once) {
    $cliArgs += "--once"
}

Write-Host "[INFO] Launching creg witness runner..." -ForegroundColor Cyan
& cargo @cliArgs
