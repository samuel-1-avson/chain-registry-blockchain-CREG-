# Open UDP 51820 for WireGuard on creg-testnet-vm (hybrid validators).
param(
    [string]$ProjectId = "",
    [string]$Tag = "creg-testnet"
)

$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
$fw = "creg-testnet-allow-wireguard"

$prev = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
gcloud compute firewall-rules describe $fw --project=$ProjectId 2>$null | Out-Null
$exists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prev

if ($exists) {
    Write-Host "[wireguard-fw] $fw exists"
    exit 0
}

Write-Host "[wireguard-fw] Creating $fw (udp:51820 -> tag $Tag)..."
gcloud compute firewall-rules create $fw `
    --project=$ProjectId `
    --direction=INGRESS `
    --network=default `
    --action=ALLOW `
    --rules=udp:51820 `
    --target-tags=$Tag `
    --priority=1000
