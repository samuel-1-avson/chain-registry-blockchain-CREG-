# IPFS-002 — upload pin checker and install hourly cron on the edge VM.
#
# Usage:
#   .\testnet\gcp\setup-ipfs-pin-cron.ps1
#   .\testnet\gcp\setup-ipfs-pin-cron.ps1 -ApiUrl https://api.testnet.cregnet.dev

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$ApiUrl = "https://api.testnet.cregnet.dev",
    [string]$IpfsApi = "http://localhost:15001"
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

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")

function Log($m) { Write-Host "[ipfs-cron] $m" }

$localScript = Join-Path $testnetDir "ipfs-pin-check.py"
$localInstaller = Join-Path $gcpDir "install-ipfs-hourly-cron.sh"
if (-not (Test-Path $localScript)) { throw "Missing $localScript" }
if (-not (Test-Path $localInstaller)) { throw "Missing $localInstaller" }

Log "Uploading pin checker to $VmName..."
gcloud compute ssh $VmName @sshOpts --command="mkdir -p ~/creg-pin-check/reports" | Out-Null
gcloud compute scp $localScript "${VmName}:~/creg-pin-check/ipfs-pin-check.py" @sshOpts
gcloud compute scp $localInstaller "${VmName}:~/creg-pin-check/install-ipfs-hourly-cron.sh" @sshOpts
if ($LASTEXITCODE -ne 0) { throw "upload failed" }

$remoteCmd = "sed -i 's/\r$//' ~/creg-pin-check/*.sh ~/creg-pin-check/*.py 2>/dev/null; " +
    "chmod +x ~/creg-pin-check/install-ipfs-hourly-cron.sh; " +
    "CREG_API_URL='$ApiUrl' CREG_IPFS_API='$IpfsApi' bash ~/creg-pin-check/install-ipfs-hourly-cron.sh"

Log "Installing hourly cron (API=$ApiUrl IPFS=$IpfsApi)..."
gcloud compute ssh $VmName @sshOpts --command=$remoteCmd
if ($LASTEXITCODE -ne 0) { throw "cron install failed" }

Log "Running one-shot pin check for evidence..."
& (Join-Path $gcpDir "run-ipfs-pin-check.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName -ApiUrl $ApiUrl -IpfsApi $IpfsApi
if ($LASTEXITCODE -ne 0) { throw "initial pin check failed" }

Log "IPFS hourly cron installed and first check passed."
