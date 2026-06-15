# SSH to creg-validator-vm via IAP (no public IP).
#
# Usage:
#   .\testnet\gcp\ssh-validator-vm.ps1
#   .\testnet\gcp\ssh-validator-vm.ps1 -Command "docker ps"

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
if (-not $VmName) {
    $VmName = if ($cfg.GCP_VALIDATOR_VM_NAME) { $cfg.GCP_VALIDATOR_VM_NAME } else { "creg-validator-vm" }
}

$sshBase = @(
    "--zone=$Zone",
    "--project=$ProjectId",
    "--tunnel-through-iap",
    "--strict-host-key-checking=no"
)
if ($Command) {
    gcloud compute ssh $VmName @sshBase --command="$Command"
} else {
    gcloud compute ssh $VmName @sshBase
}
