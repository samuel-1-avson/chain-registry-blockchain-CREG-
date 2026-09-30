param(
    [Parameter(Mandatory = $true)] [string]$RulesDirectory,
    [Parameter(Mandatory = $true)] [string]$ThreatDatabasePath,
    [Parameter(Mandatory = $true)] [string]$OsvSnapshotPath,
    [Parameter(Mandatory = $true)] [string]$OsvEpoch,
    [bool]$BlockCriticalOsv = $true
)

$ErrorActionPreference = 'Stop'

function Convert-BytesToHex([byte[]]$Bytes) {
    return ([BitConverter]::ToString($Bytes) -replace '-', '').ToLowerInvariant()
}

function Get-Sha256Hex([byte[]]$Bytes) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return Convert-BytesToHex $sha.ComputeHash($Bytes) } finally { $sha.Dispose() }
}

function Add-ProfileBytes(
    [Security.Cryptography.IncrementalHash]$Hasher,
    [byte[]]$Bytes
) {
    $lengthBytes = [byte[]]::new(8)
    $value = [long]$Bytes.Length
    for ($index = 7; $index -ge 0; $index--) {
        $lengthBytes[$index] = [byte]($value -band 255)
        $value = $value -shr 8
    }
    $Hasher.AppendData($lengthBytes)
    $Hasher.AppendData($Bytes)
}

$rulesPath = (Resolve-Path -LiteralPath $RulesDirectory).Path
$ruleFiles = @(Get-ChildItem -LiteralPath $rulesPath -File | Where-Object { $_.Extension -in @('.yar', '.yara') } | ForEach-Object { $_.FullName })
if ($ruleFiles.Count -eq 0) { throw 'No YARA rule source files were found.' }
[Array]::Sort($ruleFiles, [StringComparer]::Ordinal)

$yaraHash = [Security.Cryptography.IncrementalHash]::CreateHash([Security.Cryptography.HashAlgorithmName]::SHA256)
try {
    $yaraHash.AppendData([Text.Encoding]::UTF8.GetBytes("creg-yara-rule-bundle-v1`0"))
    foreach ($file in $ruleFiles) {
        $nameBytes = [Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFileName($file))
        $sourceBytes = [IO.File]::ReadAllBytes($file)
        Add-ProfileBytes $yaraHash $nameBytes
        Add-ProfileBytes $yaraHash $sourceBytes
    }
    $yaraDigest = Convert-BytesToHex $yaraHash.GetHashAndReset()
} finally {
    $yaraHash.Dispose()
}

$threatBytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $ThreatDatabasePath).Path)
$osvBytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $OsvSnapshotPath).Path)
$threatDigest = Get-Sha256Hex $threatBytes
$osvDigest = Get-Sha256Hex $osvBytes
$profile = "creg-scanner-profile-v1|version=creg-deterministic-scanner-v1.1.0|yara=$yaraDigest|threat=$threatDigest|osv_enabled=true|osv_epoch=$OsvEpoch|osv=$osvDigest|osv_block_critical=$($BlockCriticalOsv.ToString().ToLowerInvariant())|osv_live_fallback=false"
$profileBytes = [Text.Encoding]::UTF8.GetBytes($profile)
$profileDigest = Get-Sha256Hex $profileBytes
Write-Output "CREG_SCANNER_PROFILE_DIGEST=$profileDigest"
Write-Output "YARA_RULE_FILES=$($ruleFiles.Count)"
Write-Output "YARA_SHA256=$yaraDigest"
Write-Output "THREAT_DB_SHA256=$threatDigest"
Write-Output "OSV_SNAPSHOT_SHA256=$osvDigest"
