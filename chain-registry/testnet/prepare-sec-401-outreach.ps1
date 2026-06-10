# SEC-401 prep - fill outreach email with pinned tag SHA and write send-ready file.
#
# Usage:
#   .\testnet\prepare-sec-401-outreach.ps1
#   .\testnet\prepare-sec-401-outreach.ps1 -Tag v0.1.0-testnet -ContactName "Samuel" -OutFile ..\..\docs\SEC-401-outreach-ready.md

param(
    [string]$Tag = "v0.1.0-testnet",
    [string]$GithubRepo = "",
    [string]$ContactName = "",
    [string]$OutFile = ""
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
$docsRoot = Split-Path -Parent $repoRoot
Set-Location $repoRoot

if (-not $GithubRepo) {
    $remote = git remote get-url origin 2>$null
    if ($remote -match 'github\.com[:/](.+?)(?:\.git)?$') {
        $GithubRepo = $matches[1]
    } else {
        $GithubRepo = "samuel-1-avson/chain-registry-blockchain-CREG-"
    }
}

# Prefer GitHub remote SHA (what auditors clone); fall back to local tag.
$tagSha = ""
try {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$GithubRepo/git/refs/tags/$Tag" -TimeoutSec 20
    $tagSha = $rel.object.sha
} catch { }
if (-not $tagSha) {
    try {
        $tagSha = (git rev-list -n 1 $Tag 2>$null)
    } catch { }
}
if (-not $tagSha) {
    Write-Warning "Could not resolve tag $Tag - fill SHA manually"
    $tagSha = "<TAG_SHA>"
}

$signoff = if ($ContactName) { $ContactName } else { "_______________" }
$repoUrl = "https://github.com/$GithubRepo"
$scopeUrl = "$repoUrl/blob/main/docs/SEC-401-AUDIT-SCOPE.md"

# Single-quoted here-string avoids PowerShell parsing "(4 weeks, Rust + Solidity)" as an expression.
$body = @'
Subject: RFP: Chain Registry Sepolia testnet security review (4 weeks, Rust + Solidity)

Hello,

We are scheduling a fixed-scope security review of the Chain Registry testnet stack before opening a coordinated public testnet. The engagement targets Sepolia only (no mainnet keys).

Repository: {{REPO_URL}}
Scope document: {{SCOPE_URL}}
Commit / tag for review: `{{TAG}}` (SHA: `{{TAG_SHA}}`) - pin this ref before audit starts

In scope (priority order):
1. Off-chain package admission and validator pipeline (package_admission, validator_pipeline, publish API)
2. L1 contracts on Sepolia: Staking.sol, Registry.sol, ZKVerifier.sol
3. Operational controls: chain-spec signing, validator set sync, rate limits

Out of scope: Mainnet, cross-chain (cross_chain: false), full ZK soundness proof, governance UI.

Deliverables: Rolling findings (weeks 2-3), final report with severity + PoC (week 4), optional retest window for P0/P1.

Environment we provide: Sepolia RPC access, operator runbook (chain-registry/testnet/OPERATOR.md), optional synced node on public API after HOSTING-301.

Please reply with:
- Earliest start date and team availability
- Fixed-fee or T&M estimate for the scoped 4-week timeline
- Sample smart-contract + systems audit report (redacted)

Thank you,
{{SIGNOFF}}

---
Attachments: docs/SEC-401-AUDIT-SCOPE.md (or link above)
After booking: record vendor + start date in docs/NEXT_WORK.md (SEC-401 row)
'@ -replace '\{\{REPO_URL\}\}', $repoUrl `
     -replace '\{\{SCOPE_URL\}\}', $scopeUrl `
     -replace '\{\{TAG\}\}', $Tag `
     -replace '\{\{TAG_SHA\}\}', $tagSha `
     -replace '\{\{SIGNOFF\}\}', $signoff

if (-not $OutFile) {
    $OutFile = Join-Path $docsRoot "docs\SEC-401-outreach-ready.md"
}
Set-Content -Path $OutFile -Value $body -Encoding utf8
Write-Host "[sec-401] Wrote $OutFile" -ForegroundColor Green
Write-Host "[sec-401] Tag $Tag @ $tagSha" -ForegroundColor DarkGray
Write-Host "[sec-401] Send to vendors listed in docs/SEC-401-VENDOR-OUTREACH.md" -ForegroundColor Cyan
