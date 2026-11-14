$ErrorActionPreference = 'Stop'

$source = (Resolve-Path 'chain-registry/target/release/creg.exe').Path
$expectedSourceHash = 'de45f5811ccc062efb09040f7bcf7d898c336c9d3ff893e5c5ab072e122ef831'
$actualSourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLower()
if ($actualSourceHash -ne $expectedSourceHash) { throw 'The reviewed CLI r15 binary changed after live TUI verification.' }

$targets = @(
    @{ Path = 'C:/Users/samue/.cargo/bin/creg.exe'; ExpectedOldHash = 'bdabd1698821af9ff9ad1fe53678a7e82a8c10041acbdc82aa63c359d81f344c' },
    @{ Path = 'C:/Users/samue/.creg/bin/creg.exe'; ExpectedOldHash = 'bdabd1698821af9ff9ad1fe53678a7e82a8c10041acbdc82aa63c359d81f344c' }
)

$installed = @()
foreach ($target in $targets) {
    $path = $target.Path
    $backup = "$path.pre-explorer-r15-20260926.bak"
    $staging = "$path.r15.tmp"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "CLI target is missing: $path" }

    $oldHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
    if ($oldHash -eq $expectedSourceHash) {
        if (-not (Test-Path -LiteralPath $backup)) { throw "Installed CLI is missing its recorded predecessor backup: $backup" }
        $backupHash = (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash.ToLower()
        if ($backupHash -ne $target.ExpectedOldHash) { throw "CLI backup hash mismatch: $backup" }
    } else {
        if ($oldHash -ne $target.ExpectedOldHash) { throw "CLI target changed since review: $path" }
        if (Test-Path -LiteralPath $backup) {
            $backupHash = (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash.ToLower()
            if ($backupHash -ne $target.ExpectedOldHash) { throw "CLI backup hash mismatch: $backup" }
        } else {
            Copy-Item -LiteralPath $path -Destination $backup
        }

        if (Test-Path -LiteralPath $staging) {
            $stagingHash = (Get-FileHash -LiteralPath $staging -Algorithm SHA256).Hash.ToLower()
            if ($stagingHash -ne $expectedSourceHash) { throw "Staging file has unexpected content: $staging" }
        } else {
            Copy-Item -LiteralPath $source -Destination $staging
        }
        $stagingHash = (Get-FileHash -LiteralPath $staging -Algorithm SHA256).Hash.ToLower()
        if ($stagingHash -ne $expectedSourceHash) { throw "Staged CLI hash mismatch: $path" }
        Copy-Item -LiteralPath $staging -Destination $path -Force
    }

    $newHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLower()
    if ($newHash -ne $expectedSourceHash) { throw "Installed CLI hash mismatch: $path" }
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Force }
    $installed += [pscustomobject]@{
        path = $path
        old_sha256 = $oldHash
        new_sha256 = $newHash
        backup = $backup
    }
}

$report = [pscustomobject]@{
    installed_at = (Get-Date).ToUniversalTime().ToString('o')
    source = $source
    source_sha256 = $expectedSourceHash
    installed = $installed
}
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath 'docs/testnet-heal-evidence/2026-09-26-explorer-pipelines/cli-install-r15.json' -Encoding utf8
$report | ConvertTo-Json -Depth 5
