# GCP-009 — External HTTPS load balancer in front of edge VM API (optional DNS cutover).
#
# Creates a global HTTPS LB + managed cert for api.<base-domain>. Cloud Armor attaches
# via setup-cloud-armor.ps1. DNS can stay on the edge VM until you point api.* to the LB IP.
#
# Usage:
#   .\testnet\gcp\setup-gcp-public-lb.ps1
#   .\testnet\gcp\setup-gcp-public-lb.ps1 -Confirm
#   .\testnet\gcp\setup-gcp-public-lb.ps1 -Confirm -ApiHost api.testnet.cregnet.dev

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$ApiHost = "",
    [string]$LbIpName = "",
    [string]$BackendName = "",
    [string]$UrlMapName = "",
    [string]$ProxyName = "",
    [string]$ForwardingRuleName = "",
    [string]$CertName = "",
    [int]$BackendPort = 443,
    [switch]$Confirm
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $ApiHost) { $ApiHost = "api.$($cfg.BASE_DOMAIN)" }
if (-not $LbIpName) { $LbIpName = if ($cfg.GCP_API_LB_IP_NAME) { $cfg.GCP_API_LB_IP_NAME } else { "creg-api-lb-ip" } }
if (-not $BackendName) { $BackendName = if ($cfg.GCP_EDGE_BACKEND_NAME) { $cfg.GCP_EDGE_BACKEND_NAME } else { "creg-edge-api-backend" } }
if (-not $UrlMapName) { $UrlMapName = "creg-api-url-map" }
if (-not $ProxyName) { $ProxyName = "creg-api-https-proxy" }
if (-not $ForwardingRuleName) { $ForwardingRuleName = "creg-api-https-fr" }
if (-not $CertName) { $CertName = "creg-api-managed-cert" }

$igName = "creg-edge-unmanaged-ig"
$hcName = "creg-edge-https-hc"

function Log($m) { Write-Host "[public-lb] $m" }

if (-not $Confirm) {
    Write-Host ""
    Write-Host "Will create external HTTPS LB (GCP-009 prerequisite):" -ForegroundColor Yellow
    Write-Host "  Global IP: $LbIpName"
    Write-Host "  Backend:   $BackendName -> $VmName`:$BackendPort (HTTPS to Caddy)"
    Write-Host "  Host:      $ApiHost (Google-managed certificate)"
    Write-Host "  URL map:   default -> $BackendName"
    Write-Host ""
    Write-Host "After -Confirm:" -ForegroundColor Cyan
    Write-Host "  1. setup-cloud-armor.ps1 -Confirm"
    Write-Host "  2. Point DNS A record for $ApiHost to LB IP (optional; test with Host header first)"
    Write-Host ""
    Write-Host "Re-run with -Confirm to proceed." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null

# Global IP
$ipExists = gcloud compute addresses describe $LbIpName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute addresses create $LbIpName --global --project=$ProjectId | Out-Null
}
$lbIp = (gcloud compute addresses describe $LbIpName --global --project=$ProjectId --format="value(address)").Trim()
Log "LB IP: $lbIp"

# Unmanaged instance group (edge VM)
$igExists = gcloud compute instance-groups unmanaged describe $igName --zone=$Zone --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute instance-groups unmanaged create $igName --zone=$Zone --project=$ProjectId | Out-Null
    gcloud compute instance-groups unmanaged add-instances $igName `
        --zone=$Zone --project=$ProjectId --instances=$VmName | Out-Null
}

gcloud compute instance-groups unmanaged set-named-ports $igName `
    --zone=$Zone --project=$ProjectId --named-ports="https:${BackendPort}" | Out-Null

# Health check (HTTPS to /v1/health via Caddy)
$hcExists = gcloud compute health-checks describe $hcName --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute health-checks create https $hcName `
        --project=$ProjectId `
        --port=$BackendPort `
        --request-path=/v1/health `
        --host=$ApiHost `
        --check-interval=15s `
        --timeout=5s `
        --unhealthy-threshold=3 `
        --healthy-threshold=2 | Out-Null
}

$beExists = gcloud compute backend-services describe $BackendName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute backend-services create $BackendName `
        --project=$ProjectId `
        --global `
        --protocol=HTTPS `
        --port-name=https `
        --health-checks=$hcName `
        --timeout=30s | Out-Null
}

$backendCount = (gcloud compute backend-services describe $BackendName --global --project=$ProjectId --format="value(backends.group)" 2>$null | Where-Object { $_ }).Count
if (-not $backendCount) {
    Log "Attaching $igName to backend $BackendName"
    gcloud compute backend-services add-backend $BackendName `
        --project=$ProjectId `
        --global `
        --instance-group=$igName `
        --instance-group-zone=$Zone | Out-Null
}

$certExists = gcloud compute ssl-certificates describe $CertName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute ssl-certificates create $CertName `
        --project=$ProjectId `
        --domains=$ApiHost `
        --global | Out-Null
}

$umExists = gcloud compute url-maps describe $UrlMapName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute url-maps create $UrlMapName `
        --project=$ProjectId `
        --default-service=$BackendName | Out-Null
}

$proxyExists = gcloud compute target-https-proxies describe $ProxyName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute target-https-proxies create $ProxyName `
        --project=$ProjectId `
        --url-map=$UrlMapName `
        --ssl-certificates=$CertName | Out-Null
}

$frExists = gcloud compute forwarding-rules describe $ForwardingRuleName --global --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    gcloud compute forwarding-rules create $ForwardingRuleName `
        --project=$ProjectId `
        --global `
        --target-https-proxy=$ProxyName `
        --address=$LbIpName `
        --ports=443 | Out-Null
}

$stateFile = Join-Path $gcpDir "public-lb-state.env"
@"
GCP_API_LB_IP=$lbIp
GCP_API_LB_HOST=$ApiHost
GCP_EDGE_BACKEND_NAME=$BackendName
"@ | Set-Content -Path $stateFile -Encoding utf8

Log "Done. LB IP $lbIp for $ApiHost"
Write-Host "Next: .\testnet\gcp\setup-cloud-armor.ps1 -Confirm" -ForegroundColor Cyan
Write-Host "State: $stateFile" -ForegroundColor Cyan
