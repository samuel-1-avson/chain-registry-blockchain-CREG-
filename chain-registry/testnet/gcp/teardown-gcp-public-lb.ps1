# Remove external HTTPS API load balancer (waitlist-only cost cut).
# Does NOT stop creg-testnet-vm or the edge instance group.
#
# Usage:
#   .\teardown-gcp-public-lb.ps1
#   .\teardown-gcp-public-lb.ps1 -Yes

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string]$ProjectId = "",
    [string]$LbIpName = "",
    [string]$BackendName = "",
    [string]$UrlMapName = "",
    [string]$ProxyName = "",
    [string]$ForwardingRuleName = "",
    [string]$CertName = "",
    [string]$HttpHcName = "creg-edge-http-hc",
    [string]$HttpsHcName = "creg-edge-https-hc",
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $LbIpName) { $LbIpName = if ($cfg.GCP_API_LB_IP_NAME) { $cfg.GCP_API_LB_IP_NAME } else { "creg-api-lb-ip" } }
if (-not $BackendName) { $BackendName = if ($cfg.GCP_EDGE_BACKEND_NAME) { $cfg.GCP_EDGE_BACKEND_NAME } else { "creg-edge-api-backend" } }
if (-not $UrlMapName) { $UrlMapName = "creg-api-url-map" }
if (-not $ProxyName) { $ProxyName = "creg-api-https-proxy" }
if (-not $ForwardingRuleName) { $ForwardingRuleName = "creg-api-https-fr" }
if (-not $CertName) { $CertName = "creg-api-managed-cert" }

function Log($m) { Write-Host "[teardown-lb] $m" }

if (-not (Get-Command gcloud -ErrorAction SilentlyContinue)) {
    throw "gcloud not found"
}

$doIt = $Yes
if (-not $doIt -and -not $WhatIfPreference) {
    Write-Host "Will delete API HTTPS LB + release $LbIpName. Re-run with -Yes." -ForegroundColor Yellow
    exit 0
}

function Invoke-Gcloud([string[]]$GcloudArgs) {
    if ($WhatIfPreference) {
        Write-Host "[WhatIf] gcloud $($GcloudArgs -join ' ')" -ForegroundColor DarkGray
        return
    }
    & gcloud @GcloudArgs
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $($GcloudArgs -join ' ')" }
}

function Remove-IfExists([string]$Kind, [string]$Name, [string[]]$ExtraArgs) {
    $exists = gcloud compute $Kind describe $Name @ExtraArgs --project=$ProjectId 2>$null
    if (-not $exists) {
        Log "$Kind $Name not found - skip"
        return
    }
    if (-not $doIt -and -not $PSCmdlet.ShouldProcess($Name, "Delete $Kind")) { return }
    if ($doIt) {
        Log "Deleting $Kind $Name ..."
        Invoke-Gcloud @("compute", $Kind, "delete", $Name, "--project=$ProjectId", "--quiet") + $ExtraArgs
    }
}

gcloud config set project $ProjectId | Out-Null

Remove-IfExists "forwarding-rules" $ForwardingRuleName @("--global")
Remove-IfExists "target-https-proxies" $ProxyName @("--global")
Remove-IfExists "url-maps" $UrlMapName @("--global")
Remove-IfExists "ssl-certificates" $CertName @("--global")
Remove-IfExists "backend-services" $BackendName @("--global")
Remove-IfExists "health-checks" $HttpHcName @()
Remove-IfExists "health-checks" $HttpsHcName @()
Remove-IfExists "addresses" $LbIpName @("--global")

$stateFile = Join-Path $gcpDir "public-lb-state.env"
if (Test-Path $stateFile) {
    Remove-Item $stateFile -Force
    Log "Removed $stateFile"
}

Log 'Done. Recreate with setup-gcp-public-lb.ps1 -Confirm'
