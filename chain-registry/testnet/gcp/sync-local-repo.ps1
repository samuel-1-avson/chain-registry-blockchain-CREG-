# Sync local chain-registry tree to GCP VM (excludes target/). Use when GitHub main lags local HOSTING-301 scripts.
#
# Usage:
#   .\testnet\gcp\sync-local-repo.ps1

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = ""
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteRel = "creg-hosting/$repoSlug/chain-registry"
$tarLocal = Join-Path $env:TEMP "creg-chain-registry-sync.tgz"

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
$remoteRoot = "$remoteHome/$remoteRel"

Write-Host "[gcp-sync] Packing local repo (excluding target/) ..."
Push-Location $repoRoot
if (Test-Path $tarLocal) { Remove-Item $tarLocal -Force }
& tar -czf $tarLocal --exclude=target --exclude=.git .
Pop-Location

Write-Host "[gcp-sync] Uploading to ${VmName}:$remoteRoot ..."
gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$remoteRoot'" | Out-Null
gcloud compute scp $tarLocal "${VmName}:/tmp/creg-chain-registry-sync.tgz" --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet
gcloud compute ssh $VmName @sshOpts --command="tar -xzf /tmp/creg-chain-registry-sync.tgz -C '$remoteRoot' && rm -f /tmp/creg-chain-registry-sync.tgz" | Out-Null

Remove-Item $tarLocal -Force -ErrorAction SilentlyContinue
Write-Host "[gcp-sync] Local tree synced to $remoteRoot"
