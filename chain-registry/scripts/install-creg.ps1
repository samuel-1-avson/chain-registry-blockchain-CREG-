# ==============================================================================
# Chain Registry (CREG) -- Universal Remote Installer for Windows
# ==============================================================================
# Installs `creg.exe` (CLI) and `creg-node.exe` (Validator / Daemon).
#
# Quick remote install (PowerShell):
#   irm https://spec.testnet.cregnet.dev/install.ps1 | iex
#
# Options:
#   .\scripts\install-creg.ps1 -Version v0.4.8-testnet
#   .\scripts\install-creg.ps1 -BuildFromSource
#   .\scripts\install-creg.ps1 -InstallDir "C:\Custom\Path"
# ==============================================================================

param(
    [string]$Version = "v0.4.8-testnet",
    [switch]$BuildFromSource,
    [string]$InstallDir = "$env:USERPROFILE\.creg\bin",
    [string]$ConfigDir = "$env:USERPROFILE\.creg"
)

$ErrorActionPreference = "Stop"

function Invoke-CregDownload {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$OutFile,
        [switch]$GitHubRelease
    )

    if (-not $GitHubRelease -or -not $env:CREG_GITHUB_TOKEN) {
        Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing
        return
    }

    $assetName = [System.IO.Path]::GetFileName(([Uri]$Uri).AbsolutePath)
    $apiHeaders = @{
        Authorization = "Bearer $env:CREG_GITHUB_TOKEN"
        Accept        = "application/vnd.github+json"
        'User-Agent'  = "creg-installer"
    }
    $release = Invoke-RestMethod `
        -Uri "https://api.github.com/repos/$githubRepo/releases/tags/$Version" `
        -Headers $apiHeaders
    $releaseAsset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
    if (-not $releaseAsset) {
        throw "GitHub release $Version does not contain asset $assetName"
    }

    Invoke-WebRequest -Uri $releaseAsset.url -OutFile $OutFile -UseBasicParsing -Headers @{
        Authorization = "Bearer $env:CREG_GITHUB_TOKEN"
        Accept        = "application/octet-stream"
        'User-Agent'  = "creg-installer"
    }
}

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "   Chain Registry (CREG) -- Windows Remote Installer        " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# Determine if running from within a local repo
$repoRoot = $null
if ($MyInvocation.MyCommand -and $MyInvocation.MyCommand.Path) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $candidate = Split-Path -Parent $scriptDir
    if (Test-Path (Join-Path $candidate "Cargo.toml")) {
        $repoRoot = $candidate
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null

# ── Strategy 1: Build from source if requested ────────────────────────────────
if ($BuildFromSource) {
    if (-not $repoRoot -or -not (Test-Path (Join-Path $repoRoot "Cargo.toml"))) {
        throw "-BuildFromSource specified but not running inside the chain-registry repository."
    }
    Set-Location $repoRoot
    Write-Host "Building creg + creg-node from source..." -ForegroundColor Cyan
    if (-not $env:CREG_SPEC_SIGNING_PUBKEY) {
        $env:CREG_SPEC_SIGNING_PUBKEY = "3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01"
    }
    cargo build --locked --release --package chain-registry-cli --package chain-registry-node
    Copy-Item "target\release\creg.exe" (Join-Path $InstallDir "creg.exe") -Force
    if (Test-Path "target\release\creg-node.exe") {
        Copy-Item "target\release\creg-node.exe" (Join-Path $InstallDir "creg-node.exe") -Force
    }
    Write-Host "Installed to $InstallDir" -ForegroundColor Green
    exit 0
}

# ── Strategy 2: Precompiled binary download ───────────────────────────────────
$specHost = "https://spec.testnet.cregnet.dev"
$githubRepo = $env:CREG_GITHUB_REPO
if (-not $githubRepo) {
    $githubRepo = "samuel-1-avson/chain-registry-blockchain-CREG-"
}

$asset = "chain-registry-$Version-windows-amd64.zip"
$specUrl = "$specHost/dist/$asset"
$ghUrl = "https://github.com/$githubRepo/releases/download/$Version/$asset"

$zip = Join-Path $env:TEMP $asset
$checksums = Join-Path $env:TEMP "creg-$Version-SHA256SUMS"
$checksumUrl = $null

Write-Host "Fetching CREG $Version for Windows (x86_64)..." -ForegroundColor Cyan

$downloaded = $false
$downloadedFromGitHub = $false
try {
    Write-Host "Connecting to CREG Testnet CDN ($specUrl)..." -ForegroundColor DarkGray
    Invoke-CregDownload -Uri $specUrl -OutFile $zip
    $downloaded = $true
    $checksumUrl = "$specHost/dist/SHA256SUMS"
    Write-Host "[OK] Downloaded package from CREG Testnet CDN" -ForegroundColor Green
} catch {
    Write-Host "CDN fallback to GitHub Releases ($ghUrl)..." -ForegroundColor DarkGray
    try {
        Invoke-CregDownload -Uri $ghUrl -OutFile $zip -GitHubRelease
        $downloaded = $true
        $downloadedFromGitHub = $true
        $checksumUrl = "https://github.com/$githubRepo/releases/download/$Version/SHA256SUMS"
        Write-Host "[OK] Downloaded package from GitHub Releases" -ForegroundColor Green
    } catch {
        throw "Failed to download CREG Windows package from both CDN and GitHub Releases: $($_.Exception.Message)"
    }
}

# Checksums verification: require the manifest from the same source as the
# archive so a newer GitHub fallback cannot be checked against a stale CDN file.
try {
    Invoke-CregDownload -Uri $checksumUrl -OutFile $checksums -GitHubRelease:$downloadedFromGitHub
} catch {
    Remove-Item $zip -Force -ErrorAction SilentlyContinue
    throw "Failed to download SHA256SUMS for ${asset}: $($_.Exception.Message)"
}

$checksumPattern = "^\s*([0-9a-fA-F]{64})\s+\*?" + [regex]::Escape($asset) + "\s*$"
$checksumLine = (Get-Content $checksums -Raw -ErrorAction SilentlyContinue) -split "`r?`n" |
    Where-Object { $_ -match $checksumPattern } |
    Select-Object -First 1
if (-not $checksumLine) {
    Remove-Item $zip -Force -ErrorAction SilentlyContinue
    throw "Security Error: SHA256SUMS does not contain an entry for $asset"
}

$null = $checksumLine -match $checksumPattern
$expectedHash = $matches[1].ToLowerInvariant()
$actualHash = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash) {
    Remove-Item $zip -Force -ErrorAction SilentlyContinue
    throw "Security Error: Checksum mismatch for $asset (expected $expectedHash, got $actualHash)"
}
Write-Host "[OK] SHA-256 integrity verified: $actualHash" -ForegroundColor Green

Write-Host "Extracting binaries to $InstallDir ..." -ForegroundColor Cyan
Expand-Archive -Path $zip -DestinationPath $InstallDir -Force
Remove-Item $zip -Force -ErrorAction SilentlyContinue
if (Test-Path $checksums) {
    Remove-Item $checksums -Force -ErrorAction SilentlyContinue
}

# Fetch signed testnet chain spec
Write-Host "Syncing testnet chain specification..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri "$specHost/chain-spec.sepolia.json" -OutFile (Join-Path $ConfigDir "chain-spec.sepolia.json") -UseBasicParsing
    Invoke-WebRequest -Uri "$specHost/chain-spec.sepolia.json.sig" -OutFile (Join-Path $ConfigDir "chain-spec.sepolia.json.sig") -UseBasicParsing
} catch {
    Write-Host "Warning: Could not sync chain-spec directly from spec server: $($_.Exception.Message)" -ForegroundColor Yellow
}

# Configure default config.toml pointing to public decentralized testnet
$configFile = Join-Path $ConfigDir "config.toml"
if (-not (Test-Path $configFile)) {
    @'
[node]
url = "https://api.testnet.cregnet.dev"
timeout = 30

[defaults]
allow_unverified = false

[display]
colors = true
format = "Text"
progress = true

[ipfs]
url = "https://spec.testnet.cregnet.dev/ipfs"
timeout = 120
'@ | Out-File -FilePath $configFile -Encoding utf8
    Write-Host "[OK] Preconfigured public testnet endpoint (https://api.testnet.cregnet.dev)" -ForegroundColor Green
}

# Set persistent User Environment Variable CREG_NODE_URL
[Environment]::SetEnvironmentVariable("CREG_NODE_URL", "https://api.testnet.cregnet.dev", "User")
$env:CREG_NODE_URL = "https://api.testnet.cregnet.dev"

# Update Windows Environment Path
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notmatch [regex]::Escape($InstallDir)) {
    $newUserPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    Write-Host "[OK] Added $InstallDir to User Environment PATH" -ForegroundColor Green
}

# Update current session Path
if ($env:Path -notmatch [regex]::Escape($InstallDir)) {
    $env:Path = "$env:Path;$InstallDir"
}

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host "   Chain Registry (CREG) Installation Complete!             " -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host "Binary Location : $(Join-Path $InstallDir 'creg.exe')"
Write-Host "Chain Spec      : $(Join-Path $ConfigDir 'chain-spec.sepolia.json')"
Write-Host "Active Network  : https://api.testnet.cregnet.dev"
Write-Host ""

# Verify invocation
try {
    $ver = & (Join-Path $InstallDir 'creg.exe') --version
    Write-Host "Installed Version : $ver" -ForegroundColor Green
} catch {
    Write-Host "Note: Restart your terminal session to pick up the updated PATH environment." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Next steps:" -ForegroundColor Cyan
Write-Host "  creg --help                   View available commands"
Write-Host "  creg blocks                   View live blockchain blocks on testnet"
Write-Host "  creg keygen                   Create an Ed25519 identity keypair"
Write-Host "  creg init                     Interactive setup wizard"
Write-Host "============================================================" -ForegroundColor Green
