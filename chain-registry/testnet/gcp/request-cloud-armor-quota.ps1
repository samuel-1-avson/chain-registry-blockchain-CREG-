# Request Cloud Armor SECURITY_POLICIES quota for the testnet GCP project.
# Quota cannot be raised via gcloud alone — this script checks current limit and prints the console steps.
#
# Usage:
#   .\testnet\gcp\request-cloud-armor-quota.ps1
#   .\testnet\gcp\request-cloud-armor-quota.ps1 -DesiredPolicies 5

param(
    [string]$ProjectId = "",
    [int]$DesiredPolicies = 5,
    [switch]$OpenConsole
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")
if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }

function Log($m) { Write-Host "[cloud-armor-quota] $m" }

gcloud config set project $ProjectId | Out-Null
$prevEa = $ErrorActionPreference
$ErrorActionPreference = "Continue"
gcloud services enable compute.googleapis.com serviceusage.googleapis.com --project=$ProjectId 2>&1 | Out-Null
$ErrorActionPreference = $prevEa

$quotaUrl = "https://console.cloud.google.com/iam-admin/quotas?project=$ProjectId&pageState=(%22allQuotasTable%22:(%22f%22:%22%255B%255D%22,%22s%22:%5B(%22i%22:%22displayName%22,%22s%22:%220%22),(%22i%22:%22currentPercent%22,%22s%22:%221%22),(%22i%22:%22sevenDayPeakPercent%22,%22s%22:%221%22),(%22i%22:%22currentUsage%22,%22s%22:%221%22),(%22i%22:%22sevenDayPeakUsage%22,%22s%22:%221%22),(%22i%22:%22effectiveLimit%22,%22s%22:%221%22)%5D))&filter=metric:compute.googleapis.com%2Fsecurity_policies"

Log "Project: $ProjectId"
Log "Metric: compute.googleapis.com/security_policies (Cloud Armor policies per project)"
Log ""

$prev = $ErrorActionPreference
$ErrorActionPreference = "SilentlyContinue"
$quotaJson = gcloud services quotas list `
    --service=compute.googleapis.com `
    --consumer="projects/$ProjectId" `
    --filter="metric:compute.googleapis.com/security_policies" `
    --format=json 2>$null
$ErrorActionPreference = $prev

if ($quotaJson) {
    $rows = $quotaJson | ConvertFrom-Json
    foreach ($row in $rows) {
        $limit = $row.quotaBuckets[0].effectiveLimit
        $usage = $row.quotaBuckets[0].usage
        Log "Current limit: $limit  usage: $usage"
    }
} else {
    Log "Could not list quota via gcloud (CLI may need upgrade). Check console manually."
}

Log ""
Log "Request increase to at least $DesiredPolicies policies:"
Log "  1. Open: $quotaUrl"
Log "  2. Select 'Security policies per project' -> EDIT QUOTAS"
Log "  3. New limit: $DesiredPolicies (testnet edge WAF + staging)"
Log "  4. Justification: Public HTTPS LB for api.testnet.cregnet.dev; rate-limit + OWASP preview (setup-cloud-armor.ps1)"
Log ""
Log "After approval (usually 1-3 business days):"
Log "  .\testnet\gcp\setup-cloud-armor.ps1 -Confirm"
Log "  .\testnet\gcp\setup-gcp-public-lb.ps1 -Confirm   # if backend not yet attached"

if ($OpenConsole) {
    Start-Process $quotaUrl
}

exit 0
