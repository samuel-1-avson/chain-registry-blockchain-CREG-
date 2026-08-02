# Clone/pull repo on the edge VM OR sync local tree, then start the GCP
# public-services stack. Validators run independently outside GCP.
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

$fleetEnv = Join-Path $testnetDir "sepolia-fleet.env"
$legacyEnv = Join-Path $testnetDir "sepolia-3node.env"
$envFile = if (Test-Path $fleetEnv) { $fleetEnv } else { $legacyEnv }
if (Test-Path -LiteralPath $envFile) {
    $legacyValidatorPlacement = Select-String -Path $envFile -Pattern '^\s*(CREG_VALIDATOR_FLEET_MODE|CREG_HYBRID_MODE)\s*=\s*true' -Quiet
    if ($legacyValidatorPlacement) {
        throw "Legacy validator placement is disabled. Remove CREG_VALIDATOR_FLEET_MODE/CREG_HYBRID_MODE and configure the external observer upstream."
    }
}

& (Join-Path $gcpDir "verify-public-services-profile.ps1") -EnvFile $envFile -Target edge
if ($LASTEXITCODE -ne 0) { throw "Public edge preflight failed; no edge resources were changed." }

& (Join-Path $gcpDir "push-env.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName -EnvFile $envFile

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
Write-Host "[gcp-deploy] Starting GCP public-services stack on $VmName (first Docker build may take 15-30+ min)..."
gcloud compute ssh $VmName --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet --command="${envExports} bash $remoteDest"
Assert-LastExitCode "remote stack start"

Write-Host ""
Write-Host "[gcp-deploy] Done. Watch TLS:" -ForegroundColor Cyan
Write-Host "  .\testnet\gcp\ssh-vm.ps1 -Command 'sudo docker logs -f creg-cloud-caddy'"
