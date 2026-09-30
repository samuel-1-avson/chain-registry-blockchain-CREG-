param(
    [string]$EvidenceDirectory = (Join-Path (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))) 'docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness'),
    [string]$PreviousManifest = (Join-Path (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))) 'docs/testnet-heal-evidence/2026-09-26-phase-c-source-review/source-manifest.json'),
    [string]$ImageTag = 'creg-node:phase-c-secure-r9-review'
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$evidence = (Resolve-Path -LiteralPath $EvidenceDirectory).Path
$previous = Get-Content -LiteralPath $PreviousManifest -Raw | ConvertFrom-Json

Push-Location -LiteralPath $repo
try {
    $paths = @(rg --files --hidden -g '!.git/**' -g '!target/**' -g '!node_modules/**' -g '!docs/**')
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate candidate build source files.' }
    if ($paths -notcontains '.dockerignore') { $paths += '.dockerignore' }
    [Array]::Sort($paths, [StringComparer]::Ordinal)

    $files = foreach ($relativePath in $paths) {
        $fullPath = Join-Path $repo $relativePath
        if (Test-Path -LiteralPath $fullPath -PathType Leaf) {
            [ordered]@{
                path = $relativePath.Replace('\', '/')
                sha256 = (Get-FileHash -LiteralPath $fullPath -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
    }
} finally {
    Pop-Location
}

$deleted = foreach ($oldFile in $previous.files) {
    $fullPath = Join-Path $repo $oldFile.path
    if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
        [ordered]@{ path = $oldFile.path; prior_sha256 = $oldFile.sha256 }
    }
}

$manifest = [ordered]@{
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    chain_id = 'creg-phase-c-testnet-2026-09-26-r7'
    genesis_hash = '0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
    build_feature = 'phase-c-real-fleet'
    image_tag = $ImageTag
    compilation = 'pending'
    real_host_acceptance = 'pending'
    files = @($files)
    deleted_files = @($deleted)
}
$json = $manifest | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $evidence 'source-manifest.json'), $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Recorded $(@($files).Count) candidate inputs and $(@($deleted).Count) explicit source deletions."
