# Deploy waitlist SPA to Firebase Hosting (replaces edge VM static hosting).
# Backend (Firestore + Cloud Functions) stays on the same Firebase project.
#
# Prerequisites:
#   - Creg-waitlist/.env with VITE_RECAPTCHA_SITE_KEY and VITE_WALLETCONNECT_PROJECT_ID
#   - firebase login (or GOOGLE_APPLICATION_CREDENTIALS)
#   - reCAPTCHA / WalletConnect allowlists include Firebase default domains (see .env.example)
#
# Usage:
#   .\deploy-waitlist-hosting.ps1
#   .\deploy-waitlist-hosting.ps1 -SkipBuild   # deploy existing dist/

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$WaitlistRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..\..\Creg-waitlist")

Push-Location $WaitlistRoot
try {
    if (-not $SkipBuild) {
        if (-not (Test-Path ".env")) {
            Write-Error "Missing $WaitlistRoot\.env - copy from .env.example and set VITE_* keys."
        }
        Write-Host "Building waitlist SPA..." -ForegroundColor Cyan
        npm ci
        npm run build
    } elseif (-not (Test-Path "dist\index.html")) {
        Write-Error "dist/index.html missing. Run without -SkipBuild first."
    }

    $env:FUNCTIONS_DISCOVERY_TIMEOUT = "60"
    Write-Host "Deploying Firebase Hosting (project gen-lang-client-0098858574)..." -ForegroundColor Cyan
    $firebaseArgs = @(
        "deploy",
        "--project", "gen-lang-client-0098858574",
        "--only", "hosting"
    )
    $firebaseCmd = Get-Command firebase.cmd -ErrorAction SilentlyContinue
    if ($firebaseCmd) {
        & $firebaseCmd.Source @firebaseArgs
    } else {
        npx --yes firebase-tools@latest @firebaseArgs
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host ""
    Write-Host "Hosting URLs:" -ForegroundColor Green
    Write-Host "  https://gen-lang-client-0098858574.web.app"
    Write-Host "  https://gen-lang-client-0098858574.firebaseapp.com"
    Write-Host ""
    Write-Host "Next: add custom domain waitlist.cregnet.dev in Firebase console, then:"
    Write-Host "  .\set-waitlist-dns-firebase.ps1"
} finally {
    Pop-Location
}
