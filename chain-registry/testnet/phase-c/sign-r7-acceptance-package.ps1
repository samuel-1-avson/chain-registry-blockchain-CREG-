param(
    [string]$PrivateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher',
    [string]$Image = 'creg-node:phase-c-secure-r9-review'
)

$ErrorActionPreference = 'Stop'
$expectedImageIds = @(
    'sha256:cf9fda37887e0d8c6125280c07bdb36737e495e75d070b80abcc722187180d33',
    'sha256:e25b3470e0372b22c5bb98a91cb40a9449051d51cddd366ff8f957cd983f447b'
)
$imageId = (& docker image inspect $Image --format '{{.Id}}' 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $imageId -notin $expectedImageIds) { throw 'The publisher CLI image is not the reviewed r9 artifact.' }
$specPath = Join-Path $PSScriptRoot 'r7/chain-spec.json'
$spec = Get-Content -LiteralPath $specPath -Raw | ConvertFrom-Json
if ($spec.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or
    $spec.genesis_hash -ne '0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234') {
    throw 'The acceptance publisher must use the reviewed r7 chain context.'
}
$tarballPath = Join-Path $PrivateRoot 'is-number-7.0.0.tgz'
$tarballHash = (Get-FileHash -LiteralPath $tarballPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($tarballHash -ne '7b75c1057198cf97696909a9bee176c9c5e9bcb5b03bf3ecef2f484defadd51e') {
    throw 'Acceptance tarball differs from the integrity-verified real npm artifact.'
}
foreach ($file in @('publisher-ed25519.key', 'creg-manifest.json')) {
    if (-not (Test-Path -LiteralPath (Join-Path $PrivateRoot $file))) { throw 'Missing private publisher inputs.' }
}
$signedPath = Join-Path $PrivateRoot 'signed-publish-r7.json'
if (Test-Path -LiteralPath $signedPath) { throw 'Signed acceptance request exists; preserve it before renewing authorization.' }

$variables = @('CREG_PUBLISHER_EVM_KEY', 'CREG_CHAIN_ID', 'CREG_GENESIS_HASH', 'CREG_STAKING_ADDR')
$savedEnvironment = @{}
foreach ($name in $variables) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $key = (& gcloud secrets versions access latest --secret=creg-fresh-testnet-publisher-evm --project=gen-lang-client-0022105784 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $key -notmatch '^0x[0-9a-fA-F]{64}$') { throw 'Cannot securely load the fresh publisher signing key.' }
    $env:CREG_PUBLISHER_EVM_KEY = $key
    $key = $null
    $env:CREG_CHAIN_ID = [string]$spec.chain_id
    $env:CREG_GENESIS_HASH = [string]$spec.genesis_hash
    $env:CREG_STAKING_ADDR = [string]$spec.contracts.staking
    $dockerArgs = @(
        'run', '--rm', '--entrypoint', '/app/creg',
        '--env', 'CREG_PUBLISHER_EVM_KEY', '--env', 'CREG_CHAIN_ID',
        '--env', 'CREG_GENESIS_HASH', '--env', 'CREG_STAKING_ADDR',
        '--mount', "type=bind,source=$PrivateRoot,target=/publisher",
        $Image, 'publish', '/publisher/is-number-7.0.0.tgz',
        '--manifest', '/publisher/creg-manifest.json',
        '--key-file', '/publisher/publisher-ed25519.key',
        '--publisher-address', '0xa40D79527314efD4964D49c4a0673C87b612348B',
        '--ipfs-url', 'https://ipfs.testnet.cregnet.dev',
        '--offline', '/publisher/signed-publish-r7.json'
    )
    & docker @dockerArgs
    if ($LASTEXITCODE -ne 0) { throw 'Real package pinning or signing failed.' }
} finally {
    $key = $null
    foreach ($name in $variables) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}
$request = Get-Content -LiteralPath $signedPath -Raw | ConvertFrom-Json
if ($request.id.ecosystem -ne 'npm' -or $request.id.name -ne 'is-number' -or $request.id.version -ne '7.0.0' -or
    $request.content_hash -ne $tarballHash -or $request.publisher_chain_id -ne $spec.chain_id -or
    ($request.publisher_genesis_hash -replace '^0x', '') -ne ($spec.genesis_hash -replace '^0x', '') -or
    $request.publisher_staking_contract -ne $spec.contracts.staking -or $request.publisher_authorization_version -ne 4) {
    throw 'Signed request does not bind the real npm artifact to the fresh r7 chain.'
}
'Real npm package pinned and signed with the reviewed r9 CLI; r7 authorization context verified.'
