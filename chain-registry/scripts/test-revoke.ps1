# scripts/test-revoke.ps1
# End-to-end test: revoke the package published by test-publish.ps1.
# Must be run AFTER test-publish.ps1 (reads state.json from $env:TEMP\creg-test\)

$ErrorActionPreference = "Stop"

$NODE_API = "http://localhost:8080"

function Banner($msg) { Write-Host "" ; Write-Host "==> $msg" -ForegroundColor Cyan }
function OK($msg)     { Write-Host "    [OK] $msg" -ForegroundColor Green }
function FAIL($msg)   { Write-Host "    [FAIL] $msg" -ForegroundColor Red ; exit 1 }

# --- 0. Load state from publish run ------------------------------------------
Banner "Loading state from previous publish run"

$stateFile = Join-Path $env:TEMP "creg-test\state.json"

if (-not (Test-Path $stateFile)) {
    FAIL "No state.json found at $stateFile - run test-publish.ps1 first"
}

$state     = Get-Content $stateFile | ConvertFrom-Json
$CANONICAL = $state.canonical
OK "Found state: $CANONICAL"

# --- 1. Confirm current status is verified -----------------------------------
Banner "Checking current package status"

$encoded = [Uri]::EscapeDataString($CANONICAL)

try {
    $pkg = Invoke-RestMethod "$NODE_API/v1/packages/$encoded" -TimeoutSec 10
    Write-Host "  Status before revoke: $($pkg.status)" -ForegroundColor White
    if ($pkg.status -ne "verified") {
        Write-Host "  Warning: status is '$($pkg.status)' not 'verified' - proceeding anyway" -ForegroundColor Yellow
    } else {
        OK "Package is verified - ready to revoke"
    }
} catch {
    FAIL "Package not found on chain. Run test-publish.ps1 first and wait for confirmation."
}

# --- 2. Submit revocation ----------------------------------------------------
Banner "Submitting revocation for $CANONICAL"

$revokeBody = '{"reason":"End-to-end test: deliberate revocation"}'

try {
    $resp = Invoke-RestMethod `
        -Uri         "$NODE_API/v1/packages/$encoded/revoke" `
        -Method      Post `
        -ContentType "application/json" `
        -Body        $revokeBody `
        -TimeoutSec  15
    OK "Revocation accepted: $($resp | ConvertTo-Json -Compress)"
} catch {
    $statusCode = $_.Exception.Response.StatusCode.value__
    $detail     = $_.ErrorDetails.Message
    FAIL "Revocation rejected (HTTP $statusCode): $detail"
}

# --- 3. Poll until status changes to revoked ---------------------------------
Banner "Waiting for block confirmation of revocation"

$stats0 = Invoke-RestMethod "$NODE_API/v1/chain/stats"
$startH = $stats0.tip_height
Write-Host "    Starting block height: $startH" -ForegroundColor Gray

$confirmed = $false
for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 2
    $stats = Invoke-RestMethod "$NODE_API/v1/chain/stats"
    Write-Host "    Block: $($stats.tip_height)" -ForegroundColor Gray
    if ($stats.tip_height -gt $startH) {
        $confirmed = $true
        break
    }
}

if (-not $confirmed) { FAIL "Block not produced within 60s" }
OK "New block confirmed at height $($stats.block_height)"

# --- 4. Fetch updated package record -----------------------------------------
Banner "Fetching updated package record"

Start-Sleep -Seconds 1
$pkg = Invoke-RestMethod "$NODE_API/v1/packages/$encoded" -TimeoutSec 10

Write-Host ""
Write-Host "  Package           : $($pkg.canonical)"         -ForegroundColor White
$statusColor = if ($pkg.status -eq "revoked") { "Red" } else { "Yellow" }
Write-Host "  Status            : $($pkg.status)"            -ForegroundColor $statusColor
Write-Host "  Revocation reason : $($pkg.revocation_reason)" -ForegroundColor Gray
Write-Host ""

if ($pkg.status -ne "revoked") {
    FAIL "Expected status 'revoked' but got '$($pkg.status)'"
}
OK "Package status is now: revoked"

# --- 5. Check slash pool -----------------------------------------------------
Banner "Checking slash pool (validator redistribution)"

try {
    $slashInfo = Invoke-RestMethod "$NODE_API/v1/validators/slash-pool" -TimeoutSec 10
    Write-Host "  Slash pool balance     : $($slashInfo.balance) CREG"         -ForegroundColor White
    Write-Host "  Pending distribution   : $($slashInfo.pending_distribution)" -ForegroundColor White
    OK "Slash pool data retrieved"
} catch {
    Write-Host "    /v1/validators/slash-pool not yet implemented - check Explorer UI" -ForegroundColor Yellow
}

# --- 6. Summary --------------------------------------------------------------
Write-Host ""
Write-Host "  Explorer package page :"                                               -ForegroundColor Gray
Write-Host "  http://localhost:3000/packages/$([Uri]::EscapeDataString($CANONICAL))" -ForegroundColor Cyan
Write-Host ""
Write-Host "REVOKE TEST PASSED" -ForegroundColor Green
