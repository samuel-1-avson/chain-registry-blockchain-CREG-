# IPFS-001/IPFS-002 — run the operator pin + availability check on the edge VM.
#
# Uploads testnet/ipfs-pin-check.py to creg-testnet-vm (which hosts Kubo on
# :15001), runs it against the public API, and copies the JSON report back to
# testnet/ipfs-pin-logs/ locally as launch-gate evidence.
#
# Usage:
#   .\testnet\gcp\run-ipfs-pin-check.ps1
#   .\testnet\gcp\run-ipfs-pin-check.ps1 -ApiUrl https://api.testnet.cregnet.dev
#   .\testnet\gcp\run-ipfs-pin-check.ps1 -CheckOnly     # availability only, no pin add

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$ApiUrl = "https://api.testnet.cregnet.dev",
    [string]$IpfsApi = "http://localhost:15001",
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) {
    $VmName = if ($cfg.GCP_VM_NAME) { $cfg.GCP_VM_NAME } else { "creg-testnet-vm" }
}

function Log($m) { Write-Host "[ipfs-pin-check] $m" }

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")

$localScript = Join-Path $testnetDir "ipfs-pin-check.py"
if (-not (Test-Path $localScript)) { throw "Missing $localScript" }

Log "Uploading ipfs-pin-check.py to $VmName..."
gcloud compute ssh $VmName @sshOpts --command="mkdir -p ~/creg-pin-check" | Out-Null
gcloud compute scp $localScript "${VmName}:~/creg-pin-check/ipfs-pin-check.py" @sshOpts
if ($LASTEXITCODE -ne 0) { throw "scp failed" }

$skipPin = if ($CheckOnly) { "1" } else { "0" }
$remoteCmd = "cd ~/creg-pin-check && sed -i 's/\r$//' ipfs-pin-check.py && " +
    "CREG_API_URL='$ApiUrl' CREG_IPFS_API='$IpfsApi' CREG_PIN_SKIP_PIN=$skipPin " +
    "CREG_PIN_REPORT_DIR=~/creg-pin-check/reports python3 ipfs-pin-check.py"

Log "Running pin + availability check (API=$ApiUrl IPFS=$IpfsApi)..."
gcloud compute ssh $VmName @sshOpts --command=$remoteCmd
$checkExit = $LASTEXITCODE
if ($checkExit -eq 2) { throw "pin check could not reach the registry API" }

Log "Fetching latest report..."
$latest = (gcloud compute ssh $VmName @sshOpts --command="ls -1t ~/creg-pin-check/reports | head -n1").Trim()
if (-not $latest) { throw "no report produced" }

$outDir = Join-Path $testnetDir "ipfs-pin-logs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
gcloud compute scp "${VmName}:~/creg-pin-check/reports/$latest" (Join-Path $outDir $latest) @sshOpts
if ($LASTEXITCODE -ne 0) { throw "report download failed" }

if ($checkExit -ne 0) {
    Log "FAILED — unavailable package content detected (see $outDir\$latest)"
    exit 1
}
Log "PASSED — all package CIDs pinned and available (evidence: $outDir\$latest)"
