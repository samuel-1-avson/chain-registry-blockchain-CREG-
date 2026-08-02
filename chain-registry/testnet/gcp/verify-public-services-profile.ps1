# Read-only preflight for the GCP public-services profile.
#
# This script deliberately does not create or modify cloud resources. It checks
# that the environment describes an observer/edge deployment and that no
# validator private material or legacy validator placement is being sent to GCP.
# Validators and bootnodes remain independently operated outside GCP.
#
# Usage:
#   .\verify-public-services-profile.ps1
#   .\verify-public-services-profile.ps1 -Target observer
#   .\verify-public-services-profile.ps1 -EnvFile ..\sepolia-3node.env -Target edge

param(
    [string]$EnvFile = "",
    [ValidateSet("all", "observer", "edge")]
    [string]$Target = "all"
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
if (-not $EnvFile) {
    $fleetEnv = Join-Path $testnetDir "sepolia-fleet.env"
    $legacyEnv = Join-Path $testnetDir "sepolia-3node.env"
    $EnvFile = if (Test-Path $fleetEnv) { $fleetEnv } else { $legacyEnv }
}
if (-not (Test-Path -LiteralPath $EnvFile)) {
    throw "Missing $EnvFile. Copy sepolia-fleet.env.example and fill operator-approved values."
}

$values = @{}
$lineNumber = 0
foreach ($line in Get-Content -LiteralPath $EnvFile) {
    $lineNumber++
    if ($line -match '^\s*#' -or $line -match '^\s*$') { continue }
    if ($line -notmatch '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$') {
        throw "Invalid env syntax at $EnvFile`:$lineNumber"
    }
    $key = $Matches[1]
    $value = $Matches[2].Trim()
    if ($value.Length -ge 2 -and (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'")))) {
        $value = $value.Substring(1, $value.Length - 2)
    }
    $values[$key] = $value
}

$failures = [System.Collections.Generic.List[string]]::new()
function Require-Value([string]$Name, [string]$Reason, [switch]$ForObserver, [switch]$ForEdge) {
    if (($ForObserver -and $Target -eq "edge") -or ($ForEdge -and $Target -eq "observer")) { return }
    $value = if ($values.ContainsKey($Name)) { [string]$values[$Name] } else { "" }
    if ([string]::IsNullOrWhiteSpace($value) -or $value -match '(?i)(YOUR_|replace-with|example\.com|\.\.\.)') {
        $failures.Add("${Name}: $Reason")
    }
}

$legacy = @("CREG_VALIDATOR_FLEET_MODE", "CREG_HYBRID_MODE") | Where-Object {
    $values.ContainsKey($_) -and [string]$values[$_] -match '^(?i:true|1|yes)$'
}
if ($legacy.Count -gt 0) {
    $failures.Add("Legacy validator placement is enabled ($($legacy -join ', ')); validators must run outside GCP.")
}

# Do not allow validator private material to cross the deployment boundary.
foreach ($key in $values.Keys) {
    if ($key -match '(?i)(^|_)SPEC_SIGNING_KEY$|(^|_)GENESIS_SIGNING_KEY$|CREG_VALIDATOR_(KEY|PRIVATE|SECRET|SEED)') {
        $failures.Add("$key must not be present in a GCP public-services env file.")
    }
}

Require-Value "CREG_ETH_RPC" "set an approved Sepolia RPC URL"
Require-Value "CREG_VALIDATOR_P2P_SEEDS" "provide independently operated bootnode/validator multiaddrs"
Require-Value "CREG_VALIDATOR_HTTP_PEERS" "provide independently operated validator HTTP peers"
Require-Value "CREG_OPERATOR_API_KEY" "provide a high-entropy server-side operator key (never a sample)"

$signedImages = if ($values.ContainsKey("CREG_REQUIRE_SIGNED_IMAGES")) { [string]$values["CREG_REQUIRE_SIGNED_IMAGES"] } else { "false" }
if ($signedImages -match '^(?i:true|1|yes)$') {
    $image = if ($values.ContainsKey("CREG_FLEET_IMAGE")) { [string]$values["CREG_FLEET_IMAGE"] } else { "" }
    if ($Target -ne "edge" -and $image -notmatch '@sha256:[0-9a-fA-F]{64}$') {
        $failures.Add("CREG_REQUIRE_SIGNED_IMAGES=true requires CREG_FLEET_IMAGE to end in an approved @sha256:<64-hex-digest>.")
    }
}

if ($Target -ne "observer") {
    $observerTarget = if ($values.ContainsKey("CREG_OBSERVER_API_UPSTREAM")) { [string]$values["CREG_OBSERVER_API_UPSTREAM"] } else { "" }
    if (-not $observerTarget -and $values.ContainsKey("CREG_OBSERVER_POOL_LB_IP")) {
        $observerTarget = [string]$values["CREG_OBSERVER_POOL_LB_IP"]
    }
    if ([string]::IsNullOrWhiteSpace($observerTarget) -or $observerTarget -match '(?i)(YOUR_|replace-with|example\.com|\.\.\.)') {
        $failures.Add("Set CREG_OBSERVER_API_UPSTREAM or CREG_OBSERVER_POOL_LB_IP before deploying the edge.")
    } elseif ($observerTarget -match ',') {
        $failures.Add("The edge requires one observer upstream; use a single ILB address in CREG_OBSERVER_API_UPSTREAM (comma-separated targets belong only in monitoring).")
    }
}

$mandatory = if ($values.ContainsKey("CREG_REQUIRE_CERTIFIED_COMMITTEES")) { [string]$values["CREG_REQUIRE_CERTIFIED_COMMITTEES"] } else { "false" }
if ($mandatory -match '^(?i:true|1|yes)$') {
    Require-Value "CREG_WEAK_SUBJECTIVITY_CHECKPOINT" "mandatory mode requires the independently verified signed checkpoint certificate"
}
if ($values.ContainsKey("CREG_WEAK_SUBJECTIVITY_CHECKPOINT") -and [string]$values["CREG_WEAK_SUBJECTIVITY_CHECKPOINT"] -match '(?i)(schema_version.*\.\.\.|YOUR_|replace-with)') {
    $failures.Add("CREG_WEAK_SUBJECTIVITY_CHECKPOINT still contains a template placeholder.")
}

Write-Host "Public-services preflight: $EnvFile" -ForegroundColor Cyan
Write-Host "  Target: $Target"
Write-Host "  Validators/bootnodes: external (no private validator material in env)"
Write-Host "  Certified committees: $mandatory"
if ($failures.Count -gt 0) {
    Write-Host "FAIL ($($failures.Count) issue(s))" -ForegroundColor Red
    $failures | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
    exit 1
}
Write-Host "PASS: environment is suitable for the selected GCP public-services profile." -ForegroundColor Green
