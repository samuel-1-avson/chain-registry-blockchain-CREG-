param(
    [string]$PrivateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate\publisher',
    [string]$PackageTarball = 'creg-phase-c-heal-20260925-0.1.0.tgz',
    [string]$SignedOutput = 'signed-publish.json',
    [string]$SpecPath = (Join-Path $PSScriptRoot 'chain-spec.json')
)

$ErrorActionPreference = 'Stop'
$spec = Get-Content -LiteralPath $SpecPath -Raw | ConvertFrom-Json
if ($spec.chain_id -notmatch '^creg-phase-c-testnet-' -or
    $spec.genesis_hash -notmatch '^0x[0-9a-fA-F]{64}$' -or
    $spec.contracts.staking -notmatch '^0x[0-9a-fA-F]{40}$') {
    throw 'Invalid Phase C spec for publisher authorization.'
}
$project = 'gen-lang-client-0022105784'
$key = & gcloud secrets versions access latest --secret=creg-fresh-testnet-publisher-evm --project=$project
if ($LASTEXITCODE -ne 0) { throw 'Cannot read fresh publisher EVM signing key.' }
$key = ([string]$key).Trim()
if ($key -notmatch '^0x[0-9a-fA-F]{64}$') { throw 'Invalid fresh publisher EVM signing key format.' }

try {
    $env:CREG_PUBLISHER_EVM_KEY = $key
    $env:CREG_CHAIN_ID = [string]$spec.chain_id
    $env:CREG_GENESIS_HASH = [string]$spec.genesis_hash
    $env:CREG_STAKING_ADDR = [string]$spec.contracts.staking
    $tarball = Join-Path $PrivateRoot $PackageTarball
    $manifest = Join-Path $PrivateRoot 'creg-manifest.json'
    $publisherKey = Join-Path $PrivateRoot 'publisher-ed25519.key'
    $signed = Join-Path $PrivateRoot $SignedOutput
    & (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'target/release/creg.exe') publish $tarball `
        --manifest $manifest --key-file $publisherKey `
        --publisher-address 0xa40D79527314efD4964D49c4a0673C87b612348B `
        --ipfs-url https://ipfs.testnet.cregnet.dev --offline $signed
    if ($LASTEXITCODE -ne 0) { throw 'Signing or IPFS pinning failed.' }
} finally {
    $key = $null
    [Environment]::SetEnvironmentVariable('CREG_PUBLISHER_EVM_KEY', $null, 'Process')
    [Environment]::SetEnvironmentVariable('CREG_CHAIN_ID', $null, 'Process')
    [Environment]::SetEnvironmentVariable('CREG_GENESIS_HASH', $null, 'Process')
    [Environment]::SetEnvironmentVariable('CREG_STAKING_ADDR', $null, 'Process')
}
