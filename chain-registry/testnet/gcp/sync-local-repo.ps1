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

function Assert-LastExitCode([string]$step) {
    if ($LASTEXITCODE -ne 0) {
        throw "gcloud failed during: $step (exit $LASTEXITCODE)"
    }
}

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteRel = "creg-hosting/$repoSlug/chain-registry"
$tarLocal = Join-Path $env:TEMP "creg-chain-registry-sync.tgz"

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
Assert-LastExitCode "resolve remote home"
$remoteRoot = "$remoteHome/$remoteRel"

Write-Host "[gcp-sync] Packing local repo (excluding target/) ..."
Push-Location $repoRoot
if (Test-Path $tarLocal) {
  try { Remove-Item $tarLocal -Force -ErrorAction Stop }
  catch { $tarLocal = Join-Path $env:TEMP ("creg-chain-registry-sync-" + [guid]::NewGuid().ToString("n") + ".tgz") }
}
& tar -czf $tarLocal --exclude=target --exclude=.git --exclude=node_modules --exclude=hub-web/node_modules .
if ($LASTEXITCODE -ne 0) { throw "tar pack failed" }
& tar -tzf $tarLocal | Out-Null
if ($LASTEXITCODE -ne 0) { throw "local tarball failed integrity check" }
$localBytes = (Get-Item $tarLocal).Length
Pop-Location

Write-Host "[gcp-sync] Uploading to ${VmName}:$remoteRoot ($localBytes bytes) ..."
gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$remoteRoot'" | Out-Null
Assert-LastExitCode "mkdir remote root"
gcloud compute scp $tarLocal "${VmName}:/tmp/creg-chain-registry-sync.tgz" --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet
Assert-LastExitCode "scp tarball"

$extractCmd = @(
    "set -euo pipefail",
    "remote_bytes=`$(stat -c%s /tmp/creg-chain-registry-sync.tgz)",
    "if [ `"`$remote_bytes`" -ne $localBytes ]; then echo upload size mismatch >&2; exit 1; fi",
    "gzip -t /tmp/creg-chain-registry-sync.tgz",
    "tar -xzf /tmp/creg-chain-registry-sync.tgz -C '$remoteRoot'",
    "rm -f /tmp/creg-chain-registry-sync.tgz",
    "find '$remoteRoot'/testnet -name '*.sh' -exec sed -i 's/\r$//' {} +"
) -join "; "
gcloud compute ssh $VmName @sshOpts --command=$extractCmd
Assert-LastExitCode "extract on VM"

Remove-Item $tarLocal -Force -ErrorAction SilentlyContinue
Write-Host "[gcp-sync] Local tree synced to $remoteRoot"
