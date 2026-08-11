<#
.SYNOPSIS
  Restart full testnet GCP hosting after waitlist-only mode.

.DESCRIPTION
  Starts the optional Geth RPC VM, scales observer MIG back to 1, and points to
  the GCP public-services stack. Validators are external and are never started
  by this script. Does not auto-run start-cloud-edge-gcp.sh (run on VM after sync).

.EXAMPLE
  .\resume-testnet-hosting.ps1
  .\resume-testnet-hosting.ps1 -Yes
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$EdgeVmName = "",
    [string]$GethVmName = "",
    [string]$ObserverMigName = "",
    [int]$ObserverReplicas = 1,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $EdgeVmName) { $EdgeVmName = $cfg.GCP_VM_NAME }
if (-not $GethVmName) { $GethVmName = $cfg.GCP_SEPOLIA_GETH_VM_NAME }
if (-not $ObserverMigName) { $ObserverMigName = if ($cfg.GCP_OBSERVER_MIG_NAME) { $cfg.GCP_OBSERVER_MIG_NAME } else { "creg-observer-pool" } }

function Log($m) { Write-Host "[resume-testnet] $m" }

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud not found"
}
if (-not $ProjectId) { throw "Set GCP_PROJECT in hosting.env" }

if (-not $Yes -and -not $WhatIfPreference) {
    Write-Host "Will start GCP public services and observer pool. Re-run with -Yes." -ForegroundColor Yellow
    Write-Host "  Start: $GethVmName (optional internal Sepolia RPC)"
    Write-Host "  MIG:   $ObserverMigName -> $ObserverReplicas"
    Write-Host "  Then:  sync repo + start-cloud-edge-gcp.sh on edge VM"
    exit 0
}

function Invoke-Gcloud([string[]]$GcloudArgs) {
    if ($WhatIfPreference) {
        Write-Host "[WhatIf] gcloud $($GcloudArgs -join ' ')" -ForegroundColor DarkGray
        return
    }
    & gcloud @GcloudArgs
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $($GcloudArgs -join ' ')" }
}

gcloud config set project $ProjectId | Out-Null

# Geth before observers (faucet/RPC dependency).
foreach ($vm in @($GethVmName)) {
    $status = (gcloud compute instances describe $vm --zone=$Zone --project=$ProjectId --format="value(status)" 2>$null)
    if (-not $status) {
        Log "VM $vm not found — skipping"
        continue
    }
    if ($status -eq "RUNNING") {
        Log "$vm already running"
        continue
    }
    if ($PSCmdlet.ShouldProcess($vm, "Start VM")) {
        Log "Starting $vm ..."
        Invoke-Gcloud @("compute", "instances", "start", $vm, "--zone=$Zone", "--project=$ProjectId", "--quiet")
    }
}

$migExists = gcloud compute instance-groups managed describe $ObserverMigName --zone=$Zone --project=$ProjectId --format="value(name)" 2>$null
if ($migExists) {
    if ($PSCmdlet.ShouldProcess($ObserverMigName, "Resize MIG to $ObserverReplicas")) {
        Log "Scaling $ObserverMigName to $ObserverReplicas ..."
        Invoke-Gcloud @("compute", "instance-groups", "managed", "resize", $ObserverMigName, "--size=$ObserverReplicas", "--zone=$Zone", "--project=$ProjectId", "--quiet")
        Write-Host "After observers boot: .\deploy-observer-pool.ps1 -Confirm" -ForegroundColor Cyan
    }
} else {
    Log "MIG $ObserverMigName not found - skip observer scale-up"
}

$hubService = "creg-hub-api"
$hubRegion = "us-central1"
$hubExists = gcloud run services describe $hubService --region=$hubRegion --project=$ProjectId --format="value(metadata.name)" 2>$null
if ($hubExists) {
    if ($PSCmdlet.ShouldProcess($hubService, "Restore Cloud Run public ingress")) {
        Log "Restoring $hubService ingress=all ..."
        Invoke-Gcloud @("run", "services", "update", $hubService, "--region=$hubRegion", "--project=$ProjectId", "--ingress=all", "--quiet")
    }
}

Write-Host ""
Log "Next steps (manual on edge VM):"
Write-Host "  1. .\sync-local-repo.ps1"
Write-Host "  2. .\push-env.ps1"
Write-Host "  3. Configure CREG_VALIDATOR_P2P_SEEDS and CREG_VALIDATOR_HTTP_PEERS from independent operators"
Write-Host "  4. SSH: ./testnet/start-observer-pool-gcp.sh (on observer MIG instances)"
Write-Host "  5. SSH edge: ./testnet/start-cloud-edge-gcp.sh"
Write-Host ""
Write-Host "See docs/WAITLIST-ONLY-HOSTING.md — Relaunch checklist"
