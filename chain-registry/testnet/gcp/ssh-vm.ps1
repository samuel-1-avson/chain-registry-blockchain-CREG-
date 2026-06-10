# Open SSH session or run a command on the testnet VM.
#
# Usage:
#   .\testnet\gcp\ssh-vm.ps1
#   .\testnet\gcp\ssh-vm.ps1 -Command "docker ps"

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$Command = ""
)

$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }

$sshBase = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
if ($Command) {
    gcloud compute ssh $VmName @sshBase --command=$Command
} else {
    gcloud compute ssh $VmName @sshBase
}
