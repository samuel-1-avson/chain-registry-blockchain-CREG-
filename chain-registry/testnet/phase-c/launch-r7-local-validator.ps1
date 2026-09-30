param(
    [Parameter(Mandatory = $true)] [string]$Image,
    [string]$PrivateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1',
    [string]$ScannerBundlePath = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9',
    [string]$DataDir = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
)

$ErrorActionPreference = 'Stop'
$expectedImageId = 'sha256:cf9fda37887e0d8c6125280c07bdb36737e495e75d070b80abcc722187180d33'
$expectedConfigId = 'sha256:e25b3470e0372b22c5bb98a91cb40a9449051d51cddd366ff8f957cd983f447b'
$expectedGenesis = '0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
$expectedProfile = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
$expectedEpoch = 'osv-2026-09-26-phase-c-r8'
$container = 'creg-phase-c-r7-v1'
$envPath = Join-Path $PrivateRoot 'node.env'
$identityPath = Join-Path $PrivateRoot 'identity.key'

$actualImageId = (& docker image inspect $Image --format '{{.Id}}')
if ($LASTEXITCODE -ne 0 -or $actualImageId -notin @($expectedImageId, $expectedConfigId)) {
    throw 'Local candidate image does not match the reviewed r9 config digest.'
}
if (-not (Test-Path -LiteralPath $envPath) -or -not (Test-Path -LiteralPath $identityPath)) {
    throw 'Private r7 environment or P2P identity is missing.'
}
if (Test-Path -LiteralPath $DataDir) {
    throw 'Fresh r7 data directory already exists; preserve it and inspect before continuing.'
}
$existing = (& docker container inspect $container --format '{{.Id}}' 2>$null)
if ($LASTEXITCODE -eq 0) { throw 'r7 validator container already exists; preserve it and inspect before continuing.' }

$requiredLines = @(
    'CREG_NODE_ID=validator-1',
    'CREG_NODE_ROLE=validator',
    'CREG_IS_VALIDATOR=true',
    'CREG_CHAIN_ID=creg-phase-c-testnet-2026-09-26-r7',
    "CREG_GENESIS_HASH=$expectedGenesis",
    'CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json',
    'CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json.sig',
    "CREG_SCANNER_PROFILE_DIGEST=$expectedProfile",
    "CREG_OSV_SNAPSHOT_EPOCH=$expectedEpoch",
    'CREG_OSV_CONSENSUS=true',
    'CREG_OSV_BLOCK_CRITICAL=true',
    'CREG_OSV_LIVE_FALLBACK=false',
    'CREG_YARA_RULES_DIR=/scanner/rules',
    'CREG_THREAT_DB_PATH=/scanner/known_malicious_hashes.json',
    'CREG_OSV_SNAPSHOT_PATH=/scanner/osv_snapshot.json',
    'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false',
    'CREG_DEV_SANDBOX=false'
)
$envLines = Get-Content -LiteralPath $envPath
foreach ($required in $requiredLines) {
    if ($envLines -notcontains $required) { throw 'Private r7 environment does not match the signed chain or scanner profile.' }
}

$profileOutput = @(& (Join-Path $PSScriptRoot 'compute-scanner-profile.ps1') `
    -RulesDirectory (Join-Path $ScannerBundlePath 'rules') `
    -ThreatDatabasePath (Join-Path $ScannerBundlePath 'known_malicious_hashes.json') `
    -OsvSnapshotPath (Join-Path $ScannerBundlePath 'osv_snapshot.json') `
    -OsvEpoch $expectedEpoch -BlockCriticalOsv $true)
$computedProfile = ($profileOutput | Where-Object { $_ -like 'CREG_SCANNER_PROFILE_DIGEST=*' } | Select-Object -First 1) -replace '^CREG_SCANNER_PROFILE_DIGEST=', ''
if ($computedProfile -ne $expectedProfile) { throw 'Local scanner bundle does not match the pinned r7 profile.' }

$dataP2pDir = Join-Path $DataDir 'p2p'
New-Item -ItemType Directory -Path $dataP2pDir -Force | Out-Null
Copy-Item -LiteralPath $identityPath -Destination (Join-Path $dataP2pDir 'identity.key')
$dockerArgs = @(
    'run', '-d', '--name', $container, '--restart', 'unless-stopped',
    '--privileged', '--env-file', $envPath,
    '-p', '18080:8080', '-p', '9001:9000',
    '--mount', "type=bind,source=$DataDir,target=/data",
    '--mount', "type=bind,source=$ScannerBundlePath,target=/scanner,readonly",
    '-v', '/var/run/docker.sock:/var/run/docker.sock',
    $Image
)
& docker @dockerArgs | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not start isolated r7 validator; preserve its new data directory for inspection.' }
$runningImageId = (& docker inspect $container --format '{{.Image}}')
if ($LASTEXITCODE -ne 0 -or $runningImageId -notin @($expectedImageId, $expectedConfigId)) {
    throw 'Started local r7 validator image does not match the reviewed r9 config digest.'
}
Write-Output "Started isolated r7 V1 with pinned scanner assets and image $runningImageId"
