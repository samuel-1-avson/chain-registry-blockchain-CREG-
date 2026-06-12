# GCP-009 — Create Cloud Armor security policy (rate limits + OWASP preview rules).
#
# Attach to the external HTTPS load balancer backend via setup-gcp-public-lb.ps1.
#
# Usage:
#   .\testnet\gcp\setup-cloud-armor.ps1
#   .\testnet\gcp\setup-cloud-armor.ps1 -Confirm
#   .\testnet\gcp\setup-cloud-armor.ps1 -Confirm -AttachToBackend creg-edge-api-backend

param(
    [string]$ProjectId = "",
    [string]$PolicyName = "",
    [int]$RateLimitPerIp = 300,
    [int]$RateLimitIntervalSec = 60,
    [string]$AttachToBackend = "",
    [switch]$Confirm
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $PolicyName) { $PolicyName = if ($cfg.GCP_ARMOR_POLICY_NAME) { $cfg.GCP_ARMOR_POLICY_NAME } else { "creg-testnet-edge" } }
if (-not $AttachToBackend) { $AttachToBackend = if ($cfg.GCP_EDGE_BACKEND_NAME) { $cfg.GCP_EDGE_BACKEND_NAME } else { "creg-edge-api-backend" } }

function Log($m) { Write-Host "[cloud-armor] $m" }

if (-not $Confirm) {
    Write-Host ""
    Write-Host "Will create Cloud Armor policy (GCP-009):" -ForegroundColor Yellow
    Write-Host "  Policy: $PolicyName"
    Write-Host "  Rate limit: $RateLimitPerIp requests / $RateLimitIntervalSec sec per IP (429)"
    Write-Host "  OWASP CRS preview rules (sqli, xss)"
    if ($AttachToBackend) {
        Write-Host "  Attach to backend service: $AttachToBackend (global)"
    }
    Write-Host ""
    Write-Host "Re-run with -Confirm. Create LB first: setup-gcp-public-lb.ps1 -Confirm" -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable compute.googleapis.com --project=$ProjectId | Out-Null

$prevEap = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
$null = gcloud compute security-policies describe $PolicyName --project=$ProjectId 2>&1
$policyExists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = $prevEap

if (-not $policyExists) {
    Log "Creating security policy $PolicyName"
    gcloud compute security-policies create $PolicyName `
        --project=$ProjectId `
        --description="CREG testnet public edge (Phase 2)"
    if ($LASTEXITCODE -ne 0) {
        Log "WARN: Could not create security policy (quota/API). Request Cloud Armor quota in GCP Console:"
        Log "  https://console.cloud.google.com/iam-admin/quotas?project=$ProjectId (filter: SECURITY_POLICIES)"
        Log "LB works without Armor; re-run this script after quota is granted."
        exit 2
    }

    gcloud compute security-policies rules create 1000 `
        --project=$ProjectId `
        --security-policy=$PolicyName `
        --expression="true" `
        --action=rate-based-ban `
        --rate-limit-threshold-count=$RateLimitPerIp `
        --rate-limit-threshold-interval-sec=$RateLimitIntervalSec `
        --ban-duration-sec=300 `
        --conform-action=allow `
        --exceed-action=deny-429 `
        --enforce-on-key=IP | Out-Null

    foreach ($rule in @(
        @{ id = 2000; type = "sqli-v33-stable" }
        @{ id = 2001; type = "xss-v33-stable" }
    )) {
        gcloud compute security-policies rules create $rule.id `
            --project=$ProjectId `
            --security-policy=$PolicyName `
            --expression="evaluatePreconfiguredExpr('$($rule.type)')" `
            --action=deny-403 `
            --preview | Out-Null
    }
} else {
    Log "Policy $PolicyName already exists"
}

if ($AttachToBackend) {
    $ErrorActionPreference = "SilentlyContinue"
    $null = gcloud compute backend-services describe $AttachToBackend --global --project=$ProjectId 2>&1
    $beExists = ($LASTEXITCODE -eq 0)
    $ErrorActionPreference = $prevEap
    if ($beExists -and $policyExists) {
        Log "Attaching policy to backend $AttachToBackend"
        gcloud compute backend-services update $AttachToBackend `
            --project=$ProjectId `
            --global `
            --security-policy=$PolicyName | Out-Null
    } else {
        Log "WARN: Backend $AttachToBackend not found - run setup-gcp-public-lb.ps1 -Confirm first"
    }
}

Log "Done. Policy: $PolicyName"
