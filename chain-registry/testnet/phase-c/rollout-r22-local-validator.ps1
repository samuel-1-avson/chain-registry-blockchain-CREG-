param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/r22-rollout/v1-rollout.json')
)

$ErrorActionPreference = 'Stop'
$image = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$expectedImageId = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$acceptedPriorImage = 'sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423'
$container = 'creg-phase-c-r7-v1'
$preserved = 'creg-phase-c-r7-v1-r21-before-r22'
$failed = 'creg-phase-c-r7-v1-r22-failed'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$envPath = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1\node.env'
$scanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r16-osv-r10'
$api = 'http://127.0.0.1:18080'
$profile = 'd09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$chainId = 'creg-phase-c-testnet-2026-09-26-r7'
$genesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$tip = 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d'

if (Test-Path -LiteralPath $EvidencePath) { throw 'Evidence path already exists; choose a new path to preserve prior evidence.' }
if ((docker image inspect $image --format '{{.Id}}').Trim() -ne $expectedImageId) { throw 'R22 digest is not locally available or does not match the reviewed image.' }
if ((docker inspect $container --format '{{.State.Running}}').Trim() -ne 'true') { throw 'V1 is not running; rollout cancelled.' }
if ((docker inspect $container --format '{{.Image}}').Trim() -ne $acceptedPriorImage) { throw 'V1 is not on the accepted R21 image; rollout cancelled.' }
foreach ($path in @($envPath, (Join-Path $data 'p2p/identity.key'), (Join-Path $data 'chain.rocksdb'), (Join-Path $data 'pbft-safety.rocksdb'), $scanner)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required preserved path missing: $path" }
}
foreach ($name in @($preserved, $failed)) {
    $existing = (docker container ls -aq --filter "name=^/$name$" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect Docker container names.' }
    if ($existing) { throw "Preservation container already exists: $name" }
}

$envText = [IO.File]::ReadAllText($envPath)
foreach ($entry in @("CREG_SCANNER_PROFILE_DIGEST=$profile", 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false', 'CREG_NODE_ID=validator-1', 'CREG_NODE_ROLE=validator')) {
    if ([regex]::Matches($envText, '(?m)^' + [regex]::Escape($entry) + '\r?$').Count -ne 1) { throw "Required non-secret V1 setting missing or duplicated: $($entry.Split('=')[0])" }
}
if ([regex]::IsMatch($envText, '(?m)^CREG_PHASE_C_COMMIT_CAPTURE_')) { throw 'The controlled crash-capture arm must remain unset during ordinary rollout.' }

$initial = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 10
if ($initial.chain_id -ne $chainId -or $initial.tip_height -ne 12 -or $initial.tip_hash -ne $tip -or -not $initial.spec_signature_verified) {
    throw 'V1 chain identity or tip differs from the accepted baseline; rollout cancelled.'
}
$identityPath = Join-Path $data 'p2p/identity.key'
$identityHash = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
$envHash = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()
$journalFilesBefore = @(Get-ChildItem -LiteralPath (Join-Path $data 'pbft-safety.rocksdb') -File -Recurse | ForEach-Object { $_.Name })
$startedAt = [DateTime]::UtcNow
$startedAtIso = $startedAt.ToString('o')
$report = [ordered]@{
    schema = 'creg-real-r22-local-validator-rollout-v1'
    slot = 'v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    rollout_started_at_utc = $null
    image_ref = $image
    image_id = $expectedImageId
    prior_image_id = $acceptedPriorImage
    preserved_container = $preserved
    chain_id = $chainId
    starting_height = $initial.tip_height
    starting_tip_hash = $initial.tip_hash
    public_cutover = $false
    passed = $false
}
$mutationStarted = $false
$rollbackOnFailure = $true

try {
    $mutationStarted = $true
    $report.rollout_started_at_utc = [DateTime]::UtcNow.ToString('o')
    docker stop --time 20 $container | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'V1 R21 container did not stop cleanly.' }
    docker rename $container $preserved | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not preserve the prior V1 container.' }

    $runArgs = @(
        'run', '-d', '--name', $container, '--restart', 'unless-stopped', '--privileged',
        '--env-file', $envPath, '-p', '18080:8080', '-p', '9001:9000',
        '--mount', "type=bind,source=$data,target=/data",
        '--mount', "type=bind,source=$scanner,target=/scanner,readonly",
        '-v', '/var/run/docker.sock:/var/run/docker.sock', $image
    )
    docker @runArgs | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'R22 V1 container creation failed.' }
    if ((docker inspect $container --format '{{.Image}}').Trim() -ne $expectedImageId) { throw 'Running V1 image differs from the reviewed R22 digest.' }
    if ((Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $identityHash) { throw 'V1 P2P identity changed.' }
    if ((Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $envHash) { throw 'V1 node environment changed.' }

    $deadline = [DateTime]::UtcNow.AddMinutes(6)
    $last = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
            $health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 8
            $pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
            $metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8).Content
            $unresolved = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') -or ($_.phase -eq 'failed' -and $_.prepare_count -ge $_.quorum) })
            $checks = [ordered]@{
                image_digest = (docker inspect $container --format '{{.Image}}').Trim() -eq $expectedImageId
                chain_identity = $stats.chain_id -eq $chainId -and $stats.spec_signature_verified
                genesis_preserved = ($stats.genesis_hash -replace '^0x', '') -eq $genesis
                h12_preserved = $stats.tip_height -eq 12 -and $stats.finalized_height -eq 12 -and $stats.tip_hash -eq $tip
                four_validator_roster = $stats.active_validators -eq 4 -and $stats.validator_count -eq 4
                connected_peers = $stats.peer_count -gt 0
                correct_role = $health.is_validator -eq $true
                scanner_profile_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($profile)
                yara_ready = $metrics -match '(?m)^creg_scanner_yara_ready 1\s*$'
                threat_feed_ready = $metrics -match '(?m)^creg_scanner_threat_intel_ready 1\s*$' -and $metrics -match '(?m)^creg_scanner_threat_intel_entries 100\s*$'
                osv_ready = $metrics -match '(?m)^creg_scanner_osv_ready 1\s*$'
                queues_clear = $metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_ready_depth 0\s*$' -and $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
                terminal_records_preserved = $metrics -match '(?m)^creg_durable_finalized_work_dead_letter_depth 7\s*$'
                pbft_clear = $unresolved.Count -eq 0
                sandbox_isolated = $health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -eq $true -and $health.sandbox.dev_bypass -eq $false
            }
            $last = [ordered]@{ checks = $checks; stats = $stats; health = $health; pbft = $pbft; unresolved_rounds = $unresolved }
            if (@($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0) { break }
        } catch { $last = [ordered]@{ last_error = $_.Exception.Message } }
        Start-Sleep -Seconds 2
    }
    if (-not $last.checks -or @($last.checks.Values | Where-Object { $_ -ne $true }).Count -gt 0) { throw 'R22 V1 readiness did not pass within six minutes.' }
    if ((docker inspect $preserved --format '{{.State.Status}}').Trim() -ne 'exited') { throw 'The prior R21 V1 container is not preserved stopped.' }
    $report.readiness_passed = $true
    $rollbackOnFailure = $false

    $logText = docker logs --since $startedAtIso $container 2>&1 | Out-String
    $markers = @($logText -split '\r?\n' | Where-Object { $_ -match 'pbft_local_commit_durable|pbft_local_commit_recovered|Recovered durable local PBFT COMMIT' })
    $journalFilesAfter = @(Get-ChildItem -LiteralPath (Join-Path $data 'pbft-safety.rocksdb') -File -Recurse | ForEach-Object { $_.Name })
    $report.after = $last
    $report.p2p_identity_preserved = $true
    $report.node_env_preserved = $true
    $report.signer_journal_present = Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
    $report.signer_journal_file_names_before = $journalFilesBefore
    $report.signer_journal_file_names_after = $journalFilesAfter
    $report.commit_recovery_markers = $markers
    $report.commit_recovery_marker_count = $markers.Count
    $report.passed = $report.signer_journal_present -and $markers.Count -gt 0
    $report.final_height = $last.stats.tip_height
    $report.final_tip_hash = $last.stats.tip_hash
    $report.completed_at_utc = [DateTime]::UtcNow.ToString('o')
    if (-not $report.passed) { throw 'V1 is healthy on R22, but a durable local COMMIT recovery marker was not observed; retain R22 and investigate before continuing rollout.' }
    New-Item -ItemType Directory -Path (Split-Path -Parent $EvidencePath) -Force | Out-Null
    $report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
} catch {
    $report.failure = $_.Exception.Message
    $report.completed_at_utc = [DateTime]::UtcNow.ToString('o')
    if ($mutationStarted -and $rollbackOnFailure) {
        try {
            $currentImage = (docker inspect $container --format '{{.Image}}' 2>$null | Out-String).Trim()
            $currentStatus = (docker inspect $container --format '{{.State.Status}}' 2>$null | Out-String).Trim()
            if ($currentImage -eq $expectedImageId) {
                if ($currentStatus -eq 'running') { docker stop --time 20 $container | Out-Null }
                if (-not (docker container ls -aq --filter "name=^/$failed$")) { docker rename $container $failed | Out-Null }
            } elseif ($currentImage -eq $acceptedPriorImage -and $currentStatus -eq 'exited') {
                docker start $container | Out-Null
            }
            $preservedStatus = (docker inspect $preserved --format '{{.State.Status}}' 2>$null | Out-String).Trim()
            if ($preservedStatus -eq 'exited' -and -not (docker container ls -aq --filter "name=^/$container$")) {
                docker rename $preserved $container | Out-Null
                docker start $container | Out-Null
                $report.rollback_started = $true
            }
            if ((docker inspect $container --format '{{.Image}}' 2>$null | Out-String).Trim() -eq $acceptedPriorImage) {
                $rollbackDeadline = [DateTime]::UtcNow.AddMinutes(2)
                while ([DateTime]::UtcNow -lt $rollbackDeadline) {
                    try {
                        $rollbackStats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 5
                        if ($rollbackStats.chain_id -eq $chainId -and $rollbackStats.tip_height -eq 12 -and $rollbackStats.tip_hash -eq $tip) {
                            $report.rollback_verified = $true
                            break
                        }
                    } catch { Start-Sleep -Seconds 2 }
                }
            }
        } catch { $report.rollback_error = $_.Exception.Message }
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $EvidencePath) -Force | Out-Null
    $report | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
    throw
}

Write-Output "R22 V1 rollout passed; image=$expectedImageId; h12=$($report.final_tip_hash); durable_recovery_markers=$($report.commit_recovery_marker_count); prior R21 container preserved as $preserved."
