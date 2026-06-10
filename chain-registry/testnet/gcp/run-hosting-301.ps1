# HOSTING-301 end-to-end orchestrator (gcloud + optional Cloudflare API).
#
# Usage:
#   Copy hosting.env.example -> hosting.env, edit if needed
#   .\testnet\gcp\run-hosting-301.ps1 -Step all -Confirm
#   .\testnet\gcp\run-hosting-301.ps1 -Step provision -Confirm
#   .\testnet\gcp\run-hosting-301.ps1 -Step dns -StaticIp 34.x.x.x
#   .\testnet\gcp\run-hosting-301.ps1 -Step prep
#   .\testnet\gcp\run-hosting-301.ps1 -Step deploy
#   .\testnet\gcp\run-hosting-301.ps1 -Step verify

param(
    [ValidateSet("check", "provision", "prep", "dns", "push", "deploy", "verify", "all")]
    [string]$Step = "check",
    [string]$StaticIp = "",
    [switch]$Confirm,
    [switch]$SkipDns
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

function Log($m) { Write-Host "[hosting-301] $m" }

function Get-StaticIpFromState {
    $statePath = Join-Path $gcpDir "hosting-state.json"
    if (Test-Path $statePath) {
        return (Get-Content $statePath | ConvertFrom-Json).staticIp
    }
    return ""
}

function Step-Check {
    Log "gcloud account: $(gcloud config get-value account 2>$null)"
    Log "gcloud project: $(gcloud config get-value project 2>$null)"
    Log "BASE_DOMAIN: $($cfg.BASE_DOMAIN)"
    Log "ACME_EMAIL: $($cfg.ACME_EMAIL)"
    if (-not (Test-Path (Join-Path $gcpDir "hosting.env"))) {
        Write-Host "Tip: copy hosting.env.example -> hosting.env" -ForegroundColor DarkYellow
    }
    $envFile = Join-Path $testnetDir "sepolia-3node.env"
    if (Test-Path $envFile) { Log "sepolia-3node.env: present" } else { Log "sepolia-3node.env: MISSING" }
}

function Step-Provision {
    $args = @{ Confirm = $Confirm }
    if (-not $Confirm) { Log "Provision dry-run (add -Confirm to create VM)" }
    & (Join-Path $gcpDir "provision-vm.ps1") @args
}

function Step-Prep {
    $ip = $StaticIp
    if (-not $ip) { $ip = Get-StaticIpFromState }
    if (-not $ip) { throw "Need -StaticIp or run provision first (hosting-state.json)" }
    if (-not $cfg.ACME_EMAIL) { throw "Set ACME_EMAIL in hosting.env" }
    & (Join-Path $testnetDir "prepare-public-hosting.ps1") `
        -BaseDomain $cfg.BASE_DOMAIN `
        -AcmeEmail $cfg.ACME_EMAIL `
        -StaticIp $ip
}

function Step-Dns {
    $ip = $StaticIp
    if (-not $ip) { $ip = Get-StaticIpFromState }
    if (-not $ip) { throw "Need -StaticIp or run provision first" }
    if ($SkipDns) {
        Log "SkipDns - add these A records in Cloudflare (DNS only):"
        & (Join-Path $testnetDir "prepare-public-hosting.ps1") -BaseDomain $cfg.BASE_DOMAIN -AcmeEmail $cfg.ACME_EMAIL -StaticIp $ip -WhatIf -SkipChainSpecPatch | Out-Null
        return
    }
    & (Join-Path $gcpDir "set-cloudflare-dns.ps1") -StaticIp $ip
}

function Step-Push {
    & (Join-Path $gcpDir "push-env.ps1")
}

function Step-Deploy {
    & (Join-Path $gcpDir "deploy-stack.ps1") -PushEnv
}

function Step-Verify {
    & (Join-Path $testnetDir "hosting-301-verify.ps1") -BaseDomain $cfg.BASE_DOMAIN
}

switch ($Step) {
    "check" { Step-Check }
    "provision" { Step-Provision }
    "prep" { Step-Prep }
    "dns" { Step-Dns }
    "push" { Step-Push }
    "deploy" { Step-Deploy }
    "verify" { Step-Verify }
    "all" {
        Step-Check
        Step-Provision
        if (-not $Confirm) {
            Log "Stopped before create (all requires -Confirm). Re-run: -Step all -Confirm"
            exit 0
        }
        Start-Sleep -Seconds 30
        Step-Prep
        if (-not $SkipDns) {
            try { Step-Dns } catch {
                Log "DNS automation failed: $($_.Exception.Message)"
                Log "Add A records manually in Cloudflare, then continue with -Step deploy"
                throw
            }
        }
        Log "Waiting 120s for DNS propagation..."
        Start-Sleep -Seconds 120
        Step-Deploy
        Step-Verify
    }
}
