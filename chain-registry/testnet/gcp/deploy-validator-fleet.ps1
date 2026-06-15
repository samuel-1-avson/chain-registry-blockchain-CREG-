# Sync repo + env to creg-validator-vm and start validator fleet.
#
# Usage:
#   .\testnet\gcp\deploy-validator-fleet.ps1
#   .\testnet\gcp\deploy-validator-fleet.ps1 -SkipSync   # env + restart only (faster)
#   .\testnet\gcp\deploy-validator-fleet.ps1 -FleetBuild # compile on VM (CREG_FLEET_BUILD=1)

param(
    [switch]$SkipSync,
    [switch]$FleetBuild
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

$vmName = if ($cfg.GCP_VALIDATOR_VM_NAME) { $cfg.GCP_VALIDATOR_VM_NAME } else { "creg-validator-vm" }
$zone = $cfg.GCP_ZONE
$project = $cfg.GCP_PROJECT

function Log($m) { Write-Host "[deploy-validator-fleet] $m" }

$envFile = Join-Path $testnetDir "sepolia-3node.env"
if (-not (Test-Path $envFile)) {
    throw "Missing $envFile - copy from sepolia-3node.env.example"
}

Log "Pushing sepolia-3node.env to $vmName..."
& (Join-Path $gcpDir "push-env.ps1") -ProjectId $project -Zone $zone -VmName $vmName -EnvFile $envFile -TunnelThroughIap

if (-not $SkipSync) {
    Log "Syncing local chain-registry tree to $vmName (IAP)..."
    & (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $project -Zone $zone -VmName $vmName
} else {
    Log "Skipping full repo sync (-SkipSync); uploading start script only."
}

Log "Starting validator fleet on $vmName (GHCR pull by default; CREG_FLEET_BUILD=1 for local compile)..."
$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$sshOpts = @("--zone=$zone", "--project=$project", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$scpOpts = @("--zone=$zone", "--project=$project", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$remoteHome = (gcloud compute ssh $vmName @sshOpts --command="printf '%s' `$HOME").Trim()
$remoteStart = "$remoteHome/creg-hosting/$repoSlug/chain-registry/testnet/start-validator-fleet-gcp.sh"
if ($SkipSync) {
    $localStart = Join-Path $testnetDir "start-validator-fleet-gcp.sh"
    gcloud compute ssh $vmName @sshOpts --command="mkdir -p '$(Split-Path $remoteStart -Parent)'" | Out-Null
    gcloud compute scp $localStart "${vmName}:${remoteStart}" @scpOpts
}
if ($FleetBuild) {
    Log "FleetBuild: remote start sets CREG_FLEET_BUILD=1 (VM compile)"
}
# nohup: long Docker builds must survive IAP SSH disconnects.
$envPrefix = if ($FleetBuild) { "env CREG_FLEET_BUILD=1 " } else { "" }
$remoteCmd = "sed -i 's/\r$//' '$remoteStart' && chmod +x '$remoteStart' && nohup ${envPrefix}bash '$remoteStart' > /tmp/creg-fleet-start.log 2>&1 & sleep 2 && tail -n 30 /tmp/creg-fleet-start.log"
& (Join-Path $gcpDir "ssh-validator-vm.ps1") -Command $remoteCmd
if ($LASTEXITCODE -ne 0) { throw "remote start failed" }

Log "Done. Check nodes:"
Log "  .\testnet\gcp\ssh-validator-vm.ps1 -Command 'docker ps'"
