param(
    [Parameter(Mandatory=$true)][string]$ImageId,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)
$ErrorActionPreference='Stop'
if ($ImageId -notmatch '^sha256:[a-f0-9]{64}$') { throw 'Pinned image ID required.' }
$container='creg-phase-c-r7-v1'
$preserved="$container-r12-osv-r9-preserved-for-r16"
$image='creg-node:phase-c-secure-r16-committed-events'
$data='C:\Users\samue\.creg-phase-c\r7\v1\data'
$privateRoot='C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$envPath=Join-Path $privateRoot 'node.env'
$envBackup=Join-Path $privateRoot 'node.env.r12-osv-r9-preserved-for-r16'
$snapshot=Join-Path $privateRoot 'diagnostics-r16-h5'
$scanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r16-osv-r10'
$oldScanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r10-osv-r9'
$oldProfile='dd52b5f77776aa0cad9cb25a4a308f9ef3755c1d3addec44073a5a322ee2e4fb'
$newProfile='d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$identity=Join-Path $data 'p2p/identity.key'
$api='http://127.0.0.1:18080'
foreach ($path in @($envPath,$identity,(Join-Path $data 'chain.rocksdb'),(Join-Path $data 'pbft-safety.rocksdb'))) {
    if (-not(Test-Path -LiteralPath $path)) { throw "Required persistent path missing: $path" }
}
foreach ($path in @($envBackup,$snapshot,$EvidencePath)) {
    if (Test-Path -LiteralPath $path) { throw "Preservation/evidence path already exists: $path" }
}
if ((Get-FileHash (Join-Path $scanner 'osv_snapshot.json')).Hash.ToLowerInvariant() -ne 'eda8108603d2d674fcdc385635f2e7ac3091a044322214d1c1568b79de8986b9') { throw 'New OSV snapshot changed.' }
if ((Get-FileHash (Join-Path $oldScanner 'osv_snapshot.json')).Hash.ToLowerInvariant() -ne 'c5901bfe95fb6bebb8285202bddb0604514f73739443669215e044a7ae66deaa') { throw 'Prior OSV snapshot changed.' }
if ((& docker image inspect $image --format '{{.Id}}' | Out-String).Trim() -ne $ImageId) { throw 'R16 image pin mismatch.' }
$prior=@((& docker inspect $container | Out-String)|ConvertFrom-Json)[0]
if ($LASTEXITCODE -ne 0 -or -not $prior.State.Running -or $prior.Image -notin @(
 'sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd',
 'sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736')) { throw 'V1 is not the accepted running R12 node.' }
foreach ($mount in @(@{Path=$data;Target='/data'},@{Path=$oldScanner;Target='/scanner'})) {
    $actual=@($prior.Mounts|Where-Object Destination -eq $mount.Target)
    if ($actual.Count -ne 1 -or $actual[0].Source -ne $mount.Path) { throw "Unexpected mount for $($mount.Target)." }
}
if ((& docker inspect $preserved --format '{{.Id}}' 2>$null | Out-String).Trim()) { throw 'Preserved container name already exists.' }
$envText=[IO.File]::ReadAllText($envPath)
foreach ($entry in @("CREG_SCANNER_PROFILE_DIGEST=$oldProfile",'CREG_OSV_SNAPSHOT_EPOCH=osv-2026-09-26-phase-c-r9-write-check','CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false','CREG_NODE_ID=validator-1','CREG_NODE_ROLE=validator')) {
    if ([regex]::Matches($envText,'(?m)^'+[regex]::Escape($entry)+'\r?$').Count -ne 1) { throw "Expected environment entry missing or duplicated: $entry" }
}
function Read-LiveChecks([string]$Profile) {
 $s=Invoke-RestMethod "$api/v1/public/chain/stats" -TimeoutSec 8
 $h=Invoke-RestMethod "$api/v1/public/health" -TimeoutSec 8
 $p=Invoke-RestMethod "$api/v1/consensus/pbft" -TimeoutSec 8
 $m=(Invoke-WebRequest "$api/metrics" -TimeoutSec 8).Content
 $c=[ordered]@{
  same_h5=$s.tip_height -eq 5 -and $s.tip_hash -eq 'daab438828a678aee3aa2eebf041f5147bc6f76fed837a207e6a1f4747d427cf'
  signed_chain=$s.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $s.spec_signature_verified
  four_validators=$s.validator_count -eq 4 -and $s.active_validators -eq 4
  scanner_ready=$m -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $m.Contains("profile_digest=`"$Profile`"")
  queues_clear=@(@('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')|Where-Object{$m -notmatch ('(?m)^'+$_+' 0\s*$')}).Count -eq 0
  no_active_pbft=@($p.rounds|Where-Object phase -NotIn @('finalised','failed')).Count -eq 0
  peers_connected=$s.peer_count -gt 0
  real_isolated_sandbox=$h.sandbox.engine -eq 'nsjail' -and $h.sandbox.isolated -and -not $h.sandbox.dev_bypass
 }
 return @{checks=$c;passed=@($c.Values|Where-Object{$_ -ne $true}).Count -eq 0;stats=$s;health=$h}
}
$before=Read-LiveChecks $oldProfile
if (-not $before.passed) { throw 'V1 preflight failed.' }
$identityBefore=(Get-FileHash $identity).Hash
Copy-Item -LiteralPath $envPath -Destination $envBackup
& docker stop --time 20 $container | Out-Null
if ($LASTEXITCODE -ne 0 -or (& docker inspect $container --format '{{.State.Status}}'|Out-String).Trim() -ne 'exited') { throw 'V1 did not stop.' }
New-Item -ItemType Directory -Path $snapshot | Out-Null
Copy-Item -LiteralPath (Join-Path $data 'chain.rocksdb') -Destination (Join-Path $snapshot 'chain.rocksdb') -Recurse
$review=(& docker run --rm --user 0 --entrypoint /app/creg_recovery_inspect --mount "type=bind,source=$snapshot,target=/snapshot,readonly" $ImageId /snapshot | Out-String)
if ($LASTEXITCODE -ne 0) { & docker start $container|Out-Null; throw 'Read-only inspection failed; V1 restarted on R12.' }
$queueReview=$review|ConvertFrom-Json
& docker rename $container $preserved
if ($LASTEXITCODE -ne 0) { & docker start $container|Out-Null; throw 'Could not preserve old node.' }
$updated=[regex]::Replace($envText,'(?m)^CREG_SCANNER_PROFILE_DIGEST=[^\r\n]*',"CREG_SCANNER_PROFILE_DIGEST=$newProfile")
$updated=[regex]::Replace($updated,'(?m)^CREG_OSV_SNAPSHOT_EPOCH=[^\r\n]*','CREG_OSV_SNAPSHOT_EPOCH=osv-2026-09-26-phase-c-r10-event-check')
[IO.File]::WriteAllText($envPath,$updated,[Text.UTF8Encoding]::new($false))
$args=@('run','-d','--name',$container,'--restart','unless-stopped','--privileged','--env-file',$envPath,
 '-p','18080:8080','-p','9001:9000','--mount',"type=bind,source=$data,target=/data",
 '--mount',"type=bind,source=$scanner,target=/scanner,readonly",'-v','/var/run/docker.sock:/var/run/docker.sock',$ImageId)
& docker @args | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'New V1 failed creation; old process and environment preserved.' }
$deadline=[DateTime]::UtcNow.AddMinutes(6)
$after=@{passed=$false}
while ([DateTime]::UtcNow -lt $deadline) {
    try { $after=Read-LiveChecks $newProfile; if($after.passed){break} } catch { }
    Start-Sleep -Seconds 2
}
$checks=[ordered]@{
 live_readiness=$after.passed
 pinned_image=((& docker inspect $container --format '{{.Image}}'|Out-String).Trim() -eq $ImageId)
 p2p_identity_preserved=(Get-FileHash $identity).Hash -eq $identityBefore
 signer_journal_preserved=Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
 prior_container_preserved=((& docker inspect $preserved --format '{{.State.Status}}'|Out-String).Trim() -eq 'exited')
}
$record=[ordered]@{schema='creg-real-r16-rollout-v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');slot='v1';prior_image=$prior.Image;image_id=$ImageId;preserved_container=$preserved;before=$before;after=$after;queue_review=$queueReview;checks=$checks;passed=@($checks.Values|Where-Object{$_ -ne $true}).Count -eq 0;public_cutover=$false}
$record|ConvertTo-Json -Depth 25|Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $record.passed) { throw 'V1 R16 acceptance failed; review saved evidence.' }
Write-Output 'V1 R16 replacement and read-only queue review passed.'
