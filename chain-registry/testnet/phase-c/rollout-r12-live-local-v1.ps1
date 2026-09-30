param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-rollout/v1-local-rollout.json')
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$bundle = Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-bundle'
$container = 'creg-phase-c-r7-v1'
$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$envPath = Join-Path $privateRoot 'node.env'
$oldScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r11-osv-kleur'
$newScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r12-live-escape'
$api = 'http://127.0.0.1:18080'
$oldEpoch = 'osv-2026-09-27-phase-c-kleur-r11'
$oldProfile = 'f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
$newEpoch = 'osv-2026-09-27-phase-c-escape-string-regexp-r12-live'
$newProfile = 'e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2'
$chainId = 'creg-phase-c-testnet-2026-09-26-r7'
$genesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$tipHash = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'
$expectedImage = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$preserved = "$container-r11-before-r12-live-$runId"
$envBackup = "$envPath.r11-preserved-$runId"
$identityPath = Join-Path $data 'p2p\identity.key'
$journalPath = Join-Path $data 'pbft-safety.rocksdb'

function Get-Metric([string]$Text, [string]$Name) {
    $pattern = '^' + [regex]::Escape($Name) + '(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$'
    $line = $Text -split '\r?\n' | Where-Object { $_ -match $pattern } | Select-Object -First 1
    if (-not $line) { return $null }
    return [double]([regex]::Match($line, $pattern).Groups[1].Value)
}

$expectedHashes = @{
    'osv_snapshot.json' = '981e3388bc2b79b71daebb63f0c467bd11e06ff481766e9b9f12615de7d77928'
    'manifest.json' = 'ad5720ece2376ebf3bace85456988a74463a599f3081b49c924da93d84cc0efa'
    'known_malicious_hashes.json' = '5b3aeb024b6e6bb0b82257aae81cb680b8d2aaff48b7753056493559e43861b1'
    'rules\supply_chain.yar' = '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
}
foreach ($relative in $expectedHashes.Keys) {
    $path = Join-Path $bundle $relative
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHashes[$relative]) {
        throw "R12 bundle hash mismatch: $relative"
    }
}
$manifest = Get-Content -LiteralPath (Join-Path $bundle 'manifest.json') -Raw | ConvertFrom-Json
$snapshot = Get-Content -LiteralPath (Join-Path $bundle 'osv_snapshot.json') -Raw | ConvertFrom-Json
if ($manifest.scanner_profile_digest -ne $newProfile -or $manifest.osv_epoch -ne $newEpoch -or
    $snapshot.epoch -ne $newEpoch -or $null -eq $snapshot.entries.'npm:escape-string-regexp@5.0.0') {
    throw 'R12 manifest, profile, epoch, or fresh-package coverage differs.'
}

if ((Test-Path -LiteralPath $newScanner) -or (Test-Path -LiteralPath $envBackup) -or
    (Test-Path -LiteralPath $EvidencePath)) { throw 'A V1 R12 scanner target or evidence path already exists.' }
foreach ($path in @($envPath, $identityPath, $journalPath, $oldScanner)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "V1 required persistent state is missing: $path" }
}

$envText = [IO.File]::ReadAllText($envPath)
foreach ($pair in @(
    @{ Key = 'CREG_NODE_ID'; Value = 'validator-1' },
    @{ Key = 'CREG_NODE_ROLE'; Value = 'validator' },
    @{ Key = 'CREG_CHAIN_SPEC_URL'; Value = 'https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json' },
    @{ Key = 'CREG_OSV_SNAPSHOT_EPOCH'; Value = $oldEpoch },
    @{ Key = 'CREG_SCANNER_PROFILE_DIGEST'; Value = $oldProfile },
    @{ Key = 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM'; Value = 'false' }
)) {
    $pattern = '(?m)^' + [regex]::Escape("$($pair.Key)=$($pair.Value)") + '(?=\r?$)'
    if ([regex]::Matches($envText, $pattern).Count -ne 1) { throw "$($pair.Key) does not have exactly one expected live value." }
}
if ($envText -match '(?m)^CREG_PHASE_C_COMMIT_CAPTURE_') { throw 'Commit-capture mode must be unset.' }

$oldJson = & docker inspect $container 2>$null
if ($LASTEXITCODE -ne 0) { throw 'V1 container is not inspectable.' }
$old = @($oldJson | ConvertFrom-Json)[0]
if ($old.State.Status -ne 'running' -or $old.Image -ne $expectedImage) { throw 'V1 is not running the exact reviewed R22 image.' }
if ((& docker container inspect $preserved --format '{{.Id}}' 2>$null | Out-String).Trim()) { throw 'V1 preservation container already exists.' }
$mounts = @($old.Mounts)
$dataMount = @($mounts | Where-Object { $_.Destination -eq '/data' })[0]
$scannerMount = @($mounts | Where-Object { $_.Destination -eq '/scanner' })[0]
if ($null -eq $dataMount -or $dataMount.Source -ne $data -or $dataMount.RW -ne $true -or
    $null -eq $scannerMount -or $scannerMount.Source -ne $oldScanner -or $scannerMount.RW -ne $false) {
    throw 'V1 data or scanner mount differs from the reviewed configuration.'
}

$statsBefore = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 10
$healthBefore = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 10
$metricsBefore = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 10).Content
$pbftBefore = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 10
$unresolvedBefore = @($pbftBefore.rounds | Where-Object {
    $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
})
if ($statsBefore.chain_id -ne $chainId -or -not $statsBefore.spec_signature_verified -or
    $statsBefore.genesis_hash -ne $genesis -or $statsBefore.tip_height -ne 14 -or
    $statsBefore.finalized_height -ne 14 -or $statsBefore.tip_hash -ne $tipHash -or
    $statsBefore.validator_count -ne 4 -or $statsBefore.active_validators -ne 4 -or
    $statsBefore.peer_count -lt 1 -or -not $healthBefore.is_validator -or $healthBefore.node_id -ne 'validator-1') {
    throw 'V1 signed R7 chain, validator roster, peer, or identity preflight failed.'
}
if ($metricsBefore -notmatch ('profile_digest="' + $oldProfile + '"') -or
    (Get-Metric $metricsBefore 'creg_scanner_osv_lookups') -ne 5 -or
    (Get-Metric $metricsBefore 'creg_scanner_profile_ready') -ne 1 -or
    (Get-Metric $metricsBefore 'creg_scanner_threat_intel_entries') -ne 100) {
    throw 'V1 R11 scanner profile is not the expected ready baseline.'
}
if ($unresolvedBefore.Count -gt 0) { throw 'V1 has an unresolved PBFT round.' }
foreach ($name in @('creg_pending_block_txs', 'creg_durable_finalized_work_depth',
    'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending')) {
    if ((Get-Metric $metricsBefore $name) -ne 0) { throw "V1 has pending work: $name" }
}
$deadLettersBefore = Get-Metric $metricsBefore 'creg_durable_finalized_work_dead_letter_depth'
if ($null -eq $deadLettersBefore) { throw 'V1 dead-letter baseline is unavailable.' }

$identityBefore = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
New-Item -ItemType Directory -Path (Split-Path $newScanner -Parent) -Force | Out-Null
New-Item -ItemType Directory -Path $newScanner | Out-Null
Copy-Item -Path (Join-Path $bundle '*') -Destination $newScanner -Recurse
Copy-Item -LiteralPath $envPath -Destination $envBackup
$updatedEnv = [regex]::Replace($envText, '(?m)^CREG_OSV_SNAPSHOT_EPOCH=.*(?=\r?$)', "CREG_OSV_SNAPSHOT_EPOCH=$newEpoch")
$updatedEnv = [regex]::Replace($updatedEnv, '(?m)^CREG_SCANNER_PROFILE_DIGEST=.*(?=\r?$)', "CREG_SCANNER_PROFILE_DIGEST=$newProfile")
$temporaryEnv = "$envPath.r12-$runId.tmp"
[IO.File]::WriteAllText($temporaryEnv, $updatedEnv, [Text.UTF8Encoding]::new($false))
[IO.File]::Move($temporaryEnv, $envPath, $true)
$envHashAfterUpdate = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()

& docker stop --time 30 $container | Out-Null
if ($LASTEXITCODE -ne 0) {
    Copy-Item -LiteralPath $envBackup -Destination $envPath -Force
    $state = (& docker inspect $container --format '{{.State.Status}}' 2>$null | Out-String).Trim()
    if ($state -and $state -ne 'running') { & docker start $container | Out-Null }
    throw 'V1 did not stop cleanly; its prior environment was restored.'
}
& docker rename $container $preserved | Out-Null
if ($LASTEXITCODE -ne 0) {
    Copy-Item -LiteralPath $envBackup -Destination $envPath -Force
    & docker start $container | Out-Null
    throw 'Could not preserve the old V1 container; its prior environment was restored and the original node restarted.'
}
$runArgs = @(
    'run', '-d', '--name', $container, '--restart', 'unless-stopped', '--privileged',
    '--env-file', $envPath, '-p', '18080:8080', '-p', '9001:9000',
    '--mount', "type=bind,source=$data,target=/data",
    '--mount', "type=bind,source=$newScanner,target=/scanner,readonly",
    '-v', '/var/run/docker.sock:/var/run/docker.sock', $expectedImage
)
& docker @runArgs | Out-Null
if ($LASTEXITCODE -ne 0) {
    Copy-Item -LiteralPath $envBackup -Destination $envPath -Force
    & docker rename $preserved $container | Out-Null
    & docker start $container | Out-Null
    throw 'R12 V1 container did not start; the preserved R11 node was restored.'
}

$deadline = [DateTime]::UtcNow.AddMinutes(6)
$health = $null
$stats = $null
$containerHealthy = $false
while ([DateTime]::UtcNow -lt $deadline) {
    try {
        $health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 5
        $stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
        $containerJson = & docker inspect $container 2>$null
        if ($LASTEXITCODE -eq 0) {
            $containerHealthy = (@($containerJson | ConvertFrom-Json)[0].State.Health.Status -eq 'healthy')
        }
        if ($health.status -eq 'ok' -and $stats.peer_count -gt 0 -and $containerHealthy) { break }
    } catch { }
    Start-Sleep -Seconds 3
}
if ($null -eq $health -or $health.status -ne 'ok' -or $null -eq $stats -or $stats.peer_count -lt 1 -or -not $containerHealthy) {
    throw 'V1 R12 container did not regain Docker health, API health, and peer connections within six minutes.'
}
$metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 10).Content
$pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 10
$unresolved = @($pbft.rounds | Where-Object {
    $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
})
$new = @((& docker inspect $container | ConvertFrom-Json))[0]
$oldAfter = @((& docker inspect $preserved | ConvertFrom-Json))[0]
$currentEnvHash = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()
$identityAfter = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
$newMetrics = [ordered]@{
    profile_ready = (Get-Metric $metrics 'creg_scanner_profile_ready') -eq 1
    osv_ready = (Get-Metric $metrics 'creg_scanner_osv_ready') -eq 1
    osv_lookups = (Get-Metric $metrics 'creg_scanner_osv_lookups') -eq 6
    yara_ready = (Get-Metric $metrics 'creg_scanner_yara_ready') -eq 1
    threat_feed_ready = (Get-Metric $metrics 'creg_scanner_threat_intel_ready') -eq 1
    threat_feed_entries = (Get-Metric $metrics 'creg_scanner_threat_intel_entries') -eq 100
}
$checks = [ordered]@{
    same_image = $new.Image -eq $expectedImage
    new_container_running = $new.State.Running -and $new.State.Health.Status -eq 'healthy'
    previous_container_preserved = $oldAfter.State.Status -eq 'exited'
    data_mount_unchanged = @($new.Mounts | Where-Object { $_.Destination -eq '/data' -and $_.Source -eq $data -and $_.RW }).Count -eq 1
    new_scanner_mount_read_only = @($new.Mounts | Where-Object { $_.Destination -eq '/scanner' -and $_.Source -eq $newScanner -and -not $_.RW }).Count -eq 1
    signed_r7_tip_unchanged = $stats.chain_id -eq $chainId -and $stats.genesis_hash -eq $genesis -and $stats.spec_signature_verified -and $stats.tip_height -eq 14 -and $stats.finalized_height -eq 14 -and $stats.tip_hash -eq $tipHash
    four_validator_roster = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
    validator_identity_and_peers = $health.is_validator -and $health.node_id -eq 'validator-1' -and $stats.peer_count -gt 0
    scanner_profile = $metrics -match ('profile_digest="' + $newProfile + '"') -and $newMetrics.profile_ready
    scanner_inputs = $newMetrics.osv_ready -and $newMetrics.osv_lookups -and $newMetrics.yara_ready -and $newMetrics.threat_feed_ready -and $newMetrics.threat_feed_entries
    queues_clear = (Get-Metric $metrics 'creg_pending_block_txs') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_depth') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_ready_depth') -eq 0 -and (Get-Metric $metrics 'creg_forced_inclusion_pending') -eq 0
    dead_letters_unchanged = (Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth') -eq $deadLettersBefore
    pbft_clear = $unresolved.Count -eq 0
    identity_unchanged = $identityBefore -eq $identityAfter
    env_stable_after_start = $envHashAfterUpdate -eq $currentEnvHash
    signer_journal_preserved = Test-Path -LiteralPath $journalPath
    old_scanner_preserved = Test-Path -LiteralPath $oldScanner
}
$evidence = [ordered]@{
    schema = 'creg-real-r12-live-v1-scanner-rollout-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    slot = 'v1'
    image_id = $new.Image
    node_binary_sha256 = ((& docker exec $container sha256sum /app/creg-node | Out-String).Trim() -split '\s+')[0]
    old_profile = $oldProfile
    scanner_profile_digest = $newProfile
    scanner_epoch = $newEpoch
    osv_snapshot_sha256 = $expectedHashes['osv_snapshot.json']
    manifest_sha256 = $expectedHashes['manifest.json']
    signed_tip = [ordered]@{ height = $stats.tip_height; hash = $stats.tip_hash; genesis = $stats.genesis_hash; chain_id = $stats.chain_id }
    dead_letter_depth = Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth'
    preserved_container = $preserved
    environment_backup = $envBackup
    stats = $stats
    health = $health
    scanner_metrics = $newMetrics
    checks = $checks
    passed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    public_cutover = $false
}
New-Item -ItemType Directory -Path (Split-Path $EvidencePath -Parent) -Force | Out-Null
$evidence | ConvertTo-Json -Depth 24 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $evidence.passed) { throw "V1 R12 postflight failed; the R11 container and state are preserved. Evidence: $EvidencePath" }
Write-Output "V1 R12 scanner profile passed; signed h14 and all persistent identity/signing state remain intact. Evidence: $EvidencePath"
