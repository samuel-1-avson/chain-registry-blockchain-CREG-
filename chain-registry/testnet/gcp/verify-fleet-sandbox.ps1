# MAL-001 — Verify the GCP validator fleet runs a real behavioural sandbox.
#
# Proves (on creg-validator-vm via IAP SSH):
#   - creg-fleet-node1/node2 use the chain-registry-node-secure image
#   - CREG_DEV_SANDBOX is not "true" inside validator containers
#   - nsjail binary works inside validator containers
#   - /v1/health reports sandbox.engine=nsjail and sandbox.dev_bypass=false
#   - No "dev-bypass" in recent validator logs
#
# Writes evidence JSON to testnet/sandbox-301-logs/fleet-sandbox-<ts>.json.
#
# Usage:
#   .\testnet\gcp\verify-fleet-sandbox.ps1

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = ""
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) {
    $VmName = if ($cfg.GCP_VALIDATOR_VM_NAME) { $cfg.GCP_VALIDATOR_VM_NAME } else { "creg-validator-vm" }
}

function Log($m) { Write-Host "[verify-fleet-sandbox] $m" }

function Invoke-Remote([string]$Command) {
    $sshArgs = @(
        "--zone=$Zone", "--project=$ProjectId",
        "--tunnel-through-iap", "--strict-host-key-checking=no", "--quiet"
    )
    $out = gcloud compute ssh $VmName @sshArgs --command=$Command 2>&1
    return @{ exit = $LASTEXITCODE; out = ($out | Out-String).Trim() }
}

$dockerPrefix = "sudo docker"
$probe = Invoke-Remote "docker info >/dev/null 2>&1 && echo plain || echo sudo"
if ($probe.out -match "plain") { $dockerPrefix = "docker" }

$checks = @{
    timestamp      = (Get-Date).ToUniversalTime().ToString("o")
    vm             = $VmName
    mal001         = $false
    validators     = @()
    nsjail_present = $false
    dev_bypass_log = $false
}

foreach ($node in @(
        @{ ctr = "creg-fleet-node1"; port = 28180 },
        @{ ctr = "creg-fleet-node2"; port = 28181 }
    )) {
    $ctr = $node.ctr

    $img = Invoke-Remote "$dockerPrefix inspect --format '{{.Config.Image}}' $ctr"
    if ($img.exit -ne 0) { throw "Container $ctr not running on $VmName - deploy with deploy-validator-fleet.ps1" }
    if ($img.out -notmatch "secure") {
        throw "$ctr image=$($img.out) - expected chain-registry-node-secure:fleet (re-run start-validator-fleet-gcp.sh without CREG_FLEET_DEV_SANDBOX)"
    }

    $sb = Invoke-Remote "$dockerPrefix exec $ctr printenv CREG_DEV_SANDBOX || true"
    if ($sb.out -eq "true") { throw "$ctr has CREG_DEV_SANDBOX=true - MAL-001 requires false on public validators" }

    # Runtime self-report: public /v1/health exposes sandbox status (MAL-001).
    $rc = Invoke-Remote "curl -fsS http://localhost:$($node.port)/v1/health"
    $engine = ""
    $bypass = $null
    if ($rc.exit -eq 0 -and $rc.out) {
        try {
            $json = $rc.out | ConvertFrom-Json
            $engine = $json.sandbox.engine
            $bypass = $json.sandbox.dev_bypass
        } catch { Log "WARN: could not parse /v1/health from $ctr" }
    }
    if ($bypass -eq $true) { throw "$ctr /v1/health reports sandbox.dev_bypass=true" }
    if (-not $engine) {
        throw "$ctr /v1/health missing sandbox.engine - rebuild fleet with CREG_FLEET_BUILD=1 (deploy-validator-fleet.ps1)"
    }
    if ($engine -ne "nsjail") {
        throw "$ctr sandbox.engine=$engine (expected nsjail)"
    }

    Log "$ctr image=$($img.out) CREG_DEV_SANDBOX=$($sb.out) engine=$engine dev_bypass=$bypass"
    $checks.validators += @{
        container   = $ctr
        image       = $img.out
        dev_sandbox = $sb.out
        engine      = $engine
        dev_bypass  = $bypass
    }
}

$nsjail = Invoke-Remote "$dockerPrefix exec creg-fleet-node1 nsjail --help >/dev/null 2>&1 && echo ok || echo missing"
if ($nsjail.out -notmatch "ok") { throw "nsjail not available inside creg-fleet-node1" }
$checks.nsjail_present = $true
Log "nsjail present in validator container"

$logs = Invoke-Remote "$dockerPrefix logs creg-fleet-node1 --tail 500 2>&1 | grep -c 'dev-bypass' || true"
if ($logs.out -match "^[1-9]") {
    $checks.dev_bypass_log = $true
    throw "Found dev-bypass in creg-fleet-node1 logs - sandbox engine must not use dev bypass"
}

$checks.mal001 = $true
$outDir = Join-Path $testnetDir "sandbox-301-logs"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$outPath = Join-Path $outDir ("fleet-sandbox-{0}.json" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
$checks | ConvertTo-Json -Depth 5 | Set-Content -Path $outPath -Encoding utf8
Log "MAL-001 fleet sandbox verify PASSED (evidence: $outPath)"
