param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-queue-recovery-r12/post-osv-r9/v1-rollout.json')
)
$ErrorActionPreference = 'Stop'
$container = 'creg-phase-c-r7-v1'
$preserved = 'creg-phase-c-r7-v1-r12-osv-r8-preserved'
$image = 'creg-node:phase-c-secure-r12-bounded-l1-retry'
$acceptedImages = @(
    'sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd',
    'sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736'
)
$oldEpoch = 'osv-2026-09-26-phase-c-r8'
$newEpoch = 'osv-2026-09-26-phase-c-r9-write-check'
$oldProfile = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
$newProfile = 'dd52b5f77776aa0cad9cb25a4a308f9ef3755c1d3addec44073a5a322ee2e4fb'
$newOsvHash = 'c5901bfe95fb6bebb8285202bddb0604514f73739443669215e044a7ae66deaa'
$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$oldScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9'
$newScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r10-osv-r9'
$envPath = Join-Path $privateRoot 'node.env'
$backupEnv = Join-Path $privateRoot 'node.env.osv-r8-preserved'
$identityPath = Join-Path $data 'p2p\identity.key'
$journalPath = Join-Path $data 'pbft-safety.rocksdb'
$api = 'http://127.0.0.1:18080'
if (-not (Test-Path -LiteralPath $newScanner)) { throw 'Prepared r10 scanner bundle is missing.' }
if ((Get-FileHash -LiteralPath (Join-Path $newScanner 'osv_snapshot.json') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $newOsvHash) { throw 'Prepared OSV snapshot hash mismatch.' }
if ((Get-FileHash -LiteralPath (Join-Path $newScanner 'known_malicious_hashes.json') -Algorithm SHA256).Hash.ToLowerInvariant() -ne '5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef') { throw 'Pinned threat feed changed.' }
if ((Get-FileHash -LiteralPath (Join-Path $newScanner 'rules\supply_chain.yar') -Algorithm SHA256).Hash.ToLowerInvariant() -ne '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e') { throw 'Pinned YARA source changed.' }
if (-not (Test-Path -LiteralPath $envPath) -or -not (Test-Path -LiteralPath $identityPath) -or -not (Test-Path -LiteralPath $journalPath)) { throw 'V1 environment, P2P identity, or signer journal missing.' }
if (Test-Path -LiteralPath $backupEnv) { throw 'V1 OSV r8 environment backup already exists.' }
$envText = [IO.File]::ReadAllText($envPath)
foreach ($entry in @(
    @{ Key='CREG_OSV_SNAPSHOT_EPOCH'; Old=$oldEpoch; New=$newEpoch },
    @{ Key='CREG_SCANNER_PROFILE_DIGEST'; Old=$oldProfile; New=$newProfile }
)) {
    $pattern = '(?m)^' + [regex]::Escape($entry.Key + '=' + $entry.Old) + '(?=\r?$)'
    if ([regex]::Matches($envText, $pattern).Count -ne 1) { throw "$($entry.Key) does not have exactly one expected old value." }
}
$actualOldImage = (& docker inspect $container --format '{{.Image}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $actualOldImage -notin $acceptedImages) { throw 'V1 is not on pinned R12.' }
$oldState = (& docker inspect $container --format '{{.State.Status}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $oldState -ne 'running') { throw 'V1 is not running.' }
if ((& docker inspect $preserved --format '{{.Id}}' 2>$null | Out-String).Trim()) { throw 'V1 R12 preservation target already exists.' }
$mounts = @((& docker inspect $container | Out-String) | ConvertFrom-Json)[0].Mounts
$dataMount = @($mounts | Where-Object { $_.Destination -eq '/data' })[0]
$scannerMount = @($mounts | Where-Object { $_.Destination -eq '/scanner' })[0]
if ($null -eq $dataMount -or $dataMount.Source -notlike '*\.creg-phase-c\r7\v1\data') { throw 'Unexpected V1 persistent data mount.' }
if ($null -eq $scannerMount -or $scannerMount.RW -ne $false) { throw 'V1 scanner mount is not read-only.' }
$statsBefore = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
if ($statsBefore.tip_height -ne 4 -or $statsBefore.tip_hash -ne '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0') { throw 'V1 chain tip changed before profile rollout.' }
$pbftBefore = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
if (@($pbftBefore.rounds | Where-Object { $_.phase -notin @('finalised','failed') }).Count -gt 0) { throw 'V1 has an active PBFT round.' }
$identityBefore = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
Copy-Item -LiteralPath $envPath -Destination $backupEnv
$updatedEnv = $envText
$updatedEnv = [regex]::Replace($updatedEnv, '(?m)^CREG_OSV_SNAPSHOT_EPOCH=.*(?=\r?$)', "CREG_OSV_SNAPSHOT_EPOCH=$newEpoch")
$updatedEnv = [regex]::Replace($updatedEnv, '(?m)^CREG_SCANNER_PROFILE_DIGEST=.*(?=\r?$)', "CREG_SCANNER_PROFILE_DIGEST=$newProfile")
[IO.File]::WriteAllText($envPath, $updatedEnv, [Text.UTF8Encoding]::new($false))
& docker stop --time 20 $container | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'V1 did not stop cleanly.' }
& docker rename $container $preserved
if ($LASTEXITCODE -ne 0) { & docker start $container | Out-Null; throw 'Could not preserve the existing V1 R12 container.' }
$args = @(
    'run','-d','--name',$container,'--restart','unless-stopped','--privileged',
    '--env-file',$envPath,
    '-p','18080:8080','-p','9001:9000',
    '--mount',"type=bind,source=$data,target=/data",
    '--mount',"type=bind,source=$newScanner,target=/scanner,readonly",
    '-v','/var/run/docker.sock:/var/run/docker.sock',
    $image
)
& docker @args | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'R12 V1 scanner-profile container creation failed; previous process is preserved.' }
$actualImage = (& docker inspect $container --format '{{.Image}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $actualImage -notin $acceptedImages) { throw 'V1 did not start on the pinned R12 image.' }
$deadline = [DateTime]::UtcNow.AddMinutes(6)
$health = $null
while ([DateTime]::UtcNow -lt $deadline) {
    try { $health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 4; if ($health.status -eq 'ok') { break } } catch { }
    Start-Sleep -Seconds 2
}
if ($null -eq $health -or $health.status -ne 'ok') { throw 'V1 did not become healthy; both containers remain inspectable.' }
$peerDeadline = [DateTime]::UtcNow.AddMinutes(3)
$stats = $null
while ([DateTime]::UtcNow -lt $peerDeadline) {
    try {
        $stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
        if ($stats.peer_count -gt 0) { break }
    } catch { }
    Start-Sleep -Seconds 2
}
if ($null -eq $stats -or $stats.peer_count -le 0) { throw 'V1 API is healthy but peers did not reconnect within three minutes.' }
$metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8).Content
$pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
$identityAfter = (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
$checks = [ordered]@{
    same_tip = $stats.tip_height -eq 4 -and $stats.tip_hash -eq '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
    same_chain = $stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats.spec_signature_verified
    four_validators = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
    peers_connected = $stats.peer_count -gt 0
    scanner_profile_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($newProfile)
    queue_clear = $metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
    no_active_pbft = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised','failed') }).Count -eq 0
    p2p_identity_unchanged = $identityBefore -eq $identityAfter
    signer_journal_preserved = Test-Path -LiteralPath $journalPath
    old_container_preserved = (& docker inspect $preserved --format '{{.State.Status}}' | Out-String).Trim() -eq 'exited'
}
$evidence = [ordered]@{ schema='creg-real-r12-v1-osv-r9-rollout-v1'; captured_at_utc=[DateTime]::UtcNow.ToString('o'); slot='v1'; image_id=$actualImage; old_scanner_profile=$oldProfile; scanner_profile=$newProfile; osv_snapshot_sha256=$newOsvHash; old_container=$preserved; stats=$stats; health=$health; checks=$checks; passed=@($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0; public_cutover=$false }
$evidence | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $evidence.passed) { throw "V1 scanner-profile rollout failed; evidence at $EvidencePath" }
Write-Output "R12 V1 scanner-profile update passed; old R12 container and r8 scanner are preserved. Evidence: $EvidencePath"
