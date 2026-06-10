# Copy sepolia-3node.env to GCP VM via gcloud compute scp.
#
# Usage:
#   .\testnet\gcp\push-env.ps1

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$EnvFile = ""
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $EnvFile) { $EnvFile = Join-Path $testnetDir "sepolia-3node.env" }

if (-not (Test-Path $EnvFile)) {
    throw "Missing $EnvFile - run prepare-public-hosting.ps1 first"
}

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteRel = "creg-hosting/$repoSlug/chain-registry/testnet"
$remoteFile = "$remoteRel/sepolia-3node.env"

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--strict-host-key-checking=no", "--quiet")

Write-Host "[gcp-push] Resolving remote home on $VmName ..."
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
if (-not $remoteHome) { throw "Could not resolve remote HOME on $VmName" }

$remoteAbsDir = "$remoteHome/$remoteRel"
$remoteAbsFile = "$remoteHome/$remoteFile"

Write-Host "[gcp-push] Uploading env to ${VmName}:$remoteAbsFile ..."
gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$remoteAbsDir' && chmod 700 '$remoteHome/creg-hosting'" | Out-Null
gcloud compute scp $EnvFile "${VmName}:${remoteAbsFile}" --zone=$Zone --project=$ProjectId --strict-host-key-checking=no --quiet
gcloud compute ssh $VmName @sshOpts --command="chmod 600 '$remoteAbsFile'" | Out-Null
Write-Host "[gcp-push] Done. Remote path: $remoteAbsFile"
