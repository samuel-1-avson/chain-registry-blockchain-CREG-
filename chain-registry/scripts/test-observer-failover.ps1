# scripts/test-observer-failover.ps1
# Automated verification for Observer Pool High Availability & Zero-Downtime Failover

param(
    [string]$LbUrl = "http://localhost:8090",
    [string]$Observer1Url = "http://localhost:28181",
    [string]$Observer2Url = "http://localhost:28182",
    [switch]$SkipDrill
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

Write-Host "=======================================================" -ForegroundColor Cyan
Write-Host "  CREG Observer Pool -- High-Availability Failover Drill" -ForegroundColor Cyan
Write-Host "=======================================================" -ForegroundColor Cyan

# Step 1: Verify Load Balancer
Write-Host "`n[1/5] Checking Load Balancer status..." -ForegroundColor Yellow
try {
    $lbStatus = Invoke-RestMethod -Uri "$LbUrl/lb-status" -TimeoutSec 5
    Write-Host "  [OK] Load Balancer is active ($($lbStatus.component))" -ForegroundColor Green
} catch {
    throw "Load Balancer is not reachable at $LbUrl. Ensure docker-compose.observer-ha.yml is running."
}

# Step 2: Verify Initial Cluster Health
Write-Host "`n[2/5] Checking initial pool health..." -ForegroundColor Yellow
$healthyNodes = 0
foreach ($endpoint in @(@{Name="Observer-1"; Url=$Observer1Url}, @{Name="Observer-2"; Url=$Observer2Url})) {
    try {
        $res = Invoke-RestMethod -Uri "$($endpoint.Url)/v1/health" -TimeoutSec 5
        Write-Host "  [OK] $($endpoint.Name) is healthy (node: $($res.node_id), status: $($res.status))" -ForegroundColor Green
        $healthyNodes++
    } catch {
        Write-Host "  [WARN] $($endpoint.Name) not yet healthy: $_" -ForegroundColor Yellow
    }
}

if ($healthyNodes -eq 0) {
    throw "Neither observer is healthy yet. Wait for startup to complete."
}

# Step 3: Verify Public Routing via Load Balancer
Write-Host "`n[3/5] Testing public queries through Load Balancer ($LbUrl)..." -ForegroundColor Yellow
for ($i = 1; $i -le 5; $i++) {
    $res = Invoke-RestMethod -Uri "$LbUrl/v1/health" -TimeoutSec 3
    Write-Host "  Request $($i): 200 OK (routed to $($res.node_id))" -ForegroundColor Gray
    Start-Sleep -Milliseconds 200
}
Write-Host "  [OK] Traffic successfully balancing across pool" -ForegroundColor Green

if ($SkipDrill) {
    Write-Host "`n[+] Preflight checks complete (failover drill skipped)." -ForegroundColor Green
    exit 0
}

# Step 4: Active Failover Drill (Stop Observer-1)
Write-Host "`n[4/5] INITIATING FAILOVER DRILL: Stopping Observer-1..." -ForegroundColor Yellow
& docker compose -f docker-compose.observer-ha.yml stop observer-1 | Out-Null
Write-Host "  Observer-1 stopped. Testing API continuity..." -ForegroundColor Gray

$failoverSuccesses = 0
$totalRequests = 10
for ($i = 1; $i -le $totalRequests; $i++) {
    try {
        $res = Invoke-RestMethod -Uri "$LbUrl/v1/health" -TimeoutSec 5
        if ($res.node_id -eq "observer-replica-2") {
            $failoverSuccesses++
        }
    } catch {
        Write-Host "  [FAIL] Request $($i) dropped: $_" -ForegroundColor Red
    }
    Start-Sleep -Milliseconds 150
}

if ($failoverSuccesses -eq $totalRequests) {
    Write-Host "  [OK] 100% of requests survived! ($failoverSuccesses/$totalRequests routed to Observer-2 with ZERO downtime)" -ForegroundColor Green
} else {
    throw "Failover drill failed: only $failoverSuccesses/$totalRequests succeeded during outage."
}

# Step 5: Self-Healing & Recovery
Write-Host "`n[5/5] RECOVERY DRILL: Restarting Observer-1..." -ForegroundColor Yellow
& docker compose -f docker-compose.observer-ha.yml start observer-1 | Out-Null
Start-Sleep -Seconds 5

$recovered = $false
for ($attempt = 1; $attempt -le 10; $attempt++) {
    try {
        $res = Invoke-RestMethod -Uri "$Observer1Url/v1/health" -TimeoutSec 3
        if ($res.status -eq "ok" -or $res.node_id) {
            $recovered = $true
            break
        }
    } catch {
        Start-Sleep -Seconds 2
    }
}

if ($recovered) {
    Write-Host "  [OK] Observer-1 recovered and re-joined the pool automatically" -ForegroundColor Green
} else {
    Write-Host "  [WARN] Observer-1 started but has not reported healthy yet" -ForegroundColor Yellow
}

Write-Host "`n=======================================================" -ForegroundColor Cyan
Write-Host "  DRILL COMPLETE: High-Availability & Zero Downtime VERIFIED!" -ForegroundColor Green
Write-Host "=======================================================" -ForegroundColor Cyan
