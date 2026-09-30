param([switch]$Broadcast)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$project = 'gen-lang-client-0022105784'
$secrets = [ordered]@{
    CREG_PHASE_C_TREASURY_KEY = 'creg-phase-c-treasury-v2-evm'
    CREG_PHASE_C_PUBLISHER_KEY = 'creg-fresh-testnet-publisher-evm'
    CREG_PHASE_C_GOV_1_KEY = 'creg-fresh-testnet-governance-evm'
    CREG_PHASE_C_GOV_2_KEY = 'creg-phase-c-governance-signer-2-evm'
}

try {
    foreach ($entry in $secrets.GetEnumerator()) {
        $value = (& gcloud secrets versions access latest --secret=$($entry.Value) --project=$project)
        if ($LASTEXITCODE -ne 0) { throw "Cannot read Secret Manager entry $($entry.Value)." }
        $value = ([string]$value).Trim()
        if ($value -notmatch '^0x[0-9a-fA-F]{64}$') { throw "Invalid key format for $($entry.Value)." }
        [Environment]::SetEnvironmentVariable($entry.Key, $value, 'Process')
        $value = $null
    }
    Push-Location $repoRoot
    $mode = if ($Broadcast) { '--broadcast' } else { '--plan' }
    & node testnet/phase-c/configure-fresh-services.js $mode
    if ($LASTEXITCODE -ne 0) { throw 'Phase C service bootstrap stopped; inspect public transaction hashes and on-chain state.' }
} finally {
    foreach ($entry in $secrets.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable($entry.Key, $null, 'Process')
    }
    Pop-Location
}
