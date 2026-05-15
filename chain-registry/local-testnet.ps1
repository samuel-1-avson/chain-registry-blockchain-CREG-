param(
    [switch]$SkipExplorer,
    [switch]$SkipCleanup,
    [switch]$RunSmokeTests,
    [switch]$SkipPublish
)

$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "scripts\start-local-testnet.ps1"
& $scriptPath @PSBoundParameters