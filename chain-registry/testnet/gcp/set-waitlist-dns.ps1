# Cloudflare A record for waitlist.cregnet.dev (grey cloud / DNS only for ACME).
#
# Usage:
#   .\testnet\gcp\set-waitlist-dns.ps1 -StaticIp 35.225.225.20
# Requires CF_API_TOKEN (Zone.DNS Edit on cregnet.dev) in env or hosting.env

param(
    [Parameter(Mandatory = $true)]
    [string]$StaticIp,
    [string]$ParentDomain = "",
    [string]$WaitlistHost = "waitlist.cregnet.dev",
    [string]$ApiToken = "",
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ParentDomain) { $ParentDomain = $cfg.PARENT_DOMAIN }
if (-not $ApiToken) { $ApiToken = $env:CF_API_TOKEN; if (-not $ApiToken) { $ApiToken = $cfg.CF_API_TOKEN } }
if (-not $ApiToken) {
    $envPath = $cfg.ENV_FILE
    if (-not $envPath) { $envPath = Join-Path $gcpDir "hosting.env" }
    throw "Set CF_API_TOKEN env var or add a line to hosting.env: CF_API_TOKEN=<token> (file: $envPath). Uncomment CF_API_TOKEN in hosting.env.example."
}

function Log($m) { Write-Host "[waitlist-dns] $m" }

$headers = @{
    Authorization = "Bearer $ApiToken"
    "Content-Type"  = "application/json"
}

$zoneResp = Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones?name=$ParentDomain" -Headers $headers -Method Get
if (-not $zoneResp.success -or $zoneResp.result.Count -lt 1) {
    throw "Zone not found for $ParentDomain"
}
$zoneId = $zoneResp.result[0].id

# waitlist.cregnet.dev -> record name "waitlist" under zone cregnet.dev
$recordName = $WaitlistHost
if ($WaitlistHost.EndsWith(".$ParentDomain")) {
    $recordName = $WaitlistHost.Substring(0, $WaitlistHost.Length - $ParentDomain.Length - 1)
}
$fqdn = "$recordName.$ParentDomain"

$body = @{
    type    = "A"
    name    = $recordName
    content = $StaticIp
    ttl     = 1
    proxied = $false
} | ConvertTo-Json

if ($WhatIf) {
    Log "WhatIf: A $fqdn -> $StaticIp (proxied=false)"
    exit 0
}

$existing = Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records?type=A&name=$fqdn" -Headers $headers -Method Get
if ($existing.result.Count -gt 0) {
    $id = $existing.result[0].id
    Log "Updating A $fqdn -> $StaticIp"
    Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records/$id" -Headers $headers -Method Put -Body $body | Out-Null
} else {
    Log "Creating A $fqdn -> $StaticIp"
    Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records" -Headers $headers -Method Post -Body $body | Out-Null
}

Write-Host ""
Log "Waitlist DNS set for $fqdn (grey cloud / DNS only)."
Log "Wait 2-10 min, then: curl -fsSI https://$fqdn/"
