# Clone/pull repo on VM OR sync local tree, then start the public 3-node stack.
#
# Usage:
#   .\testnet\gcp\deploy-stack.ps1              # default: local sync (HOSTING-301 scripts on disk)
#   .\testnet\gcp\deploy-stack.ps1 -Source github

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$GithubRepo = "",
    [string]$Branch = "",
    [ValidateSet("local", "github")]
    [string]$Source = "local"
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $GithubRepo) { $GithubRepo = $cfg.GITHUB_REPO }
if (-not $Branch) { $Branch = $cfg.GITHUB_BRANCH }

& (Join-Path $gcpDir "push-env.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName

if ($Source -eq "local") {
    & (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName
    $remoteSh = Join-Path $gcpDir "start-remote-stack.sh"
} else {
    $remoteSh = Join-Path $gcpDir "deploy-remote.sh"
}

$remoteDest = "/tmp/creg-deploy-remote.sh"
Write-Host "[gcp-deploy] Uploading remote start script..."
gcloud compute scp $remoteSh "${VmName}:${remoteDest}" --zone=$Zone --project=$ProjectId --strict-host-key-checking=no --quiet

$envExports = "export GITHUB_REPO='$GithubRepo'; export GITHUB_BRANCH='$Branch';"
Write-Host "[gcp-deploy] Starting stack on $VmName (first Docker build may take 15-30+ min)..."
gcloud compute ssh $VmName --zone=$Zone --project=$ProjectId --strict-host-key-checking=no --quiet --command="${envExports} bash $remoteDest"

Write-Host ""
Write-Host "[gcp-deploy] Done. Watch TLS:" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\ssh-vm.ps1 -Command 'docker logs -f creg-3node-caddy'"
