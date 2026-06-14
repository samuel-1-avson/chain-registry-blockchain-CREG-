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
$testnetDir = Split-Path -Parent $gcpDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

function Assert-LastExitCode([string]$step) {
    if ($LASTEXITCODE -ne 0) {
        throw "gcloud failed during: $step (exit $LASTEXITCODE)"
    }
}

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $GithubRepo) { $GithubRepo = $cfg.GITHUB_REPO }
if (-not $Branch) { $Branch = $cfg.GITHUB_BRANCH }

& (Join-Path $gcpDir "push-env.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName

$envFile = Join-Path $testnetDir "sepolia-3node.env"
$fleetMode = $false
$hybridMode = $false
if (Test-Path $envFile) {
    $fleetMode = Select-String -Path $envFile -Pattern '^\s*CREG_VALIDATOR_FLEET_MODE\s*=\s*true' -Quiet
    $hybridMode = Select-String -Path $envFile -Pattern '^\s*CREG_HYBRID_MODE\s*=\s*true' -Quiet
}
$caddyContainer = if ($fleetMode -or $hybridMode) { "creg-cloud-caddy" } else { "creg-3node-caddy" }
if ($fleetMode) {
    Write-Host "[gcp-deploy] CREG_VALIDATOR_FLEET_MODE=true - edge stack on $VmName (validators on fleet VM)" -ForegroundColor Cyan
} elseif ($hybridMode) {
    Write-Host "[gcp-deploy] CREG_HYBRID_MODE=true - edge-only stack (start validators locally with start-local-validators.ps1)" -ForegroundColor Cyan
} else {
    Write-Host "[gcp-deploy] Full 3-node stack on $VmName (default; no separate validator VM required)" -ForegroundColor Cyan
}

if ($Source -eq "local") {
    & (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName
    $remoteSh = Join-Path $gcpDir "start-remote-stack.sh"
} else {
    $remoteSh = Join-Path $gcpDir "deploy-remote.sh"
}

$remoteDest = "/tmp/creg-deploy-remote.sh"
Write-Host "[gcp-deploy] Uploading remote start script..."
gcloud compute scp $remoteSh "${VmName}:${remoteDest}" --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet
Assert-LastExitCode "upload remote start script"

$envExports = "export GITHUB_REPO='$GithubRepo'; export GITHUB_BRANCH='$Branch';"
Write-Host "[gcp-deploy] Starting stack on $VmName (first Docker build may take 15-30+ min)..."
gcloud compute ssh $VmName --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet --command="${envExports} bash $remoteDest"
Assert-LastExitCode "remote stack start"

Write-Host ""
Write-Host "[gcp-deploy] Done. Watch TLS:" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\ssh-vm.ps1 -Command 'sudo docker logs -f $caddyContainer'"
if ($hybridMode) {
    Write-Host "  Then on this PC (WireGuard up): .\testnet\start-local-validators.ps1" -ForegroundColor Cyan
}
