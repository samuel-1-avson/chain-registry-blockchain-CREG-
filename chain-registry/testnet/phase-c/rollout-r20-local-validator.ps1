param(
    [Parameter(Mandatory=$true)][string]$ImageId,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)
$ErrorActionPreference='Stop'
if ($ImageId -notmatch '^sha256:[a-f0-9]{64}$') { throw 'Pinned image ID required.' }
$container='creg-phase-c-r7-v1'
$preserved="$container-r18-osv-r10-preserved-for-r20"
$image='creg-node:phase-c-secure-r20-durable-revocation-delivery'
$data='C:\Users\samue\.creg-phase-c\r7\v1\data'
$privateRoot='C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$envPath=Join-Path $privateRoot 'node.env'
$envBackup=Join-Path $privateRoot 'node.env.r18-osv-r10-preserved-for-r20'
$snapshot=Join-Path $privateRoot 'diagnostics-r20-h6'
$scanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r16-osv-r10'
$oldScanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r16-osv-r10'
$oldProfile='d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
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
if ((Get-FileHash (Join-Path $oldScanner 'osv_snapshot.json')).Hash.ToLowerInvariant() -ne 'eda8108603d2d674fcdc385635f2e7ac3091a044322214d1c1568b79de8986b9') { throw 'Prior OSV snapshot changed.' }
if ((& docker image inspect $image --format '{{.Id}}' | Out-String).Trim() -ne $ImageId) { throw 'R20 image pin mismatch.' }
$prior=@((& docker inspect $container | Out-String)|ConvertFrom-Json)[0]
if ($LASTEXITCODE -ne 0 -or -not $prior.State.Running -or $prior.Image -notin @(
 'sha256:dd07eef4a657707fdad696b1b4cc7397ee2589b82e3511c892ff3a806f3dcb03',
 'sha256:cee77b5210ad67a91f5d6e5ccbf2b5ec914ccf53268f8294b261bc7a37c7f49a')) { throw 'V1 is not the accepted running R18 node.' }
foreach ($mount in @(@{Path=$data;Target='/data'},@{Path=$oldScanner;Target='/scanner'})) {
    $actual=@($prior.Mounts|Where-Object Destination -eq $mount.Target)
    if ($actual.Count -ne 1 -or $actual[0].Source -ne $mount.Path) { throw "Unexpected mount for $($mount.Target)." }
}
if ((& docker inspect $preserved --format '{{.Id}}' 2>$null | Out-String).Trim()) { throw 'Preserved container name already exists.' }
$envText=[IO.File]::ReadAllText($envPath)
foreach ($entry in @("CREG_SCANNER_PROFILE_DIGEST=$oldProfile",'CREG_OSV_SNAPSHOT_EPOCH=osv-2026-09-26-phase-c-r10-event-check','CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false','CREG_NODE_ID=validator-1','CREG_NODE_ROLE=validator')) {
    if ([regex]::Matches($envText,'(?m)^'+[regex]::Escape($entry)+'\r?$').Count -ne 1) { throw "Expected environment entry missing or duplicated: $entry" }
}
function Read-LiveChecks([string]$Profile,[bool]$After=$false) {
 $s=Invoke-RestMethod "$api/v1/public/chain/stats" -TimeoutSec 8
 $h=Invoke-RestMethod "$api/v1/public/health" -TimeoutSec 8
 $p=Invoke-RestMethod "$api/v1/consensus/pbft" -TimeoutSec 8
 $m=(Invoke-WebRequest "$api/metrics" -TimeoutSec 8).Content
 $c=[ordered]@{
  same_h6=$s.tip_height -eq 6 -and $s.tip_hash -eq 'd8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'
  signed_chain=$s.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $s.spec_signature_verified
  four_validators=$s.validator_count -eq 4 -and $s.active_validators -eq 4
  scanner_ready=$m -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $m.Contains("profile_digest=`"$Profile`"")
  retained_revocation_only=@(@('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth')|Where-Object{$m -notmatch ('(?m)^'+$_+' 1\s*$')}).Count -eq 0 -and $m -match '(?m)^creg_forced_inclusion_pending 0\s*$'
  admissions_retired=-not $After -or $s.pending_tx_count -eq 0
  no_active_pbft=@($p.rounds|Where-Object{$_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)}).Count -eq 0
  peers_connected=$s.peer_count -gt 0
  real_isolated_sandbox=$h.sandbox.engine -eq 'nsjail' -and $h.sandbox.isolated -and -not $h.sandbox.dev_bypass
 }
 return @{checks=$c;passed=@($c.Values|Where-Object{$_ -ne $true}).Count -eq 0;stats=$s;health=$h}
}
$before=Read-LiveChecks $oldProfile
if (-not $before.passed) { $before|ConvertTo-Json -Depth 12|Set-Content "$EvidencePath.preflight-failure.json"; throw 'V1 preflight failed; read-only evidence saved.' }
$certificate=(& node (Join-Path $PSScriptRoot 'verify-real-tip-certificate.cjs') $api (Join-Path $PSScriptRoot 'r7/chain-spec.json') 6 'd8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'|Out-String)
if($LASTEXITCODE -ne 0){throw 'V1 canonical COMMIT certificate failed; no process stopped.'}
$identityBefore=(Get-FileHash $identity).Hash
try {
Copy-Item -LiteralPath $envPath -Destination $envBackup
& docker stop --time 20 $container | Out-Null
if ($LASTEXITCODE -ne 0 -or (& docker inspect $container --format '{{.State.Status}}'|Out-String).Trim() -ne 'exited') { throw 'V1 did not stop.' }
New-Item -ItemType Directory -Path $snapshot | Out-Null
Copy-Item -LiteralPath (Join-Path $data 'chain.rocksdb') -Destination (Join-Path $snapshot 'chain.rocksdb') -Recurse
if(Test-Path (Join-Path $data 'pending_pool.json')){Copy-Item -LiteralPath (Join-Path $data 'pending_pool.json') -Destination (Join-Path $snapshot 'pending_pool.json')}
$review=(& docker run --rm --user 0 --entrypoint /app/creg_recovery_inspect --mount "type=bind,source=$snapshot,target=/snapshot,readonly" $ImageId /snapshot | Out-String)
if ($LASTEXITCODE -ne 0) { & docker start $container|Out-Null; throw 'Read-only inspection failed; V1 restarted on R18.' }
$queueReview=$review|ConvertFrom-Json
if($queueReview.active_count -ne 1 -or @($queueReview.active|Where-Object{$_.work_id -ne '52d1bdd74854d98089711e7809328bea9118f85db2619548165c539bca836681' -or $_.transaction.type -ne 'revoke' -or $_.transaction.canonical -ne 'npm:toidentifier@1.0.1'}).Count -gt 0){ & docker start $container|Out-Null; throw 'Retained work differs from the actual authorized revocation; V1 restarted unchanged.' }
& docker rename $container $preserved
if ($LASTEXITCODE -ne 0) { & docker start $container|Out-Null; throw 'Could not preserve old node.' }
$args=@('run','-d','--name',$container,'--restart','unless-stopped','--privileged','--env-file',$envPath,
 '-p','18080:8080','-p','9001:9000','--mount',"type=bind,source=$data,target=/data",
 '--mount',"type=bind,source=$scanner,target=/scanner,readonly",'-v','/var/run/docker.sock:/var/run/docker.sock',$ImageId)
& docker @args | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'New V1 failed creation; old process and environment preserved.' }
$deadline=[DateTime]::UtcNow.AddMinutes(6)
$after=@{passed=$false}
while ([DateTime]::UtcNow -lt $deadline) {
    try { $after=Read-LiveChecks $newProfile $true; if($after.passed){break} } catch { }
    Start-Sleep -Seconds 2
}
$checks=[ordered]@{
 live_readiness=$after.passed
 pinned_image=((& docker inspect $container --format '{{.Image}}'|Out-String).Trim() -eq $ImageId)
 p2p_identity_preserved=(Get-FileHash $identity).Hash -eq $identityBefore
 signer_journal_preserved=Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
 prior_container_preserved=((& docker inspect $preserved --format '{{.State.Status}}'|Out-String).Trim() -eq 'exited')
}
$postCertificate=(& node (Join-Path $PSScriptRoot 'verify-real-tip-certificate.cjs') $api (Join-Path $PSScriptRoot 'r7/chain-spec.json') 6 'd8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'|Out-String)
$checks.canonical_commit_certificate=$LASTEXITCODE -eq 0
$record=[ordered]@{schema='creg-real-r20-rollout-v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');slot='v1';prior_image=$prior.Image;image_id=$ImageId;preserved_container=$preserved;before=$before;after=$after;queue_review=$queueReview;checks=$checks;passed=@($checks.Values|Where-Object{$_ -ne $true}).Count -eq 0;public_cutover=$false}
$record|ConvertTo-Json -Depth 25|Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $record.passed) { throw 'V1 R20 acceptance failed; review saved evidence.' }
Write-Output 'V1 R20 replacement and read-only queue review passed.'
} catch {
    $current=(& docker inspect $container --format '{{.State.Status}}' 2>$null|Out-String).Trim()
    if(-not $current -and (& docker inspect $preserved --format '{{.State.Status}}' 2>$null|Out-String).Trim()) {
        & docker rename $preserved $container|Out-Null
        $current=(& docker inspect $container --format '{{.State.Status}}' 2>$null|Out-String).Trim()
    }
    if($current -in @('exited','created')){ & docker start $container|Out-Null }
    throw
}
