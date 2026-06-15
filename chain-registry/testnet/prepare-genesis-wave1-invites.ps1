# GENESIS-001 prep - fill Wave 1 pilot invite emails from tracker CSV.
#
# Usage:
#   .\testnet\prepare-genesis-wave1-invites.ps1
#   .\testnet\prepare-genesis-wave1-invites.ps1 -TrackerPath ..\..\docs\genesis-alpha-wave1-tracker.csv

param(
    [string]$TrackerPath = "",
    [string]$OutFile = "",
    [string]$Release = "v0.1.2-testnet",
    [string]$GithubRepo = "samuel-1-avson/chain-registry-blockchain-CREG-"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$docsRoot = Split-Path -Parent $repoRoot

if (-not $TrackerPath) {
    $TrackerPath = Join-Path $docsRoot "docs\genesis-alpha-wave1-tracker.csv"
}
if (-not (Test-Path $TrackerPath)) {
    throw "Missing tracker: $TrackerPath (run export-genesis-alpha-wave1.ps1 -WriteCsv first)"
}

if (-not $OutFile) {
    $OutFile = Join-Path $docsRoot "docs\genesis-alpha-wave1-ready.md"
}

$rows = Import-Csv -Path $TrackerPath | Where-Object { $_.wallet -match '^0x[a-fA-F0-9]{40}$' }
if ($rows.Count -eq 0) {
    throw "No valid wallet rows in $TrackerPath"
}

$repoUrl = "https://github.com/$GithubRepo"
$releaseUrl = "$repoUrl/releases/tag/$Release"
$quickstartUrl = "$repoUrl/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md"
$scopeUrl = "$repoUrl/blob/main/docs/TESTNET_PHASE_SCOPE.md"

$template = @'
Hi there,

You're in the CREG Genesis Alpha cohort (waitlist position {{POSITION}}).

CREG is a supply-chain registry for chain artifacts - signed publishes, IPFS pins, and validator verification on Sepolia. This is public alpha, not mainnet.

Your path: {{PATH}}

1) Join hub (quests + status)
   https://testnet.cregnet.dev
   Connect wallet -> Sign in with Ethereum (Sepolia)
   Use Switch wallet if you need another account; WalletConnect works on mobile.

2) Install CLI ({{RELEASE}})
   export CREG_GITHUB_REPO={{GITHUB_REPO}}
   ./scripts/install-creg.sh --version {{RELEASE}}
   Or download from:
   {{RELEASE_URL}}

{{STEP3}}

4) Faucet (Sepolia ETH + tCREG)
   https://faucet.testnet.cregnet.dev?address={{WALLET}}

5) Public API
   export CREG_NODE_URL=https://api.testnet.cregnet.dev

Read first (limits and what "verified" means):
{{SCOPE_URL}}

Reply in this thread if blocked more than 24 hours. Do not share private keys or validator keys in email.

- CREG testnet ops
'@

function Get-Step3([string]$Path) {
    switch ($Path) {
        "publish" {
            return "3) Quickstart (stake, publish)`n   $quickstartUrl"
        }
        "validate" {
            return @"
3) Quickstart (stake, validate)
   $quickstartUrl

   Validator slots are operator-provisioned in Wave 1 - reply with your intended stake address after hub SIWE and we will confirm enrollment.
"@
        }
        default {
            return @"
3) Explorer (read-only)
   https://explorer.testnet.cregnet.dev

   Optional hub: https://testnet.cregnet.dev (SIWE sign-in, quests)
"@
        }
    }
}

$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine("# Genesis Alpha Wave 1 - ready-to-send pilot invites")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("**Generated:** $(Get-Date -Format 'yyyy-MM-dd HH:mm')")
[void]$sb.AppendLine("**Source:** ``$TrackerPath``")
[void]$sb.AppendLine("**Prerequisite:** HOSTING-301 verify pass on ``testnet.cregnet.dev``")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("Firestore has wallet + role only (no email). Send from your ops mailbox.")
[void]$sb.AppendLine("After send: set ``invite_sent`` in [genesis-alpha-wave1-tracker.csv](./genesis-alpha-wave1-tracker.csv).")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("---")
[void]$sb.AppendLine("")

$i = 0
foreach ($row in $rows) {
    $i++
    $wallet = $row.wallet
    $pos = $row.waitlist_position
    $path = if ($row.path) { $row.path } else { "observe" }
    $step3 = Get-Step3 $path

    $body = $template `
        -replace '\{\{POSITION\}\}', $pos `
        -replace '\{\{PATH\}\}', $path `
        -replace '\{\{RELEASE\}\}', $Release `
        -replace '\{\{GITHUB_REPO\}\}', $GithubRepo `
        -replace '\{\{RELEASE_URL\}\}', $releaseUrl `
        -replace '\{\{STEP3\}\}', $step3.TrimEnd() `
        -replace '\{\{WALLET\}\}', $wallet `
        -replace '\{\{SCOPE_URL\}\}', $scopeUrl

    [void]$sb.AppendLine("## Pilot $i - position $pos - $path")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("**Wallet:** ``$wallet``")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("**Subject:** CREG Genesis Alpha - your Wave 1 testnet invite")
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine('```')
    [void]$sb.AppendLine($body.TrimEnd())
    [void]$sb.AppendLine('```')
    [void]$sb.AppendLine("")
    [void]$sb.AppendLine("---")
    [void]$sb.AppendLine("")
}

Set-Content -Path $OutFile -Value $sb.ToString() -Encoding utf8
Write-Host "[genesis-001] Wrote $OutFile ($i pilots)" -ForegroundColor Green
Write-Host "[genesis-001] Send emails, then update invite_sent column in tracker CSV" -ForegroundColor Cyan
