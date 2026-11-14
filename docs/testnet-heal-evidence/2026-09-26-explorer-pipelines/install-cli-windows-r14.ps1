$ErrorActionPreference = 'Stop'

$source = (Resolve-Path 'chain-registry/target/release/creg.exe').Path
$expectedSourceHash = 'bdabd1698821af9ff9ad1fe53678a7e82a8c10041acbdc82aa63c359d81f344c'
$actualSourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLower()
if ($actualSourceHash -ne $expectedSourceHash) { throw 'The reviewed CLI release binary changed after its live endpoint verification.' }

$targets = @(
    @{ Path = 'C:/Users/samue/.cargo/bin/creg.exe'; ExpectedOldHash = 'c171e5a2b46dd3232087e20657edde3c7e729cac01408ee5e0ff034e5a8272df'; PreserveShellWrapper = $false },
    @{ Path = 'C:/Users/samue/.creg/bin/creg.exe'; ExpectedOldHash = 'b150c7d14189df50132bbbdcd542ae7a593c1ba07b2a25c4dfdb24c388b194d3'; PreserveShellWrapper = $true }
)

$installed = @()
foreach ($target in $targets) {
    $path = $target.Path
    $backup = "$path.pre-explorer-r14-20260926.bak"
    $staging = "$path.r14.tmp"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "CLI target is missing: $path" }
    if (Test-Path -LiteralPath $backup) { throw "CLI backup already exists; inspect before replacing: $backup" }
    if (Test-Path -LiteralPath $staging) { throw "CLI staging file already exists; inspect before replacing: $staging" }

    $oldHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
    if ($oldHash -ne $target.ExpectedOldHash) { throw "CLI target changed since review: $path" }

    Copy-Item -LiteralPath $path -Destination $backup
    if ($target.PreserveShellWrapper) {
        $shellCopy = Join-Path (Split-Path -Parent $path) 'creg-container-wrapper.sh'
        if (Test-Path -LiteralPath $shellCopy) { throw "Shell wrapper copy already exists: $shellCopy" }
        Copy-Item -LiteralPath $path -Destination $shellCopy
    }

    Copy-Item -LiteralPath $source -Destination $staging
    $stagingHash = (Get-FileHash -LiteralPath $staging -Algorithm SHA256).Hash.ToLower()
    if ($stagingHash -ne $expectedSourceHash) { throw "Staged CLI hash mismatch: $path" }
    Move-Item -LiteralPath $staging -Destination $path -Force

    $newHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
    if ($newHash -ne $expectedSourceHash) { throw "Installed CLI hash mismatch: $path" }
    $installed += [pscustomobject]@{
        path = $path
        old_sha256 = $oldHash
        new_sha256 = $newHash
        backup = $backup
        shell_wrapper = if ($target.PreserveShellWrapper) { $shellCopy } else { $null }
    }
}

$report = [pscustomobject]@{
    installed_at = (Get-Date).ToUniversalTime().ToString('o')
    source = $source
    source_sha256 = $expectedSourceHash
    installed = $installed
}
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath 'docs/testnet-heal-evidence/2026-09-26-explorer-pipelines/cli-install-r14.json' -Encoding utf8
$report | ConvertTo-Json -Depth 5
