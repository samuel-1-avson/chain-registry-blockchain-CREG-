# Export Genesis Alpha pilot roster from Firestore into docs/genesis-alpha-wave1-tracker.csv.
#
# Usage:
#   .\testnet\gcp\export-genesis-alpha-wave1.ps1
#   .\testnet\gcp\export-genesis-alpha-wave1.ps1 -Limit 10 -WriteCsv
#
# Prerequisite: gcloud auth (gcloud auth login).

param(
    [string]$WaitlistProject = "gen-lang-client-0098858574",
    [string]$DatabaseId = "ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753",
    [int]$Limit = 25,
    [switch]$WriteCsv
)

$ErrorActionPreference = "Stop"

$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$workspaceRoot = Split-Path -Parent (Split-Path -Parent $testnetDir)
$trackerPath = Join-Path $workspaceRoot "docs\genesis-alpha-wave1-tracker.csv"

$ethRe = '^0x[a-fA-F0-9]{40}$'

function Log($m) { Write-Host "[genesis-export] $m" }

function Role-ToPath([string]$role) {
    switch ($role) {
        "Publisher" { return "publish" }
        "Validator Node" { return "validate" }
        "Security Audits" { return "observe" }
        default { return "observe" }
    }
}

$token = (gcloud auth print-access-token).Trim()
$uri = "https://firestore.googleapis.com/v1/projects/$WaitlistProject/databases/$DatabaseId/documents:runQuery"
$body = @{
    structuredQuery = @{
        from    = @(@{ collectionId = "registrations" })
        where   = @{
            fieldFilter = @{
                field = @{ fieldPath = "tier" }
                op    = "EQUAL"
                value = @{ stringValue = "alpha" }
            }
        }
        orderBy = @(@{
            field     = @{ fieldPath = "position" }
            direction = "ASCENDING"
        })
        limit   = $Limit
    }
} | ConvertTo-Json -Depth 10 -Compress

$raw = Invoke-RestMethod -Uri $uri -Method Post `
    -Headers @{ Authorization = "Bearer $token" } `
    -ContentType "application/json" `
    -Body $body

$seen = @{}
$rows = @()

foreach ($entry in $raw) {
    if (-not $entry.document) { continue }
    $f = $entry.document.fields
    $wallet = $f.walletAddress.stringValue.ToLower()
    if ($wallet -match $ethRe) { $wallet = $wallet }
    if ($wallet -notmatch $ethRe) {
        Log "skip invalid wallet at position $($f.position.integerValue): $wallet"
        continue
    }
    if ($seen.ContainsKey($wallet)) { continue }
    $seen[$wallet] = $true

    $rows += [PSCustomObject]@{
        wallet             = $wallet
        waitlist_position  = [int]$f.position.integerValue
        tier               = $f.tier.stringValue
        path               = Role-ToPath $f.role.stringValue
        invite_sent        = ""
        hub_signed_in      = ""
        cli_installed      = ""
        staked             = ""
        first_action_at    = ""
        blockers           = ""
        notes              = $f.role.stringValue
    }
}

if ($rows.Count -eq 0) {
    throw "No valid alpha registrations found."
}

Log "Unique alpha wallets: $($rows.Count)"
$rows | Format-Table wallet, waitlist_position, path, notes -AutoSize

if ($WriteCsv) {
    $rows | Export-Csv -Path $trackerPath -NoTypeInformation -Encoding utf8
    Log "Wrote $trackerPath"
} else {
    Log "Pass -WriteCsv to update $trackerPath"
}
