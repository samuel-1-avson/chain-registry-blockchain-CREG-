# HOSTING-301 prep - patch chain spec + merge public env vars for GCP/Caddy ingress.
#
# Usage:
#   .\testnet\prepare-public-hosting.ps1 -BaseDomain testnet.creg.dev -AcmeEmail ops@creg.dev
#   .\testnet\prepare-public-hosting.ps1 -BaseDomain testnet.creg.dev -AcmeEmail ops@creg.dev -StaticIp 34.123.45.67

param(
    [Parameter(Mandatory = $true)]
    [string]$BaseDomain,
    [Parameter(Mandatory = $true)]
    [string]$AcmeEmail,
    [string]$StaticIp = "",
    [switch]$SkipChainSpecPatch,
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot

function Log($msg) { Write-Host "[hosting-prep] $msg" }

$apiHost = "api.$BaseDomain"
$explorerHost = "explorer.$BaseDomain"
$faucetHost = "faucet.$BaseDomain"
$specHost = "spec.$BaseDomain"
$ipfsHost = "ipfs.$BaseDomain"

if (-not $SkipChainSpecPatch) {
    Log "Patching chain-spec.sepolia.json for $BaseDomain ..."
    & (Join-Path $scriptDir "patch-sepolia-chain-spec-services.ps1") -BaseDomain $BaseDomain
}

$envPath = Join-Path $scriptDir "sepolia-3node.env"
$envExample = Join-Path $scriptDir "sepolia-3node.env.example"
if (-not (Test-Path $envPath)) {
    if (Test-Path $envExample) {
        Copy-Item $envExample $envPath
        Log "Created $envPath from example (review secrets before deploy)"
    } else {
        throw "Missing $envPath"
    }
}

$lines = Get-Content $envPath
$updates = [ordered]@{
    CREG_ACME_EMAIL            = $AcmeEmail
    CREG_PUBLIC_BASE_DOMAIN    = $BaseDomain
    CREG_PUBLIC_API_HOST       = $apiHost
    CREG_PUBLIC_EXPLORER_HOST  = $explorerHost
    CREG_PUBLIC_FAUCET_HOST    = $faucetHost
    CREG_PUBLIC_SPEC_HOST      = $specHost
    CREG_PUBLIC_IPFS_HOST      = $ipfsHost
    CREG_PUBLIC_EXPLORER_URL   = "https://$explorerHost"
    CREG_NODE_URL              = "https://$apiHost"
    FAUCET_PUBLIC_RPC_URL      = "https://$explorerHost/rpc"
}

$out = New-Object System.Collections.Generic.List[string]
$seen = @{}
foreach ($line in $lines) {
    $replaced = $false
    foreach ($key in $updates.Keys) {
        if ($line -match "^\s*$([regex]::Escape($key))\s*=") {
            $out.Add("$key=$($updates[$key])")
            $seen[$key] = $true
            $replaced = $true
            break
        }
    }
    if (-not $replaced) { $out.Add($line) }
}
foreach ($key in $updates.Keys) {
    if (-not $seen[$key]) { $out.Add("$key=$($updates[$key])") }
}
if ($WhatIf) {
    Log "WhatIf: would update public hosting vars in $envPath (no file written)"
} else {
    Set-Content -Path $envPath -Value $out -Encoding utf8
    Log "Updated public hosting vars in $envPath"
}

Write-Host ""
Write-Host "=== DNS A records (point at VM static IP) ===" -ForegroundColor Cyan
$ipHint = if ($StaticIp) { $StaticIp } else { "<VM_STATIC_IP>" }
@(
    @{ Name = "api.$BaseDomain"; Value = $ipHint }
    @{ Name = "explorer.$BaseDomain"; Value = $ipHint }
    @{ Name = "faucet.$BaseDomain"; Value = $ipHint }
    @{ Name = "spec.$BaseDomain"; Value = $ipHint }
    @{ Name = "ipfs.$BaseDomain"; Value = $ipHint }
) | ForEach-Object { Write-Host ("  {0,-40} A  {1}" -f $_.Name, $_.Value) }

Write-Host ""
Write-Host "=== After DNS propagates (GCP VM) ===" -ForegroundColor Cyan
Write-Host "  git pull && ./testnet/start-3node-gcp.sh"
Write-Host "  docker logs -f creg-3node-caddy"
Write-Host "  .\testnet\hosting-301-verify.ps1 -BaseDomain $BaseDomain"
Write-Host ""
Write-Host "Runbook: testnet/gcp-public-hosting.md" -ForegroundColor DarkGray
