# Copy sepolia-3node.env to GCP VM via gcloud compute scp.
#
# Usage:
#   .\testnet\gcp\push-env.ps1

param(
    [string]$ProjectId = "",
    [string]$Zone = "",
    [string]$VmName = "",
    [string]$EnvFile = "",
    [switch]$TunnelThroughIap
)

$ErrorActionPreference = "Stop"
$gcpDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$testnetDir = Split-Path -Parent $gcpDir
$repoRoot = Split-Path -Parent $testnetDir
$cfg = & (Join-Path $gcpDir "_Load-HostingEnv.ps1")

if (-not $ProjectId) { $ProjectId = $cfg.GCP_PROJECT }
if (-not $Zone) { $Zone = $cfg.GCP_ZONE }
if (-not $VmName) { $VmName = $cfg.GCP_VM_NAME }
if (-not $EnvFile) { $EnvFile = Join-Path $testnetDir "sepolia-3node.env" }

if (-not (Test-Path $EnvFile)) {
    throw "Missing $EnvFile - run prepare-public-hosting.ps1 first"
}

$dupKeys = @{}
foreach ($line in Get-Content -LiteralPath $EnvFile) {
    if ($line -match '^\s*#' -or $line -match '^\s*$') { continue }
    if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=') {
        $key = $Matches[1]
        if ($dupKeys.ContainsKey($key)) {
            Write-Warning "[gcp-push] Duplicate env key '$key' in $EnvFile (line $($dupKeys[$key]) and later). Keep one value per key."
        } else {
            $dupKeys[$key] = $line
        }
    }
}

$repoSlug = ($cfg.GITHUB_REPO -split '/')[-1]
$remoteRel = "creg-hosting/$repoSlug/chain-registry/testnet"
$remoteFile = "$remoteRel/sepolia-3node.env"

$sshOpts = @("--zone=$Zone", "--project=$ProjectId", "--strict-host-key-checking=no", "--quiet")
$scpOpts = @("--zone=$Zone", "--project=$ProjectId", "--strict-host-key-checking=no", "--quiet")
if ($TunnelThroughIap) {
    $sshOpts += "--tunnel-through-iap"
    $scpOpts += "--tunnel-through-iap"
}

Write-Host "[gcp-push] Resolving remote home on $VmName ..."
$remoteHome = (gcloud compute ssh $VmName @sshOpts --command="printf '%s' `$HOME").Trim()
if (-not $remoteHome) { throw "Could not resolve remote HOME on $VmName" }

$remoteAbsDir = "$remoteHome/$remoteRel"
$remoteAbsFile = "$remoteHome/$remoteFile"

# Normalize to UTF-8 without BOM (PowerShell Set-Content -Encoding utf8 adds BOM; breaks Linux source).
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
$content = [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $EnvFile).Path)
if ($content.Length -gt 0 -and [int][char]$content[0] -eq 0xFEFF) {
    $content = $content.Substring(1)
}
# Linux `source` breaks on CRLF (`$'\r': command not found`).
$content = $content -replace "`r`n", "`n" -replace "`r", ""
$tempFile = [System.IO.Path]::GetTempFileName()
try {
    [System.IO.File]::WriteAllText($tempFile, $content, $utf8NoBom)
    Write-Host "[gcp-push] Uploading env (UTF-8 no BOM) to ${VmName}:$remoteAbsFile ..."
    gcloud compute ssh $VmName @sshOpts --command="mkdir -p '$remoteAbsDir' && chmod 700 '$remoteHome/creg-hosting'" | Out-Null
    gcloud compute scp $tempFile "${VmName}:${remoteAbsFile}" @scpOpts
    gcloud compute ssh $VmName @sshOpts --command="chmod 600 '$remoteAbsFile'" | Out-Null
} finally {
    Remove-Item -Force $tempFile -ErrorAction SilentlyContinue
}
Write-Host "[gcp-push] Done. Remote path: $remoteAbsFile"
