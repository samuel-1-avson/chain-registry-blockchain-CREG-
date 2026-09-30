param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path ("docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-live-rollout/v1-local-rollout-" + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '.json'))
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$pipelineEvidence = Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover'
$bundle = Join-Path $pipelineEvidence 'scanner-r14-ansi-regex-bundle'
$packageEvidence = Join-Path $pipelineEvidence 'next-real-package-w3-h19'
$container = 'creg-phase-c-r7-v1'
$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$envPath = Join-Path $privateRoot 'node.env'
$oldScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r13-live-has-flag'
$newScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r14-live-ansi-regex'
$api = 'http://127.0.0.1:18080'
$oldEpoch = 'osv-2026-09-28-phase-c-has-flag-r13-live'
$oldProfile = '8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e'
$newEpoch = 'osv-2026-09-28-phase-c-ansi-regex-r14-live'
$newProfile = '7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd'
$canonical = 'npm:ansi-regex@6.1.0'
$chainId = 'creg-phase-c-testnet-2026-09-26-r7'
$networkGenesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$signedGenesis = '1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
$tipHash = 'ec64471b8f9c58eb2d9b958db765a98871cb444e032143d313d65b3ab80cd76c'
$expectedImage = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$preserved = "$container-r13-before-r14-live-$runId"
$failed = "$container-r14-failed-$runId"
$envBackup = "$envPath.r13-preserved-for-r14-$runId"
$identityPath = Join-Path $data 'p2p\identity.key'
$journalPath = Join-Path $data 'pbft-safety.rocksdb'
$stagingScanner = "$newScanner.staging-$runId"

function Get-Metric([string]$Text, [string]$Name) {
    $pattern = '^' + [regex]::Escape($Name) + '(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$'
    $line = $Text -split '\r?\n' | Where-Object { $_ -match $pattern } | Select-Object -First 1
    if (-not $line) { return $null }
    return [double]([regex]::Match($line, $pattern).Groups[1].Value)
}

function Get-Container([string]$Name) {
    $json = & docker inspect $Name 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return @($json | ConvertFrom-Json)[0]
}

$expectedHashes = @{
    'osv_snapshot.json' = '5defc864740b53d9a7a2085d8ace2d972922638a836444d2b5c2abed599e543e'
    'manifest.json' = 'e07463e38d3301a3170b9eaec9fd94c6062b1a9775367a5eaf663a2dad3ae012'
    'known_malicious_hashes.json' = 'c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723'
    'rules\supply_chain.yar' = '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
}
foreach ($relative in $expectedHashes.Keys) {
    $path = Join-Path $bundle $relative
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHashes[$relative]) {
        throw "R14 bundle hash mismatch: $relative"
    }
}
$manifest = Get-Content -LiteralPath (Join-Path $bundle 'manifest.json') -Raw | ConvertFrom-Json
$snapshot = Get-Content -LiteralPath (Join-Path $bundle 'osv_snapshot.json') -Raw | ConvertFrom-Json
$packageReview = Get-Content -LiteralPath (Join-Path $packageEvidence 'review.json') -Raw | ConvertFrom-Json
$absence = Get-Content -LiteralPath (Join-Path $packageEvidence 'package-absence-pre-r14-clean.json') -Raw | ConvertFrom-Json
if ($manifest.scanner_profile_digest -ne $newProfile -or $manifest.osv_epoch -ne $newEpoch -or
    $snapshot.epoch -ne $newEpoch -or $null -eq $snapshot.entries.$canonical -or
    $packageReview.canonical -ne $canonical -or $packageReview.package_executed_or_published -ne $false -or
    -not $absence.passed -or -not $absence.all_eight_absent -or -not $absence.all_eight_same_tip -or
    $absence.expected_height -ne 18 -or $absence.expected_hash -ne $tipHash) {
    throw 'R14 package review, all-eight absence record, profile, or epoch differs from the reviewed candidate.'
}
$profileOutput = @(& (Join-Path $PSScriptRoot 'compute-scanner-profile.ps1') `
    -RulesDirectory (Join-Path $bundle 'rules') `
    -ThreatDatabasePath (Join-Path $bundle 'known_malicious_hashes.json') `
    -OsvSnapshotPath (Join-Path $bundle 'osv_snapshot.json') `
    -OsvEpoch $newEpoch -BlockCriticalOsv $true)
$computedProfile = ($profileOutput | Where-Object { $_ -like 'CREG_SCANNER_PROFILE_DIGEST=*' } | Select-Object -First 1) -replace '^CREG_SCANNER_PROFILE_DIGEST=', ''
if ($computedProfile -ne $newProfile) { throw 'Local R14 scanner inputs do not compute the reviewed profile digest.' }

foreach ($path in @($envPath, $identityPath, $journalPath, $oldScanner)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "V1 required persistent state is missing: $path" }
}
foreach ($path in @($newScanner, $stagingScanner, $envBackup, $EvidencePath)) {
    if (Test-Path -LiteralPath $path) { throw "R14 target already exists; inspect it before retrying: $path" }
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

$old = Get-Container $container
if ($null -eq $old -or $old.State.Status -ne 'running' -or $old.Image -ne $expectedImage) {
    throw 'V1 is not running the reviewed R25 image.'
}
if ($old.HostConfig.NetworkMode -ne 'bridge' -or $old.HostConfig.RestartPolicy.Name -ne 'unless-stopped' -or
    -not $old.HostConfig.Privileged -or $old.HostConfig.SecurityOpt -notcontains 'label=disable') {
    throw 'V1 runtime security, network, or restart settings differ from the reviewed container.'
}
if ((@($old.HostConfig.PortBindings.'8080/tcp')[0].HostPort) -ne '18080' -or
    (@($old.HostConfig.PortBindings.'9000/tcp')[0].HostPort) -ne '9001') {
    throw 'V1 API or P2P port bindings differ from the reviewed container.'
}
if ((Get-Container $preserved) -or (Get-Container $failed)) { throw 'A V1 preservation or failure container name already exists.' }
$mounts = @($old.Mounts)
$dataMount = @($mounts | Where-Object { $_.Destination -eq '/data' })[0]
$scannerMount = @($mounts | Where-Object { $_.Destination -eq '/scanner' })[0]
$dockerSocketMount = @($mounts | Where-Object { $_.Destination -eq '/var/run/docker.sock' })[0]
if ($null -eq $dataMount -or $dataMount.Source -ne $data -or $dataMount.RW -ne $true -or
    $null -eq $scannerMount -or $scannerMount.Source -ne $oldScanner -or $scannerMount.RW -ne $false -or
    $null -eq $dockerSocketMount -or $dockerSocketMount.RW -ne $true) {
    throw 'V1 data, scanner, or sandbox-engine mount differs from the reviewed configuration.'
}
$envMap = @{}
foreach ($item in $old.Config.Env) {
    if ($item -match '^([^=]+)=(.*)$') { $envMap[$Matches[1]] = $Matches[2] }
}
if ($envMap.CREG_NODE_ID -ne 'validator-1' -or $envMap.CREG_NODE_ROLE -ne 'validator' -or
    $envMap.CREG_SCANNER_PROFILE_DIGEST -ne $oldProfile -or $envMap.CREG_OSV_SNAPSHOT_EPOCH -ne $oldEpoch -or
    $envMap.CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM -ne 'false' -or
    @($old.NetworkSettings.Networks.PSObject.Properties.Name) -notcontains 'bridge') {
    throw 'V1 live container environment or bridge attachment differs from the reviewed R13 baseline.'
}

$statsBefore = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 10
$healthBefore = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 10
$metricsBefore = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 10).Content
$pbftBefore = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 10
$packageUri = "$api/v1/public/packages/$([uri]::EscapeDataString($canonical))"
$packageResponse = Invoke-WebRequest -Uri $packageUri -TimeoutSec 10 -SkipHttpErrorCheck
$unresolvedBefore = @($pbftBefore.rounds | Where-Object {
    $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
})
if ($statsBefore.chain_id -ne $chainId -or -not $statsBefore.spec_signature_verified -or
    $statsBefore.genesis_hash -ne $networkGenesis -or $statsBefore.tip_height -ne 18 -or
    $statsBefore.finalized_height -ne 18 -or ([string]$statsBefore.tip_hash).ToLowerInvariant().Replace('0x','') -ne $tipHash -or
    $statsBefore.validator_count -ne 4 -or $statsBefore.active_validators -ne 4 -or
    $statsBefore.peer_count -lt 1 -or -not $healthBefore.is_validator -or $healthBefore.node_id -ne 'validator-1' -or
    $packageResponse.StatusCode -ne 404) {
    throw 'V1 signed R7 h18, four-validator roster, live peer, package absence, or identity preflight failed.'
}
if ($metricsBefore -notmatch ('profile_digest="' + $oldProfile + '"') -or
    (Get-Metric $metricsBefore 'creg_scanner_osv_lookups') -ne 7 -or
    (Get-Metric $metricsBefore 'creg_scanner_profile_ready') -ne 1 -or
    (Get-Metric $metricsBefore 'creg_scanner_threat_intel_entries') -ne 100) {
    throw 'V1 R13 scanner profile is not the expected ready baseline.'
}
if ($unresolvedBefore.Count -gt 0) { throw 'V1 has an unresolved PBFT round.' }
foreach ($name in @('creg_pending_block_txs', 'creg_durable_finalized_work_depth',
    'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending')) {
    if ((Get-Metric $metricsBefore $name) -ne 0) { throw "V1 has pending work: $name" }
}
$deadLettersBefore = Get-Metric $metricsBefore 'creg_durable_finalized_work_dead_letter_depth'
if ($null -eq $deadLettersBefore) { throw 'V1 dead-letter baseline is unavailable.' }

$identityBefore = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
$phase = 'stage-scanner'
$mutationStarted = $false
try {
    New-Item -ItemType Directory -Path $stagingScanner | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $stagingScanner 'rules') | Out-Null
    foreach ($relative in @('osv_snapshot.json','manifest.json','known_malicious_hashes.json')) {
        Copy-Item -LiteralPath (Join-Path $bundle $relative) -Destination (Join-Path $stagingScanner $relative)
    }
    Copy-Item -LiteralPath (Join-Path $bundle 'rules\supply_chain.yar') -Destination (Join-Path $stagingScanner 'rules\supply_chain.yar')
    foreach ($relative in $expectedHashes.Keys) {
        $relativeTarget = $relative.Replace('rules\','rules\')
        if ((Get-FileHash -LiteralPath (Join-Path $stagingScanner $relativeTarget) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHashes[$relative]) {
            throw "Staged R14 scanner hash mismatch: $relative"
        }
    }
    Rename-Item -LiteralPath $stagingScanner -NewName (Split-Path $newScanner -Leaf)

    Copy-Item -LiteralPath $envPath -Destination $envBackup
    $updatedEnv = [regex]::Replace($envText, '(?m)^CREG_OSV_SNAPSHOT_EPOCH=.*(?=\r?$)', "CREG_OSV_SNAPSHOT_EPOCH=$newEpoch")
    $updatedEnv = [regex]::Replace($updatedEnv, '(?m)^CREG_SCANNER_PROFILE_DIGEST=.*(?=\r?$)', "CREG_SCANNER_PROFILE_DIGEST=$newProfile")
    $temporaryEnv = "$envPath.r14-$runId.tmp"
    [IO.File]::WriteAllText($temporaryEnv, $updatedEnv, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporaryEnv, $envPath, $true)
    $envHashAfterUpdate = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $phase = 'replace-container'
    $mutationStarted = $true
    & docker stop --time 30 $container | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'V1 did not stop cleanly.' }
    & docker rename $container $preserved | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not preserve the old V1 container.' }
    $runArgs = @(
        'run', '-d', '--name', $container, '--restart', 'unless-stopped', '--privileged',
        '--security-opt', 'label=disable', '--env-file', $envPath,
        '-p', '18080:8080', '-p', '9001:9000',
        '--mount', "type=bind,source=$data,target=/data",
        '--mount', "type=bind,source=$newScanner,target=/scanner,readonly",
        '--mount', 'type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock',
        $expectedImage
    )
    & docker @runArgs | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'The R14 V1 container did not start.' }

    $phase = 'postflight'
    $deadline = [DateTime]::UtcNow.AddMinutes(6)
    $health = $null
    $stats = $null
    $containerHealthy = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 5
            $stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
            $containerNow = Get-Container $container
            if ($containerNow) { $containerHealthy = $containerNow.State.Running -and $containerNow.State.Health.Status -eq 'healthy' }
            if ($health.status -eq 'ok' -and $stats.peer_count -gt 0 -and $containerHealthy) { break }
        } catch { }
        Start-Sleep -Seconds 3
    }
    if ($null -eq $health -or $health.status -ne 'ok' -or $null -eq $stats -or $stats.peer_count -lt 1 -or -not $containerHealthy) {
        throw 'V1 R14 container did not regain Docker health, API health, and peer connections within six minutes.'
    }

    $metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 10).Content
    $pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 10
    $packageResponse = Invoke-WebRequest -Uri $packageUri -TimeoutSec 10 -SkipHttpErrorCheck
    $unresolved = @($pbft.rounds | Where-Object {
        $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
    })
    $new = Get-Container $container
    $oldAfter = Get-Container $preserved
    $currentEnvHash = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $identityAfter = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $newMetrics = [ordered]@{
        profile_ready = (Get-Metric $metrics 'creg_scanner_profile_ready') -eq 1
        osv_ready = (Get-Metric $metrics 'creg_scanner_osv_ready') -eq 1
        osv_lookups = (Get-Metric $metrics 'creg_scanner_osv_lookups') -eq 8
        yara_ready = (Get-Metric $metrics 'creg_scanner_yara_ready') -eq 1
        threat_feed_ready = (Get-Metric $metrics 'creg_scanner_threat_intel_ready') -eq 1
        threat_feed_entries = (Get-Metric $metrics 'creg_scanner_threat_intel_entries') -eq 100
    }
    $newMounts = @($new.Mounts)
    $networkNames = @($new.NetworkSettings.Networks.PSObject.Properties.Name)
    $checks = [ordered]@{
        same_image = $new.Image -eq $expectedImage
        new_container_running_healthy = $new.State.Running -and $new.State.Health.Status -eq 'healthy'
        previous_container_preserved_stopped = $oldAfter.State.Status -eq 'exited'
        restart_security_network_ports_preserved = $new.HostConfig.RestartPolicy.Name -eq 'unless-stopped' -and $new.HostConfig.Privileged -and $new.HostConfig.SecurityOpt -contains 'label=disable' -and $new.HostConfig.NetworkMode -eq 'bridge' -and (@($new.HostConfig.PortBindings.'8080/tcp')[0].HostPort) -eq '18080' -and (@($new.HostConfig.PortBindings.'9000/tcp')[0].HostPort) -eq '9001'
        data_mount_unchanged = @($newMounts | Where-Object { $_.Destination -eq '/data' -and $_.Source -eq $data -and $_.RW }).Count -eq 1
        scanner_mount_read_only = @($newMounts | Where-Object { $_.Destination -eq '/scanner' -and $_.Source -eq $newScanner -and -not $_.RW }).Count -eq 1
        sandbox_socket_preserved = @($newMounts | Where-Object { $_.Destination -eq '/var/run/docker.sock' -and $_.Source -eq '/var/run/docker.sock' -and $_.RW }).Count -eq 1
        bridge_attachment_preserved = $networkNames -contains 'bridge'
        signed_r7_h18_unchanged = $stats.chain_id -eq $chainId -and $stats.genesis_hash -eq $networkGenesis -and $stats.spec_signature_verified -and $stats.tip_height -eq 18 -and $stats.finalized_height -eq 18 -and ([string]$stats.tip_hash).ToLowerInvariant().Replace('0x','') -eq $tipHash
        four_validator_roster = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
        validator_identity_and_peers = $health.is_validator -and $health.node_id -eq 'validator-1' -and $stats.peer_count -gt 0
        package_still_absent = $packageResponse.StatusCode -eq 404
        scanner_profile = $metrics -match ('profile_digest="' + $newProfile + '"') -and $newMetrics.profile_ready
        scanner_inputs_ready = $newMetrics.osv_ready -and $newMetrics.osv_lookups -and $newMetrics.yara_ready -and $newMetrics.threat_feed_ready -and $newMetrics.threat_feed_entries
        queues_clear = (Get-Metric $metrics 'creg_pending_block_txs') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_depth') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_ready_depth') -eq 0 -and (Get-Metric $metrics 'creg_forced_inclusion_pending') -eq 0
        dead_letters_unchanged = (Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth') -eq $deadLettersBefore
        pbft_clear = $unresolved.Count -eq 0
        identity_unchanged = $identityBefore -eq $identityAfter
        env_stable_after_start = $envHashAfterUpdate -eq $currentEnvHash
        signer_journal_preserved = Test-Path -LiteralPath $journalPath
        previous_scanner_preserved = Test-Path -LiteralPath $oldScanner
    }
    $evidence = [ordered]@{
        schema = 'creg-real-r14-live-v1-scanner-rollout-v1'
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        slot = 'v1'
        image_id = $new.Image
        node_binary_sha256 = ((& docker exec $container sha256sum /app/creg-node | Out-String).Trim() -split '\s+')[0]
        old_profile = $oldProfile
        scanner_profile_digest = $newProfile
        scanner_epoch = $newEpoch
        canonical_coverage = $canonical
        osv_snapshot_sha256 = $expectedHashes['osv_snapshot.json']
        manifest_sha256 = $expectedHashes['manifest.json']
        signed_tip = [ordered]@{ height = $stats.tip_height; hash = $stats.tip_hash; genesis = $stats.genesis_hash; chain_id = $stats.chain_id }
        dead_letter_depth = Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth'
        preserved_container = $preserved
        environment_backup = $envBackup
        scanner_metrics = $newMetrics
        checks = $checks
        passed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
        public_cutover = $false
    }
    New-Item -ItemType Directory -Path (Split-Path $EvidencePath -Parent) -Force | Out-Null
    $evidence | ConvertTo-Json -Depth 24 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
    if (-not $evidence.passed) { throw "V1 R14 postflight checks failed. Evidence: $EvidencePath" }
    Write-Output "V1 R14 scanner profile passed; signed h18 and persistent identity/signing state remain intact. Evidence: $EvidencePath"
}
catch {
    $failureMessage = $_.Exception.Message
    if ($mutationStarted) {
        try {
            $active = Get-Container $container
            if ($active -and (Get-Container $preserved)) {
                if ($active.State.Running) { & docker stop --time 30 $container | Out-Null }
                & docker rename $container $failed | Out-Null
                if ($LASTEXITCODE -ne 0) { throw 'Could not preserve the failed R14 container.' }
            }
            if (Test-Path -LiteralPath $envBackup) { Copy-Item -LiteralPath $envBackup -Destination $envPath -Force }
            $saved = Get-Container $preserved
            if ($saved) {
                & docker rename $preserved $container | Out-Null
                if ($LASTEXITCODE -ne 0) { throw 'Could not restore the preserved R13 container name.' }
                & docker start $container | Out-Null
                if ($LASTEXITCODE -ne 0) { throw 'Could not restart the preserved R13 container.' }
            } else {
                $original = Get-Container $container
                if ($original -and -not $original.State.Running) { & docker start $container | Out-Null }
            }
        } catch { $failureMessage += " Rollback requires inspection: $($_.Exception.Message)" }
    }
    $failure = [ordered]@{
        schema = 'creg-real-r14-live-v1-scanner-rollout-failure-v1'
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        slot = 'v1'
        failed_phase = $phase
        error = $failureMessage
        previous_container = $preserved
        failed_container = $failed
        environment_backup = $envBackup
        public_cutover = $false
    }
    try {
        New-Item -ItemType Directory -Path (Split-Path $EvidencePath -Parent) -Force | Out-Null
        $failure | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath ($EvidencePath + '.failure.json') -Encoding utf8
    } catch { }
    throw $failureMessage
}
