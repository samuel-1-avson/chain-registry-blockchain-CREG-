# Build Creg-waitlist static dist, sync to GCP VM, start nginx + Caddy TLS for waitlist.cregnet.dev.
#
# Usage:
#   .\testnet\gcp\deploy-waitlist.ps1 -BuildOnly
#   .\testnet\gcp\deploy-waitlist.ps1 -Confirm
#   .\testnet\gcp\deploy-waitlist.ps1 -Confirm -SkipDns
#
# Prerequisite: gcloud auth, VM running testnet stack, CF_API_TOKEN for DNS (unless -SkipDns).

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$StaticIp = "35.225.225.20",
    [string]$WaitlistHost = "waitlist.cregnet.dev",
    [string]$WaitlistSource = "",
    [switch]$BuildOnly,
    [switch]$SkipDns,
    [switch]$Confirm,
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$workspaceRoot = Split-Path -Parent $repoRoot
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $WaitlistSource) {
    $WaitlistSource = Join-Path $workspaceRoot "Creg-waitlist"
}

$distDir = Join-Path $testnetDir "waitlist\dist"
$waitlistDockerDir = Join-Path $testnetDir "waitlist"

function Log($m) { Write-Host "[waitlist-deploy] $m" }

if (-not (Test-Path $WaitlistSource)) {
    throw "Waitlist source not found: $WaitlistSource"
}

$waitlistEnv = Join-Path $WaitlistSource ".env"
$dockerEnvArgs = @()
if (Test-Path $waitlistEnv) {
    $dockerEnvArgs += "--env-file"
    $dockerEnvArgs += $waitlistEnv
    Log "Using $waitlistEnv for VITE_RECAPTCHA_SITE_KEY at build time"
} else {
    Write-Host "[waitlist-deploy] WARN: $waitlistEnv not found - production build needs VITE_RECAPTCHA_SITE_KEY" -ForegroundColor Yellow
}

Log "Building waitlist via Linux Node container (avoids Windows rollup issues)..."
docker run --rm `
    -v "${WaitlistSource}:/app" `
    @dockerEnvArgs `
    -w /app `
    node:22-alpine `
    sh -c "rm -rf node_modules dist && npm install && npm run build"

if (-not (Test-Path (Join-Path $distDir "index.html"))) {
    New-Item -ItemType Directory -Force -Path $distDir | Out-Null
}
Log "Copying dist -> $distDir"
if (Test-Path $distDir) { Remove-Item -Recurse -Force $distDir }
Copy-Item -Recurse (Join-Path $WaitlistSource "dist") $distDir

if ($BuildOnly) {
    Log "BuildOnly complete. dist at $distDir"
    exit 0
}

if (-not $Confirm) {
    Write-Host ""
    Write-Host "Pass -Confirm to sync to $VmName, set DNS, and restart waitlist + Caddy." -ForegroundColor Yellow
    exit 0
}

& (Join-Path $gcpDir "sync-local-repo.ps1") -ProjectId $ProjectId -Zone $Zone -VmName $VmName

if (-not $SkipDns) {
    $dnsArgs = @{ StaticIp = $StaticIp; WaitlistHost = $WaitlistHost }
    if ($WhatIf) { $dnsArgs.WhatIf = $true }
    & (Join-Path $gcpDir "set-waitlist-dns.ps1") @dnsArgs
}

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteRel = "creg-hosting/$repoSlug/chain-registry"
$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet")
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
$remoteRoot = "$remoteHome/$remoteRel"

$patchEnv = @"
set -e
ENV='$remoteRoot/testnet/sepolia-3node.env'
touch "`$ENV"
grep -q '^CREG_PUBLIC_WAITLIST_HOST=' "`$ENV" 2>/dev/null && sed -i 's/^CREG_PUBLIC_WAITLIST_HOST=.*/CREG_PUBLIC_WAITLIST_HOST=$WaitlistHost/' "`$ENV" || echo 'CREG_PUBLIC_WAITLIST_HOST=$WaitlistHost' >> "`$ENV"
chmod 600 "`$ENV"
"@

Log "Setting CREG_PUBLIC_WAITLIST_HOST=$WaitlistHost on VM..."
gcloud compute ssh $VmName @sshOpts --command=$patchEnv | Out-Null

$remoteSh = Join-Path $gcpDir "start-remote-waitlist.sh"
$remoteDest = "/tmp/creg-start-remote-waitlist.sh"
gcloud compute scp $remoteSh "${VmName}:${remoteDest}" --zone=$Zone --project=$ProjectId --tunnel-through-iap --strict-host-key-checking=no --quiet

Log "Deploying waitlist container + reloading Caddy on $VmName ..."
gcloud compute ssh $VmName @sshOpts --command="bash $remoteDest '$remoteRoot'"

Log "Verify (after DNS + ACME):"
Write-Host "  curl -fsSI https://$WaitlistHost/" -ForegroundColor Cyan
Write-Host ""
Log "Testnet endpoints unchanged under testnet.cregnet.dev"
