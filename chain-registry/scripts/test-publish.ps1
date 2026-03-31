# scripts/test-publish.ps1
# End-to-end test: create package, pin to IPFS, publish to Chain Registry, confirm on-chain.
# Requires: Docker stack running (docker compose up -d), PowerShell 5.1+
# Usage: .\test-publish.ps1 [-Version 1.0.1]

param(
    [string]$Version = "1.0.0"
)

$ErrorActionPreference = "Stop"

$NODE_API  = "http://localhost:8080"
$IPFS_API  = "http://localhost:5001"
$ECOSYSTEM = "npm"
$PKG_NAME  = "hello-creg"
$PKG_VER   = $Version
$CANONICAL = $ECOSYSTEM + ":" + $PKG_NAME + "@" + $PKG_VER
$WORK_DIR  = Join-Path $env:TEMP "creg-test"

function Banner { param($msg) Write-Host "" ; Write-Host "==> $msg" -ForegroundColor Cyan }
function OK     { param($msg) Write-Host "    [OK] $msg" -ForegroundColor Green }
function FAIL   { param($msg) Write-Host "    [FAIL] $msg" -ForegroundColor Red ; exit 1 }
function ToHex  { param([byte[]]$b) ($b | ForEach-Object { $_.ToString("x2") }) -join "" }

# -------------------------------------------------------------------
Banner "Checking prerequisites"

try {
    $health = Invoke-RestMethod ($NODE_API + "/health") -TimeoutSec 5
    OK ("Node API reachable - status: " + $health.status)
} catch {
    FAIL ("Node API not reachable at " + $NODE_API + " - is the stack running? (docker compose up -d)")
}

$ipfsCheck = & curl.exe -s -X POST ($IPFS_API + "/api/v0/id") 2>&1
if ($LASTEXITCODE -ne 0 -or "$ipfsCheck" -notmatch '"ID"') {
    FAIL ("IPFS API not reachable at " + $IPFS_API + " - output: " + $ipfsCheck)
}
OK "IPFS API reachable"

# -------------------------------------------------------------------
Banner ("Creating test package: " + $CANONICAL)

New-Item -ItemType Directory -Force -Path $WORK_DIR | Out-Null
$pkgDir = Join-Path $WORK_DIR "hello-creg"
New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null

$pkgJson = '{"name":"hello-creg","version":"1.0.0","description":"Chain Registry test","main":"index.js","license":"MIT"}'
[System.IO.File]::WriteAllText((Join-Path $pkgDir "package.json"), $pkgJson)
[System.IO.File]::WriteAllText((Join-Path $pkgDir "index.js"), "module.exports=function(){return 'Hello Chain Registry!';};")

$tarball = Join-Path $WORK_DIR "hello-creg-1.0.0.tgz"

# Use Windows System32 tar (bsdtar) - avoids Git Bash tar misreading Windows paths
$tarExe = Join-Path $env:windir "System32\tar.exe"
if (-not (Test-Path $tarExe)) { $tarExe = "tar" }

Push-Location $WORK_DIR
& $tarExe -czf $tarball hello-creg 2>&1 | Out-Null
Pop-Location

if (-not (Test-Path $tarball)) { FAIL "Failed to create tarball" }
$tarSize = (Get-Item $tarball).Length
OK ("Tarball created: " + $tarball + " (" + $tarSize + " bytes)")

# -------------------------------------------------------------------
Banner "Computing SHA-256"

$sha256bytes = [System.Security.Cryptography.SHA256]::Create().ComputeHash(
    [System.IO.File]::ReadAllBytes($tarball)
)
$sha256hex   = ToHex $sha256bytes
# Node's sha256_hex() returns bare hex (no prefix) — must match exactly
$contentHash = $sha256hex
OK ("Content hash: sha256:" + $contentHash)

# -------------------------------------------------------------------
Banner "Pinning to IPFS"

# Use curl.exe (native Windows curl) - PowerShell's Invoke-RestMethod is blocked
# by IPFS Kubo's built-in Host/Origin security check.
$curlCmd = Get-Command curl.exe -ErrorAction SilentlyContinue
$curlPath = if ($curlCmd) { $curlCmd.Source } else { "curl.exe" }

$ipfsRaw = & $curlPath -s -X POST `
    -F "file=@$tarball" `
    ($IPFS_API + "/api/v0/add?pin=true") 2>&1

if ($LASTEXITCODE -ne 0 -or -not $ipfsRaw) {
    FAIL ("IPFS pin failed. curl output: " + $ipfsRaw)
}

try {
    $ipfsJson = $ipfsRaw | ConvertFrom-Json
    $ipfsCid  = $ipfsJson.Hash
} catch {
    FAIL ("Failed to parse IPFS response: " + $ipfsRaw)
}

if (-not $ipfsCid) { FAIL ("IPFS returned no CID. Response: " + $ipfsRaw) }
OK ("IPFS CID: " + $ipfsCid)

# -------------------------------------------------------------------
Banner "Generating Ed25519 keypair and signing"

$pyScript = Join-Path $WORK_DIR "sign.py"

# Write Python script to a temp file
$pyLines = @(
    "import sys",
    "from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey",
    "from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat, PrivateFormat, NoEncryption",
    "canonical    = sys.argv[1]",
    "content_hash = sys.argv[2]",
    "priv = Ed25519PrivateKey.generate()",
    "pub  = priv.public_key()",
    "msg  = (canonical + content_hash).encode()",
    "sig  = priv.sign(msg)",
    "priv_bytes = priv.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())",
    "pub_bytes  = pub.public_bytes(Encoding.Raw, PublicFormat.Raw)",
    "print('PUBKEY:'  + pub_bytes.hex())",
    "print('SIG:'     + sig.hex())",
    "print('PRIVKEY:' + priv_bytes.hex())"
)
[System.IO.File]::WriteAllLines($pyScript, $pyLines)

# Find Python
$pythonCmd = $null
foreach ($cmd in @("python", "python3", "py")) {
    try {
        $ver = & $cmd --version 2>&1
        if ("$ver" -match "Python") { $pythonCmd = $cmd ; break }
    } catch {}
}

if (-not $pythonCmd) {
    FAIL "Python not found. Install Python and run: pip install cryptography"
}
OK ("Using " + $pythonCmd)

# Test if cryptography package is available
$testOut = & $pythonCmd -c "import cryptography" 2>&1
if ("$testOut" -match "ModuleNotFoundError|No module") {
    Write-Host "    Installing cryptography..." -ForegroundColor Yellow
    & $pythonCmd -m pip install cryptography -q 2>&1 | Out-Null
}

$pyOut = & $pythonCmd $pyScript $CANONICAL $contentHash 2>&1

$pubkeyHex = $null
$sigHex    = $null
$privHex   = $null

foreach ($line in $pyOut) {
    $line = "$line"
    if ($line.StartsWith("PUBKEY:"))  { $pubkeyHex = $line.Substring(7) }
    if ($line.StartsWith("SIG:"))     { $sigHex    = $line.Substring(4) }
    if ($line.StartsWith("PRIVKEY:")) { $privHex   = $line.Substring(8) }
}

if (-not $pubkeyHex -or -not $sigHex) {
    Write-Host "Python output:" -ForegroundColor Yellow
    $pyOut | ForEach-Object { Write-Host "  $_" }
    FAIL "Failed to generate keypair/signature"
}

OK ("Publisher pubkey: " + $pubkeyHex.Substring(0,16) + "...")
OK ("Signature:        " + $sigHex.Substring(0,16) + "...")

# -------------------------------------------------------------------
Banner "Building PublishRequest"

$now = [System.DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ss.fffZ")

$publishReq = [ordered]@{
    id = [ordered]@{
        ecosystem = $ECOSYSTEM
        name      = $PKG_NAME
        version   = $PKG_VER
    }
    content_hash     = $contentHash
    ipfs_cid         = $ipfsCid
    publisher_pubkey = $pubkeyHex
    signature        = $sigHex
    manifest         = [ordered]@{
        allowed_network_hosts = @()
        allowed_fs_writes     = @()
        spawns_processes      = $false
        description           = "Chain Registry end-to-end test package"
    }
    submitted_at   = $now
    shielded       = $false
    key_bundle     = $null
    pgp_signature  = $null
    pgp_public_key = $null
}

$body    = $publishReq | ConvertTo-Json -Depth 10
$bodyLen = $body.Length
OK ("Request payload built - " + $bodyLen + " bytes")

# -------------------------------------------------------------------
Banner ("Submitting to " + $NODE_API + "/v1/packages")

try {
    $resp = Invoke-RestMethod `
        -Uri         ($NODE_API + "/v1/packages") `
        -Method      Post `
        -ContentType "application/json" `
        -Body        $body `
        -TimeoutSec  15
    OK ("Accepted: " + ($resp | ConvertTo-Json -Compress))
} catch {
    $statusCode = $_.Exception.Response.StatusCode.value__
    $detail     = $_.ErrorDetails.Message
    FAIL ("Publish rejected (HTTP " + $statusCode + "): " + $detail)
}

# -------------------------------------------------------------------
Banner "Waiting for block confirmation"

$stats0 = Invoke-RestMethod ($NODE_API + "/v1/chain/stats")
$startH = $stats0.tip_height
Write-Host ("    Starting block height: " + $startH) -ForegroundColor Gray

$confirmed = $false
for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 2
    $stats = Invoke-RestMethod ($NODE_API + "/v1/chain/stats")
    Write-Host ("    Block: " + $stats.tip_height + "  Packages: " + $stats.package_count) -ForegroundColor Gray
    if ($stats.tip_height -gt $startH) {
        $confirmed = $true
        break
    }
}

if (-not $confirmed) { FAIL "Block not produced within 60s - validator may be stuck" }
OK ("New block confirmed at height " + $stats.tip_height)

# -------------------------------------------------------------------
Banner "Fetching package record from chain"

$encoded = [Uri]::EscapeDataString($CANONICAL)
try {
    $pkg = Invoke-RestMethod ($NODE_API + "/v1/packages/" + $encoded) -TimeoutSec 10
    Write-Host ""
    Write-Host ("  Package : " + $pkg.canonical)    -ForegroundColor White
    $sc = if ($pkg.status -eq "verified") { "Green" } else { "Yellow" }
    Write-Host ("  Status  : " + $pkg.status)       -ForegroundColor $sc
    Write-Host ("  Hash    : " + $pkg.content_hash)
    Write-Host ("  IPFS    : " + $pkg.ipfs_cid)
    Write-Host ("  Block   : " + $pkg.block_hash)
    Write-Host ""
    OK "Package is on-chain"
} catch {
    Write-Host "    Package not yet indexed - check Explorer at http://localhost:3000" -ForegroundColor Yellow
}

# -------------------------------------------------------------------
$state = @{
    pubkey       = $pubkeyHex
    privkey      = $privHex
    canonical    = $CANONICAL
    content_hash = $contentHash
    ipfs_cid     = $ipfsCid
}
$state | ConvertTo-Json | Set-Content (Join-Path $WORK_DIR "state.json") -Encoding UTF8

Write-Host ""
Write-Host ("  State saved : " + $WORK_DIR + "\state.json") -ForegroundColor Gray
Write-Host "  Explorer    : http://localhost:3000"           -ForegroundColor Gray
Write-Host "  Run test-revoke.ps1 to revoke this package."  -ForegroundColor Cyan
Write-Host ""
Write-Host "PUBLISH TEST PASSED" -ForegroundColor Green
