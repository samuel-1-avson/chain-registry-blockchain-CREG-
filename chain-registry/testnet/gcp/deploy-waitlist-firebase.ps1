# Deploy waitlist Firestore rules + registerWaitlist Cloud Function.
#
# Usage:
#   .\testnet\gcp\deploy-waitlist-firebase.ps1
#   .\testnet\gcp\deploy-waitlist-firebase.ps1 -WhatIf

param(
    [string]$WaitlistSource = "",
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$workspaceRoot = Split-Path -Parent $repoRoot

if (-not $WaitlistSource) {
    $WaitlistSource = Join-Path $workspaceRoot "Creg-waitlist"
}

if (-not (Test-Path $WaitlistSource)) {
    throw "Waitlist source not found: $WaitlistSource"
}

$firebaseJson = Join-Path $WaitlistSource "firebase.json"
if (-not (Test-Path $firebaseJson)) {
    throw "firebase.json not found in $WaitlistSource (run Firebase commands from Creg-waitlist, not chain-registry)"
}

$configPath = Join-Path $WaitlistSource "firebase-applet-config.json"
$config = Get-Content $configPath -Raw | ConvertFrom-Json
$projectId = $config.projectId
$firestoreDatabaseId = $config.firestoreDatabaseId

Write-Host "[waitlist-firebase] Installing dependencies in $WaitlistSource"
Push-Location $WaitlistSource
try {
    npm install
    npm --prefix functions install
    npm --prefix functions run build

    # Deploy all Firestore configs in firebase.json (named DB). Per-DB targets
    # (firestore:<databaseId>) fail on firebase-tools 11.x; use firestore instead.
    # PowerShell splits on commas unless --only is quoted.
    $deployOnly = "firestore,functions:registerWaitlist"
    if ($WhatIf) {
        Write-Host "[waitlist-firebase] WhatIf: would run: firebase deploy --project $projectId --only `"$deployOnly`""
    } else {
        Write-Host "[waitlist-firebase] Deploying rules + registerWaitlist..."
        npx --yes firebase-tools@latest deploy --project $projectId --only $deployOnly --force
    }
} finally {
    Pop-Location
}

Write-Host "[waitlist-firebase] Done. See docs/WAITLIST_FIREBASE_DEPLOY.md"
