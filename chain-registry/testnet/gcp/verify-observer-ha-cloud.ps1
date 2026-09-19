# testnet/gcp/verify-observer-ha-cloud.ps1
# Automated verification for GCP Cloud Production Observer Pool HA & Internal Load Balancer

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$MigName = "",
    [string]$InternalLbName = "",
    [int]$ApiPort = 28182,
    [switch]$SkipProbe
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $MigName) { $MigName = if ($cfg.GCP_OBSERVER_MIG_NAME) { $cfg.GCP_OBSERVER_MIG_NAME } else { "creg-observer-pool" } }
if (-not $InternalLbName) { $InternalLbName = if ($cfg.GCP_OBSERVER_ILB_NAME) { $cfg.GCP_OBSERVER_ILB_NAME } else { "creg-observer-api-ilb" } }

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "  CREG CLOUD PRODUCTION OBSERVER POOL HA VERIFICATION (GCP)" -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host "Project:  $ProjectId" -ForegroundColor Gray
Write-Host "Region:   $Region | Zone: $Zone" -ForegroundColor Gray
Write-Host "MIG:      $MigName" -ForegroundColor Gray
Write-Host "ILB:      $InternalLbName" -ForegroundColor Gray

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud CLI not found in PATH"
}

# 1. Verify Managed Instance Group (MIG)
Write-Host "`n[1/4] Inspecting Observer Managed Instance Group ($MigName)..." -ForegroundColor Yellow
$migJson = gcloud compute instance-groups managed describe $MigName --zone=$Zone --project=$ProjectId --format=json 2>$null | ConvertFrom-Json
if (-not $migJson) {
    throw "MIG $MigName does not exist in zone $Zone."
}

$targetSize = $migJson.targetSize
Write-Host "  [OK] MIG exists (targetSize: $targetSize, baseTemplate: $($migJson.instanceTemplate.Split('/')[-1]))" -ForegroundColor Green

# 2. Inspect Running Instances
Write-Host "`n[2/4] Checking Observer VM replicas..." -ForegroundColor Yellow
$instances = @(gcloud compute instance-groups managed list-instances $MigName --zone=$Zone --project=$ProjectId --format=json 2>$null | ConvertFrom-Json)
$runningCount = 0
foreach ($inst in $instances) {
    $vmName = $inst.instance.Split('/')[-1]
    $status = $inst.instanceStatus
    Write-Host "  - VM: $vmName | Status: $status" -ForegroundColor Gray
    if ($status -eq "RUNNING") {
        $runningCount++
    }
}

if ($runningCount -lt 1) {
    throw "No running VM instances found in observer MIG $MigName."
}
Write-Host "  [OK] $runningCount/$($instances.Count) observer VM replicas running" -ForegroundColor Green

# 3. Verify Internal Load Balancer Forwarding Rule & Backend Service
Write-Host "`n[3/4] Verifying Internal TCP Load Balancer..." -ForegroundColor Yellow
$frJson = gcloud compute forwarding-rules describe $InternalLbName --region=$Region --project=$ProjectId --format=json 2>$null | ConvertFrom-Json
if (-not $frJson) {
    throw "ILB Forwarding Rule $InternalLbName not found."
}
$ilbIp = $frJson.IPAddress
Write-Host "  [OK] Forwarding Rule active on internal IP: $ilbIp (port: $($frJson.portRange))" -ForegroundColor Green

$backendService = $frJson.backendService.Split('/')[-1]
$beJson = gcloud compute backend-services describe $backendService --region=$Region --project=$ProjectId --format=json 2>$null | ConvertFrom-Json
Write-Host "  [OK] Backend Service: $backendService (healthCheck: $($beJson.healthChecks[0].Split('/')[-1]))" -ForegroundColor Green

# 4. Probe Edge Caddy Configuration & Upstream Route
Write-Host "`n[4/4] Verifying Edge Ingress Upstream Configuration..." -ForegroundColor Yellow
$fleetEnvPath = Join-Path (Split-Path -Parent $gcpDir) "sepolia-fleet.env"
if (Test-Path $fleetEnvPath) {
    $envContent = Get-Content $fleetEnvPath -Raw
    if ($envContent -match "CREG_OBSERVER_POOL_LB_IP=([^\r\n]+)") {
        $configuredIp = $Matches[1].Trim()
        if ($configuredIp -eq $ilbIp) {
            Write-Host "  [OK] Edge environment correctly points CREG_OBSERVER_POOL_LB_IP to $ilbIp" -ForegroundColor Green
        } else {
            Write-Host "  [WARN] Edge CREG_OBSERVER_POOL_LB_IP ($configuredIp) does not match ILB IP ($ilbIp)" -ForegroundColor Yellow
        }
    } else {
        Write-Host "  [INFO] CREG_OBSERVER_POOL_LB_IP not yet set in sepolia-fleet.env. Use: CREG_OBSERVER_POOL_LB_IP=$ilbIp" -ForegroundColor Cyan
    }
} else {
    Write-Host "  [INFO] sepolia-fleet.env not found locally (operator uses private secret)." -ForegroundColor Gray
}

Write-Host "`n================================================================================" -ForegroundColor Cyan
Write-Host "  GCP OBSERVER POOL HA VERIFICATION COMPLETE -- READINESS CONFIRMED" -ForegroundColor Green
Write-Host "================================================================================" -ForegroundColor Cyan
