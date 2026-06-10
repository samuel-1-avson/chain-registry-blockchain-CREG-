# Create/update Cloudflare A records for HOSTING-301 (DNS only / grey cloud).
#
# Requires CF_API_TOKEN with Zone.DNS Edit on PARENT_DOMAIN.
# Or set in testnet/gcp/hosting.env
#
# Usage:
#   $env:CF_API_TOKEN = "..."
#   .\testnet\gcp\set-cloudflare-dns.ps1 -StaticIp 34.x.x.x

param(
    [Parameter(Mandatory = $true)]
    [string]$StaticIp,
    [string]$ParentDomain = "",
    [string]$BaseDomain = "",
    [string]$ApiToken = "",
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ParentDomain) { $ParentDomain = $cfg.PARENT_DOMAIN }
if (-not $BaseDomain) { $BaseDomain = $cfg.BASE_DOMAIN }
if (-not $ApiToken) { $ApiToken = $env:CF_API_TOKEN; if (-not $ApiToken) { $ApiToken = $cfg.CF_API_TOKEN } }
if (-not $ApiToken) {
    throw "Set CF_API_TOKEN env var or CF_API_TOKEN in hosting.env. Create token: Cloudflare Dashboard -> My Profile -> API Tokens -> Zone.DNS Edit for $ParentDomain"
}

function Log($m) { Write-Host "[cf-dns] $m" }

$headers = @{
    Authorization = "Bearer $ApiToken"
    "Content-Type"  = "application/json"
}

Log "Looking up zone $ParentDomain ..."
$zoneResp = Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones?name=$ParentDomain" -Headers $headers -Method Get
if (-not $zoneResp.success -or $zoneResp.result.Count -lt 1) {
    throw "Zone not found for $ParentDomain (check token permissions)"
}
$zoneId = $zoneResp.result[0].id
Log "Zone ID: $zoneId"

# BaseDomain testnet.cregnet.dev -> record names relative to cregnet.dev: api.testnet, explorer.testnet, ...
$suffix = $BaseDomain
if ($suffix.EndsWith(".$ParentDomain")) {
    $suffix = $suffix.Substring(0, $suffix.Length - $ParentDomain.Length - 1)
}
$hosts = @("api", "explorer", "faucet", "spec", "ipfs", $suffix) | ForEach-Object {
    if ($_ -eq $suffix) { $suffix } else { "$_.${suffix}" }
}

foreach ($recName in $hosts) {
    $fqdn = "$recName.$ParentDomain"
    $body = @{
        type    = "A"
        name    = $recName
        content = $StaticIp
        ttl     = 1
        proxied = $false
    } | ConvertTo-Json

    $existing = Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records?type=A&name=$fqdn" -Headers $headers -Method Get
    if ($WhatIf) {
        Log "WhatIf: A $fqdn -> $StaticIp (proxied=false)"
        continue
    }

    if ($existing.result.Count -gt 0) {
        $id = $existing.result[0].id
        Log "Updating A $fqdn -> $StaticIp"
        Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records/$id" -Headers $headers -Method Put -Body $body | Out-Null
    } else {
        Log "Creating A $fqdn -> $StaticIp"
        Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records" -Headers $headers -Method Post -Body $body | Out-Null
    }
}

Write-Host ""
Log "DNS records set (proxied=false for Let's Encrypt). Wait 2-10 min for propagation."
Write-Host "Check: nslookup api.$BaseDomain"
