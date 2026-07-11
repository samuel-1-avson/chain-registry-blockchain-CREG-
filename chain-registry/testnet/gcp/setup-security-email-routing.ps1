#Requires -Version 5.1
<#
.SYNOPSIS
  Enables Cloudflare Email Routing for security@cregnet.dev.

.DESCRIPTION
  Creates (or updates) MX records and a forward rule for security@cregnet.dev.
  Requires a Cloudflare API token with:
    - Zone.DNS (Edit)
    - Zone.Email Routing (Edit)

  Destination inboxes must verify Cloudflare's confirmation email before routing works.

  Token: set CF_API_TOKEN or CLOUDFLARE_API_TOKEN, or add CF_API_TOKEN to hosting.env
  (see _Load-HostingEnv.ps1). DNS-only tokens will fail on Email Routing endpoints.

.PARAMETER DestinationEmail
  Verified inbox that receives forwarded security mail. Defaults to SECURITY_FORWARD_EMAIL, then ACME_EMAIL from hosting.env.

.PARAMETER DryRun
  Print planned actions without calling Cloudflare.
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
  [string]$DestinationEmail = "",
  [string]$ZoneName = "cregnet.dev",
  [string]$LocalPart = "security",
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $gcpDir "_Load-HostingEnv.ps1") | Out-Null
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

$token = $env:CLOUDFLARE_API_TOKEN
if (-not $token) { $token = $env:CF_API_TOKEN }
if (-not $token) { $token = $cfg.CF_API_TOKEN }
if (-not $token) {
  throw "Set CLOUDFLARE_API_TOKEN or CF_API_TOKEN (Zone.DNS + Zone.Email Routing Edit on $ZoneName)."
}

if (-not $DestinationEmail) {
  $DestinationEmail = $cfg.SECURITY_FORWARD_EMAIL
}
if (-not $DestinationEmail) {
  $DestinationEmail = $cfg.ACME_EMAIL
}
if (-not $DestinationEmail) {
  throw "Pass -DestinationEmail or set SECURITY_FORWARD_EMAIL in hosting.env."
}

$headers = @{
  Authorization = "Bearer $token"
  "Content-Type"  = "application/json"
}

function Invoke-CfApi {
  param([string]$Method, [string]$Uri, [object]$Body)
  if ($DryRun -and $Method -ne "GET") {
    Write-Host "[dry-run] $Method $Uri" -ForegroundColor Cyan
    if ($Body) { Write-Host ($Body | ConvertTo-Json -Compress) }
    return @{ success = $true; result = @() }
  }
  $params = @{ Uri = $Uri; Headers = $headers; Method = $Method }
  if ($Body) { $params.Body = ($Body | ConvertTo-Json -Compress) }
  $resp = Invoke-RestMethod @params
  if (-not $resp.success) {
    throw "Cloudflare API error on $Method $Uri : $(($resp.errors | ConvertTo-Json -Compress))"
  }
  return $resp
}

Write-Host "Cloudflare Email Routing setup for ${LocalPart}@${ZoneName} -> $DestinationEmail" -ForegroundColor Cyan

$zone = (Invoke-CfApi GET "https://api.cloudflare.com/client/v4/zones?name=$ZoneName").result[0]
if (-not $zone) { throw "Zone not found: $ZoneName" }
$zoneId = $zone.id
Write-Host "  Zone ID: $zoneId"

try {
  $status = Invoke-CfApi GET "https://api.cloudflare.com/client/v4/zones/$zoneId/email/routing"
  Write-Host "  Email routing status: $($status.result | ConvertTo-Json -Compress)"
} catch {
  Write-Warning "Email Routing API not accessible (token may lack Zone.Email Routing Edit). Use the dashboard steps printed at the end."
}

if ($PSCmdlet.ShouldProcess($ZoneName, "Enable Email Routing")) {
  try {
    Invoke-CfApi POST "https://api.cloudflare.com/client/v4/zones/$zoneId/email/routing/enable" | Out-Null
    Write-Host "  Enabled Email Routing." -ForegroundColor Green
  } catch {
    Write-Warning "Could not enable Email Routing via API: $($_.Exception.Message)"
  }
}

if ($PSCmdlet.ShouldProcess($DestinationEmail, "Register destination address")) {
  try {
    $destBody = @{ email = $DestinationEmail }
    Invoke-CfApi POST "https://api.cloudflare.com/client/v4/accounts/$($zone.account.id)/email/routing/addresses" $destBody | Out-Null
    Write-Host "  Destination registration requested — check $DestinationEmail for Cloudflare verification." -ForegroundColor Yellow
  } catch {
    try {
      Invoke-CfApi POST "https://api.cloudflare.com/client/v4/zones/$zoneId/email/routing/addresses" $destBody | Out-Null
      Write-Host "  Destination registration requested (zone scope)." -ForegroundColor Yellow
    } catch {
      Write-Warning "Could not register destination: $($_.Exception.Message)"
    }
  }
}

$customAddress = "${LocalPart}@${ZoneName}"
if ($PSCmdlet.ShouldProcess($customAddress, "Create routing rule")) {
  $ruleBody = @{
    name = "security-disclosure"
    enabled = $true
    matchers = @(@{ type = "literal"; field = "to"; value = $customAddress })
    actions = @(@{ type = "forward"; value = @($DestinationEmail) })
  }
  try {
    Invoke-CfApi POST "https://api.cloudflare.com/client/v4/zones/$zoneId/email/routing/rules" $ruleBody | Out-Null
    Write-Host "  Routing rule created for $customAddress." -ForegroundColor Green
  } catch {
    Write-Warning "Could not create routing rule: $($_.Exception.Message)"
  }
}

Write-Host ""
Write-Host "Verify MX after propagation:" -ForegroundColor Cyan
Write-Host "  cd F:\project\chain-registry\CREG-Contribution-Arena; npm run check:cregnet-dns"
Write-Host ""
Write-Host "If API calls failed, complete manually in Cloudflare dashboard:" -ForegroundColor Yellow
Write-Host "  1. Email -> Email Routing -> Enable"
Write-Host "  2. Destination addresses -> add and verify $DestinationEmail"
Write-Host "  3. Routing rules -> Custom address $customAddress -> forward to $DestinationEmail"
Write-Host "  Token needs Zone.Email Routing Edit (current DNS-only token is insufficient)."