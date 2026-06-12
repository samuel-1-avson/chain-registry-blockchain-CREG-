# After build-export-node-image.sh finishes on creg-validator-vm, copy image to observer and restart.
#
# Usage:
#   .\testnet\gcp\transfer-observer-node-image.ps1 -Confirm

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$ValidatorVm = "creg-validator-vm",
    [string]$ObserverVm = "creg-observer-pool-j2q7",
    [switch]$Confirm
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]

function Log($m) { Write-Host "[transfer-observer-image] $m" }

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--quiet")
$done = gcloud compute ssh $ValidatorVm @sshOpts --command="test -f /tmp/observer-image-export.done && echo yes || echo no"
if ($done.Trim() -ne "yes") {
    throw "Validator export not ready. Check: gcloud compute ssh $ValidatorVm -- tail -5 /tmp/build-export.log"
}

if (-not $Confirm) {
    Write-Host "Will copy /tmp/creg-node-fleet.tgz from $ValidatorVm to $ObserverVm and restart observer." -ForegroundColor Yellow
    Write-Host "Re-run with -Confirm."
    exit 0
}

$localTar = Join-Path $env:TEMP "creg-node-fleet.tgz"
Log "Downloading tarball from $ValidatorVm ..."
gcloud compute scp "${ValidatorVm}:/tmp/creg-node-fleet.tgz" $localTar @sshOpts
Log "Uploading to $ObserverVm ..."
gcloud compute scp $localTar "${ObserverVm}:/tmp/creg-node-fleet.tgz" @sshOpts
Log "Importing and restarting observer ..."
gcloud compute ssh $ObserverVm @sshOpts --command="chmod +x ~/creg-hosting/$repoSlug/chain-registry/testnet/gcp/import-observer-node-image.sh && ~/creg-hosting/$repoSlug/chain-registry/testnet/gcp/import-observer-node-image.sh /tmp/creg-node-fleet.tgz"
Log "Done. Verify: curl.exe -fsS https://api.testnet.cregnet.dev/v1/health"
