# Build creg-node:fleet on the validator VM and export /tmp/creg-node-fleet.tgz for observer transfer.
# Run from Windows — do not run build-export-node-image.sh locally unless you have Linux + Docker.
#
# Usage:
#   .\testnet\gcp\build-export-node-image.ps1 -Confirm
#   .\testnet\gcp\build-export-node-image.ps1 -Confirm -Tag v0.1.1-testnet
#   .\testnet\gcp\build-export-node-image.ps1 -Confirm -SyncLocal   # push this workstation's tree, then build
#   .\testnet\gcp\build-export-node-image.ps1 -Confirm -SkipCheckout   # build tree already on VM

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$ValidatorVm = "creg-validator-vm",
    [string]$Tag = "v0.1.2-testnet",
    [switch]$SyncLocal,
    [switch]$SkipCheckout,
    [switch]$Confirm
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]

function Log($m) { Write-Host "[build-export-node-image] $m" }

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--quiet")
$remoteRepo = "~/creg-hosting/$repoSlug/chain-registry"

if (-not $Confirm) {
    Write-Host "Will on $ValidatorVm :" -ForegroundColor Yellow
    if ($SyncLocal) { Write-Host "  sync-local-repo.ps1 (local tree -> VM, no .git on VM)" }
    elseif (-not $SkipCheckout) { Write-Host "  git fetch && git checkout $Tag (only if VM has git clone)" }
    Write-Host "  bash testnet/gcp/build-export-node-image.sh  (-> /tmp/creg-node-fleet.tgz)"
    Write-Host "Then run: .\testnet\gcp\transfer-observer-node-image.ps1 -Confirm" -ForegroundColor Cyan
    Write-Host "Re-run with -Confirm. On Windows use -SyncLocal after: git checkout $Tag" -ForegroundColor Cyan
    exit 0
}

if ($SyncLocal) {
    Log "Syncing local repo to $ValidatorVm ..."
    & (Join-Path $gcpDir "sync-local-repo.ps1") -VmName $ValidatorVm -ProjectId $ProjectId -Zone $Zone
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $SkipCheckout = $true
}

$checkout = ""
if (-not $SkipCheckout) {
    $checkout = @"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git fetch origin --tags &&
  git checkout '$Tag' &&
else
  echo 'No git repo on VM; use -SyncLocal from a machine with the desired checkout' >&2
  exit 1
fi &&
"@
}

$remoteCmd = @"
set -euo pipefail
cd $remoteRepo
$checkout
chmod +x testnet/gcp/build-export-node-image.sh
rm -f /tmp/observer-image-export.done
bash testnet/gcp/build-export-node-image.sh
test -f /tmp/observer-image-export.done && echo BUILD_EXPORT_OK
"@

Log "Building on $ValidatorVm (tag=$Tag, may take several minutes) ..."
gcloud compute ssh $ValidatorVm @sshOpts --command=$remoteCmd
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed. Logs: gcloud compute ssh $ValidatorVm @sshOpts --command='tail -30 /tmp/build-export.log'" -ForegroundColor Red
    exit $LASTEXITCODE
}

Log "Export ready on $ValidatorVm. Next: .\testnet\gcp\transfer-observer-node-image.ps1 -Confirm"
