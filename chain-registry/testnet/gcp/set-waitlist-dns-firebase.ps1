# Point waitlist.cregnet.dev at Firebase Hosting (Cloudflare DNS, DNS-only / grey cloud).
#
# Removes legacy A records (edge VM) and ensures a single CNAME to Firebase Hosting.
#
# Usage:
#   $env:CLOUDFLARE_API_TOKEN = "..."
#   .\set-waitlist-dns-firebase.ps1
#   .\set-waitlist-dns-firebase.ps1 -FirebaseTarget "gen-lang-client-0098858574.web.app"

param(
    [string]$ZoneName = "cregnet.dev",
    [string]$RecordName = "waitlist",
    [string]$FirebaseTarget = "gen-lang-client-0098858574.web.app",
    [string]$LegacyVmIp = "35.225.225.20"
)

$ErrorActionPreference = "Stop"

if (-not $env:CLOUDFLARE_API_TOKEN) {
    Write-Error "Set CLOUDFLARE_API_TOKEN (Zone.DNS Edit on $ZoneName)."
}

$headers = @{
    Authorization = "Bearer $env:CLOUDFLARE_API_TOKEN"
    "Content-Type" = "application/json"
}

$zoneRes = Invoke-RestMethod -Uri "https://api.cloudflare.com/client/v4/zones?name=$ZoneName" -Headers $headers
if (-not $zoneRes.result -or $zoneRes.result.Count -eq 0) {
    Write-Error "Zone not found: $ZoneName"
}
$zoneId = $zoneRes.result[0].id
$fqdn = "$RecordName.$ZoneName"

$listUri = "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records?name=$fqdn"
$existing = Invoke-RestMethod -Uri $listUri -Headers $headers

foreach ($rec in $existing.result) {
    if ($rec.type -eq "A") {
        Write-Host "Deleting A record $fqdn -> $($rec.content)" -ForegroundColor Yellow
        Invoke-RestMethod -Method Delete -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records/$($rec.id)" -Headers $headers | Out-Null
    }
    if ($rec.type -eq "CNAME" -and $rec.content -ne $FirebaseTarget) {
        Write-Host "Deleting wrong CNAME $fqdn -> $($rec.content)" -ForegroundColor Yellow
        Invoke-RestMethod -Method Delete -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records/$($rec.id)" -Headers $headers | Out-Null
    }
}

$existing = Invoke-RestMethod -Uri $listUri -Headers $headers
$cname = $existing.result | Where-Object { $_.type -eq "CNAME" } | Select-Object -First 1

$body = @{
    type    = "CNAME"
    name    = $RecordName
    content = $FirebaseTarget
    proxied = $false
    ttl     = 300
} | ConvertTo-Json

if ($cname) {
    Write-Host "Updating $fqdn CNAME -> $FirebaseTarget (proxied=false)" -ForegroundColor Cyan
    Invoke-RestMethod -Method Put -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records/$($cname.id)" -Headers $headers -Body $body | Out-Null
} else {
    Write-Host "Creating $fqdn CNAME -> $FirebaseTarget (proxied=false)" -ForegroundColor Cyan
    Invoke-RestMethod -Method Post -Uri "https://api.cloudflare.com/client/v4/zones/$zoneId/dns_records" -Headers $headers -Body $body | Out-Null
}

Write-Host ""
Write-Host "Cloudflare DNS for $fqdn :" -ForegroundColor Green
(Invoke-RestMethod -Uri $listUri -Headers $headers).result | ForEach-Object {
    Write-Host "  $($_.type) $($_.name) -> $($_.content) proxied=$($_.proxied)"
}

Write-Host ""
Write-Host "In Firebase Hosting, close/reopen the domain dialog or wait a few minutes." -ForegroundColor Yellow
Write-Host "Status should move from 'Records not detected' to 'Minting certificate' / Connected." -ForegroundColor Yellow
Write-Host "Then stop edge VM: .\stop-testnet-hosting.ps1 -Yes" -ForegroundColor Yellow
