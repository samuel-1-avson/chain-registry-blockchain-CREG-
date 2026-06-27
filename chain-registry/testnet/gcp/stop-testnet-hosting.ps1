<#
.SYNOPSIS
  Stop testnet GCP compute while keeping waitlist.cregnet.dev on the edge VM.

.DESCRIPTION
  Waitlist-only cost mode:
    - Stops creg-validator-vm and creg-sepolia-geth-vm (disks retained)
    - Scales observer MIG to 0 if present
    - Optionally trims edge Docker to waitlist + Caddy only

  Does NOT delete VMs, disks, or Firebase. Edge VM (creg-testnet-vm) stays RUNNING.

.EXAMPLE
  .\stop-testnet-hosting.ps1
  .\stop-testnet-hosting.ps1 -Yes
  .\stop-testnet-hosting.ps1 -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$EdgeVmName = "",
    [string]$ValidatorVmName = "",
    [string]$GethVmName = "",
    [string]$ObserverMigName = "",
    [switch]$SkipEdgeTrim,
    [switch]$SkipHubPause,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $EdgeVmName) { $EdgeVmName = $cfg.GCP_VM_NAME }
if (-not $ValidatorVmName) { $ValidatorVmName = $cfg.GCP_VALIDATOR_VM_NAME }
if (-not $GethVmName) { $GethVmName = $cfg.GCP_SEPOLIA_GETH_VM_NAME }
if (-not $ObserverMigName) { $ObserverMigName = if ($cfg.GCP_OBSERVER_MIG_NAME) { $cfg.GCP_OBSERVER_MIG_NAME } else { "creg-observer-pool" } }

function Log($m) { Write-Host "[stop-testnet] $m" }

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud not found. Install Google Cloud SDK and authenticate."
}
if (-not $ProjectId) { throw "Set GCP_PROJECT in hosting.env" }

# Estimated monthly savings (see docs/GCP-BUDGET-ARCHITECTURE.md; adjust if machine types differ).
$estValidator = 196
$estGeth = 49
$estObserver = 25
$estEdgeDocker = 15
$estTotal = $estValidator + $estGeth + $estObserver + $(if ($SkipEdgeTrim) { 0 } else { $estEdgeDocker })

Write-Host ""
Write-Host "Waitlist-only shutdown plan (project $ProjectId)" -ForegroundColor Cyan
Write-Host "  KEEP RUNNING: $EdgeVmName (waitlist + Caddy)"
Write-Host "  STOP VMs:     $ValidatorVmName, $GethVmName"
Write-Host "  SCALE MIG:    $ObserverMigName -> 0 (if exists)"
if (-not $SkipEdgeTrim) {
    Write-Host "  EDGE TRIM:    stop explorer/faucet/ipfs/spec/hub/monitoring; keep waitlist + caddy"
}
if (-not $SkipHubPause) {
    Write-Host "  HUB PAUSE:    set creg-hub-api Cloud Run ingress to internal"
    Write-Host "  API LB:       teardown-gcp-public-lb.ps1 if forwarding rule exists"
}
Write-Host ""
$edgeSavingsNote = if (-not $SkipEdgeTrim) { ", edge containers ~`$$estEdgeDocker" } else { "" }
Write-Host "Estimated savings: ~`$$estTotal/month (validator ~`$$estValidator, geth ~`$$estGeth, observer ~`$$estObserver$edgeSavingsNote)" -ForegroundColor Green
Write-Host "Edge VM compute (~`$98/mo e2-standard-4) + Firebase waitlist project still bill." -ForegroundColor Yellow
Write-Host ""

$doIt = $Yes
if (-not $doIt -and -not $WhatIfPreference) {
    $answer = Read-Host "Type YES to stop testnet VMs and trim hosting"
    if ($answer -ne "YES") {
        Write-Host "Aborted." -ForegroundColor Yellow
        exit 0
    }
    $doIt = $true
}

function Invoke-Gcloud([string[]]$GcloudArgs) {
    if ($WhatIfPreference) {
        Write-Host "[WhatIf] gcloud $($GcloudArgs -join ' ')" -ForegroundColor DarkGray
        return
    }
    & gcloud @GcloudArgs
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $($GcloudArgs -join ' ')" }
}

function Should-Run([string]$Target, [string]$Action) {
    if ($WhatIfPreference) { return $true }
    if ($doIt) { return $true }
    return $PSCmdlet.ShouldProcess($Target, $Action)
}

gcloud config set project $ProjectId | Out-Null

foreach ($vm in @($ValidatorVmName, $GethVmName)) {
    $status = (gcloud compute instances describe $vm --zone=$Zone --project=$ProjectId --format="value(status)" 2>$null)
    if (-not $status) {
        Log "VM $vm not found - skipping"
        continue
    }
    if ($status -eq "TERMINATED") {
        Log "$vm already stopped"
        continue
    }
    if (Should-Run $vm "Stop VM") {
        Log "Stopping $vm ..."
        Invoke-Gcloud @("compute", "instances", "stop", $vm, "--zone=$Zone", "--project=$ProjectId", "--quiet")
    }
}

$migExists = gcloud compute instance-groups managed describe $ObserverMigName --zone=$Zone --project=$ProjectId --format="value(name)" 2>$null
if ($migExists) {
    $target = gcloud compute instance-groups managed describe $ObserverMigName --zone=$Zone --project=$ProjectId --format="value(targetSize)" 2>$null
    if ($target -and [int]$target -gt 0) {
        if (Should-Run $ObserverMigName "Resize MIG to 0") {
            Log "Scaling $ObserverMigName from $target to 0 ..."
            Invoke-Gcloud @("compute", "instance-groups", "managed", "resize", $ObserverMigName, "--size=0", "--zone=$Zone", "--project=$ProjectId", "--quiet")
        }
    } else {
        Log "$ObserverMigName already at size 0"
    }
} else {
    Log "MIG $ObserverMigName not found - skipping"
}

if (-not $SkipEdgeTrim) {
    $trimScript = "set -euo pipefail; REPO=`$(ls -d ~/creg-hosting/*/chain-registry 2>/dev/null | head -1); if [ -z `"`$REPO`" ]; then echo 'Repo not found under ~/creg-hosting' >&2; exit 1; fi; cd `"`$REPO/testnet`"; chmod +x gcp/start-waitlist-edge-only.sh; ./gcp/start-waitlist-edge-only.sh `"`$REPO`""
    if (Should-Run $EdgeVmName "Run start-waitlist-edge-only.sh via SSH") {
        if (-not $WhatIfPreference) {
            Log "Trimming edge containers on $EdgeVmName ..."
            & (Join-Path $gcpDir "ssh-vm.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $EdgeVmName -Command $trimScript
        } else {
            Write-Host "[WhatIf] gcloud compute ssh $EdgeVmName ... start-waitlist-edge-only.sh" -ForegroundColor DarkGray
        }
    }
}

if (-not $SkipHubPause) {
    $hubService = "creg-hub-api"
    $hubRegion = "us-central1"
    $hubExists = gcloud run services describe $hubService --region=$hubRegion --project=$ProjectId --format="value(metadata.name)" 2>$null
    if ($hubExists) {
        if (Should-Run $hubService "Set Cloud Run ingress to internal") {
            Log "Pausing $hubService (ingress=internal) ..."
            Invoke-Gcloud @("run", "services", "update", $hubService, "--region=$hubRegion", "--project=$ProjectId", "--ingress=internal", "--quiet")
        }
    } else {
        Log "Cloud Run $hubService not found - skipping"
    }

    $lbFr = gcloud compute forwarding-rules describe creg-api-https-fr --global --project=$ProjectId --format="value(name)" 2>$null
    if ($lbFr) {
        if (Should-Run "creg-api-https-fr" "Teardown API load balancer") {
            Log "Tearing down API HTTPS load balancer ..."
            if (-not $WhatIfPreference) {
                & (Join-Path $gcpDir "teardown-gcp-public-lb.ps1") -ProjectId $ProjectId -Yes
            }
        }
    } else {
        Log "API HTTPS forwarding rule not found - LB already removed"
    }
}

Write-Host ""
Log "Done. Verify:"
Write-Host "  curl -fsS https://waitlist.cregnet.dev/"
Write-Host "  curl -fsS https://us-central1-gen-lang-client-0098858574.cloudfunctions.net/waitlistPublicStats"
Write-Host ""
Write-Host "Resume: .\resume-testnet-hosting.ps1 -Yes" -ForegroundColor Cyan
