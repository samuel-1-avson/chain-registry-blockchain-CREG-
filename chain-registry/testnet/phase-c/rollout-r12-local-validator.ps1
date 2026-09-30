param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-queue-recovery-r12/restart-after-r12-v4/v1-r12-rollout.json')
)

$ErrorActionPreference = 'Stop'
$container = 'creg-phase-c-r7-v1'
$preserved = 'creg-phase-c-r7-v1-r13-preserved'
$image = 'creg-node:phase-c-secure-r12-bounded-l1-retry'
$expectedConfig = 'sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd'
$expectedLocalIndex = 'sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736'
$expectedImages = @($expectedConfig, $expectedLocalIndex)
$expectedOldImage = 'sha256:807321dc45223f050e78bb997d1cb3d159d493502cd7d0a22bf0746a6e8a33a9'
$expectedGenesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$expectedTip = '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$expectedProfile = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$scanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$envPath = Join-Path $privateRoot 'node.env'
$identityPath = Join-Path $data 'p2p\identity.key'
$api = 'http://127.0.0.1:18080'

$restartPath = Join-Path (Split-Path -Parent $EvidencePath) 'fleet-committed-restart.json'
if (-not (Test-Path -LiteralPath $restartPath)) { throw 'All-eight committed-state restart evidence is missing.' }
$restartEvidence = Get-Content -LiteralPath $restartPath -Raw | ConvertFrom-Json
if (-not $restartEvidence.all_eight_passed) { throw 'All eight nodes must pass committed-state restart checks before image rollout.' }
if (-not (Test-Path -LiteralPath $envPath) -or -not (Test-Path -LiteralPath $identityPath) -or
    -not (Test-Path -LiteralPath $data) -or -not (Test-Path -LiteralPath $scanner) -or
    -not (Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb'))) {
    throw 'Existing V1 environment, persistent data, identity, signing journal or scanner bundle is missing.'
}

$actualOldImage = (& docker inspect $container --format '{{.Image}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $actualOldImage -ne $expectedOldImage) { throw 'V1 is not the recorded R13 node image.' }
$state = (& docker inspect $container --format '{{.State.Status}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $state -ne 'running') { throw 'V1 is not running; inspect it before rollout.' }
$existingBackup = (& docker inspect $preserved --format '{{.Id}}' 2>$null | Out-String).Trim()
if ($LASTEXITCODE -eq 0) { throw 'The R13 preservation container already exists; inspect it before rollout.' }

foreach ($asset in @(
    @{ Path = (Join-Path $scanner 'known_malicious_hashes.json'); Hash = '5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef' },
    @{ Path = (Join-Path $scanner 'osv_snapshot.json'); Hash = '39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade' },
    @{ Path = (Join-Path $scanner 'rules\supply_chain.yar'); Hash = '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e' }
)) {
    if ((Get-FileHash -LiteralPath $asset.Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset.Hash) {
        throw 'The pinned V1 scanner asset bundle does not match the accepted profile.'
    }
}

$beforeStats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
if ($beforeStats.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or
    $beforeStats.genesis_hash -ne $expectedGenesis -or $beforeStats.tip_height -ne 4 -or
    $beforeStats.tip_hash -ne $expectedTip) { throw 'Unexpected V1 chain identity or h4 tip before rollout.' }
$beforeRounds = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
if (@($beforeRounds.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') }).Count -gt 0) {
    throw 'V1 has an active PBFT round; wait for it to settle before rollout.'
}

$identityBefore = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash
& docker stop --time 20 $container | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not stop V1 cleanly; prior container remains named and inspectable.' }
& docker rename $container $preserved
if ($LASTEXITCODE -ne 0) { & docker start $container | Out-Null; throw 'Could not preserve the prior V1 container.' }
$dockerArgs = @(
    'run', '-d', '--name', $container, '--restart', 'unless-stopped', '--privileged',
    '--env-file', $envPath,
    '-p', '18080:8080', '-p', '9001:9000',
    '--mount', "type=bind,source=$data,target=/data",
    '--mount', "type=bind,source=$scanner,target=/scanner,readonly",
    '-v', '/var/run/docker.sock:/var/run/docker.sock',
    $image
)
& docker @dockerArgs | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'R12 V1 container creation failed; the stopped R13 container is preserved.' }
$actualImage = (& docker inspect $container --format '{{.Image}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $actualImage -notin $expectedImages) { throw 'Running V1 image does not match the pinned R12 config or local index digest.' }

$deadline = [DateTime]::UtcNow.AddMinutes(6)
$health = $null
while ([DateTime]::UtcNow -lt $deadline) {
    try {
        $health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 4
        if ($health.status -eq 'ok') { break }
    } catch { }
    Start-Sleep -Seconds 2
}
if ($null -eq $health -or $health.status -ne 'ok') { throw 'R12 V1 API did not become healthy within six minutes; both containers remain preserved for inspection.' }

$stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
$package = Invoke-RestMethod -Uri "$api/v1/public/packages/npm%3Ais-number%407.0.0" -TimeoutSec 8
$metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8).Content
$pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
$identityAfter = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash
$checks = [ordered]@{
    same_tip = $stats.tip_height -eq 4 -and $stats.tip_hash -eq $expectedTip
    same_chain = $stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats.genesis_hash -eq $expectedGenesis -and $stats.spec_signature_verified
    validator_roster = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
    package_state = $package.status -eq 'revoked' -and $package.block_phase -eq 'committed'
    scanner_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($expectedProfile)
    queue_clear = $metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_ready_depth 0\s*$' -and $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
    no_unfinished_pbft_round = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') }).Count -eq 0
    isolated_sandbox = $health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -and -not $health.sandbox.dev_bypass
    p2p_identity_unchanged = $identityBefore -eq $identityAfter
    signing_journal_preserved = Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
    image_pinned = $actualImage -in $expectedImages
}
$evidence = [ordered]@{
    schema = 'creg-real-r12-local-validator-rollout-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    slot = 'v1'
    image_id = $actualImage
    previous_container_preserved = $preserved
    stats = $stats
    package_state = $package | Select-Object canonical, status, block_phase, block_hash, revocation_reason
    health = $health
    checks = $checks
    passed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    public_cutover = $false
}
$evidence | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $evidence.passed) { throw 'R12 V1 live checks failed; inspect rollout evidence before continuing.' }
Write-Output "R12 V1 rollout passed; previous R13 container and all persistent data are preserved. Evidence: $EvidencePath"
