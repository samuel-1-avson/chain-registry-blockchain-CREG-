# GCP-008 — Deploy hub-api to Cloud Run (stateless public status API).
#
# SQLite (HUB_DB_PATH) is ephemeral on Cloud Run unless you add Cloud SQL later.
# Phase 2 uses hub-api for /api/health and /api/status/public probes only.
#
# Usage:
#   .\testnet\gcp\deploy-hub-api-cloudrun.ps1
#   .\testnet\gcp\deploy-hub-api-cloudrun.ps1 -Confirm
#   .\testnet\gcp\deploy-hub-api-cloudrun.ps1 -Confirm -SetJoinHostProxy

param(
    [string]$ProjectId = "",
    [string]$Region = "",
    [string]$ServiceName = "",
    [string]$ArtifactRepo = "",
    [string]$ApiHost = "",
    [string]$JoinHost = "",
    [string]$FaucetHost = "",
    [string]$ExplorerHost = "",
    [string]$SpecHost = "",
    [switch]$Confirm,
    [switch]$SetJoinHostProxy
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$hubApiDir = Join-Path $repoRoot "hub-api"
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Region) { $Region = $cfg.GCP_REGION }
if (-not $ServiceName) { $ServiceName = if ($cfg.GCP_HUB_API_SERVICE) { $cfg.GCP_HUB_API_SERVICE } else { "creg-hub-api" } }
if (-not $ArtifactRepo) { $ArtifactRepo = if ($cfg.GCP_ARTIFACT_REPO) { $cfg.GCP_ARTIFACT_REPO } else { "creg-testnet" } }
if (-not $ApiHost) { $ApiHost = "api.$($cfg.BASE_DOMAIN)" }
if (-not $JoinHost) { $JoinHost = if ($cfg.CREG_PUBLIC_JOIN_HOST) { $cfg.CREG_PUBLIC_JOIN_HOST } else { "join.$($cfg.BASE_DOMAIN)" } }
if (-not $FaucetHost) { $FaucetHost = "faucet.$($cfg.BASE_DOMAIN)" }
if (-not $ExplorerHost) { $ExplorerHost = "explorer.$($cfg.BASE_DOMAIN)" }
if (-not $SpecHost) { $SpecHost = "spec.$($cfg.BASE_DOMAIN)" }

$image = "${Region}-docker.pkg.dev/${ProjectId}/${ArtifactRepo}/${ServiceName}:latest"

function Log($m) { Write-Host "[hub-cloudrun] $m" }

if (-not (Test-Path $hubApiDir)) { throw "Missing hub-api at $hubApiDir" }

if (-not $Confirm) {
    Write-Host ""
    Write-Host "Will deploy hub-api to Cloud Run (GCP-008):" -ForegroundColor Yellow
    Write-Host "  Service: $ServiceName ($Region)"
    Write-Host "  Image:   $image"
    Write-Host "  Public:  --allow-unauthenticated"
    Write-Host "  Upstreams: https://$ApiHost, https://$FaucetHost, https://$ExplorerHost"
  if ($SetJoinHostProxy) {
        Write-Host "  Will write CREG_HUB_API_CLOUD_RUN_URL to sepolia-3node.env for join host Caddy"
    }
    Write-Host ""
    Write-Host "Re-run with -Confirm to build and deploy." -ForegroundColor Yellow
    exit 0
}

gcloud config set project $ProjectId | Out-Null
gcloud services enable run.googleapis.com artifactregistry.googleapis.com cloudbuild.googleapis.com --project=$ProjectId | Out-Null

$repoExists = gcloud artifacts repositories describe $ArtifactRepo --location=$Region --project=$ProjectId 2>$null
if ($LASTEXITCODE -ne 0) {
    Log "Creating Artifact Registry repo $ArtifactRepo"
    gcloud artifacts repositories create $ArtifactRepo `
        --repository-format=docker `
        --location=$Region `
        --project=$ProjectId `
        --description="CREG testnet containers" | Out-Null
}

Log "Building and pushing $image (Cloud Build)..."
gcloud builds submit $hubApiDir `
    --project=$ProjectId `
    --tag=$image `
    --quiet

$envVars = @(
    "HUB_API_PORT=8080",
    "HUB_DB_PATH=/tmp/hub.db",
    "HUB_NODE_API_URL=https://$ApiHost",
    "HUB_FAUCET_URL=https://$FaucetHost",
    "HUB_EXPLORER_URL=https://$ExplorerHost",
    "HUB_SPEC_URL=https://$SpecHost/chain-spec.sepolia.json"
) -join ","

Log "Deploying Cloud Run service $ServiceName ..."
gcloud run deploy $ServiceName `
    --project=$ProjectId `
    --region=$Region `
    --image=$image `
    --platform=managed `
    --port=8080 `
    --cpu=1 `
    --memory=512Mi `
    --min-instances=0 `
    --max-instances=5 `
    --timeout=60s `
    --concurrency=80 `
    --set-env-vars=$envVars `
    --allow-unauthenticated `
    --quiet

$serviceUrl = (gcloud run services describe $ServiceName --region=$Region --project=$ProjectId --format="value(status.url)").Trim()
Log "Service URL: $serviceUrl"

$manifest = Join-Path $gcpDir "hub-api-cloudrun-manifest.json"
@{
    service   = $ServiceName
    region    = $Region
    image     = $image
    url       = $serviceUrl
    deployedAt = (Get-Date).ToUniversalTime().ToString("o")
} | ConvertTo-Json | Set-Content -Path $manifest -Encoding utf8

if ($SetJoinHostProxy) {
    $envFile = Join-Path $testnetDir "sepolia-3node.env"
    $line = "CREG_HUB_API_CLOUD_RUN_URL=$serviceUrl"
    if (Test-Path $envFile) {
        $content = Get-Content $envFile -Raw
        if ($content -match '(?m)^CREG_HUB_API_CLOUD_RUN_URL=') {
            $content = $content -replace '(?m)^CREG_HUB_API_CLOUD_RUN_URL=.*$', $line
        } else {
            $content = $content.TrimEnd() + "`n$line`n"
        }
        Set-Content -Path $envFile -Value $content -Encoding utf8 -NoNewline
        Log "Updated $envFile with CREG_HUB_API_CLOUD_RUN_URL"
    } else {
        Log "WARN: $envFile not found - set CREG_HUB_API_CLOUD_RUN_URL=$serviceUrl manually"
    }
}

Write-Host ""
Write-Host "Health: curl -fsS ${serviceUrl}/api/health" -ForegroundColor Green
Write-Host "Manifest: $manifest" -ForegroundColor Cyan
