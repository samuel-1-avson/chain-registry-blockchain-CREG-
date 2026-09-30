param([switch]$Broadcast)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$secretName = 'creg-phase-c-treasury-evm'
$secretProject = 'gen-lang-client-0022105784'
$key = (& gcloud secrets versions access latest --secret=$secretName --project=$secretProject)
if ($LASTEXITCODE -ne 0) { throw 'Cannot read the exposed wallet key for rescue.' }
$key = ([string]$key).Trim()
if ($key -notmatch '^0x[0-9a-fA-F]{64}$') { throw 'Exposed wallet key has invalid format.' }
$env:CREG_EXPOSED_DEPLOYER_KEY = $key
$key = $null
try {
    Push-Location $repoRoot
    $mode = if ($Broadcast) { '--broadcast' } else { '--plan' }
    & node testnet/phase-c/retire-exposed-deployer.js $mode
    if ($LASTEXITCODE -ne 0) { throw 'Exposed-wallet rescue failed; inspect public balances and transaction status.' }
} finally {
    [Environment]::SetEnvironmentVariable('CREG_EXPOSED_DEPLOYER_KEY', $null, 'Process')
    Pop-Location
}
