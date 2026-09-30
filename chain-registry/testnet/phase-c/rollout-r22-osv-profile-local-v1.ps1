param([string]$EvidencePath=(Join-Path $PSScriptRoot '../../../docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/r22-rollout/scanner-r11/v1-rollout.json'))
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$bundle=Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/cli-write/scanner-r11-bundle'
$oldEpoch='osv-2026-09-26-phase-c-r10-event-check';$oldProfile='d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$newEpoch='osv-2026-09-27-phase-c-kleur-r11';$newProfile='f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
$osvHash='452b326cc8d7bea568f21e2fd0d42d9b45503f481a8efe26a174ecb09cabe930';$manifestHash='bbaca082dbcece3fd4ef808b17057a27e7862135921bf694a1d2ff458518d932'
$registryRef='us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$container='creg-phase-c-r7-v1';$preserved='creg-phase-c-r7-v1-r22-before-osv-r11';$image='creg-node:phase-c-secure-r22-commit-capture'
$privateRoot='C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1';$data='C:\Users\samue\.creg-phase-c\r7\v1\data'
$envPath=Join-Path $privateRoot 'node.env';$envBackup=Join-Path $privateRoot 'node.env.osv-r10-preserved'
$newScanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r11-osv-kleur';$api='http://127.0.0.1:18080'
$expectedTip='a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d';$expectedGenesis='0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$started=$false
function Get-Metric([string]$Text,[string]$Name){
 $pattern='^'+[regex]::Escape($Name)+'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$'
 $line=$Text -split '\r?\n'|Where-Object {$_ -match $pattern}|Select-Object -First 1
 if(-not $line){return $null};return [double]([regex]::Match($line,$pattern).Groups[1].Value)
}
$want=@{'osv_snapshot.json'=$osvHash;'manifest.json'=$manifestHash;'known_malicious_hashes.json'='5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef';'rules/supply_chain.yar'='217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'}
foreach($rel in $want.Keys){$p=Join-Path $bundle $rel;if((Get-FileHash $p -Algorithm SHA256).Hash.ToLowerInvariant() -ne $want[$rel]){throw "Scanner hash mismatch: $rel"}}
if((Test-Path -LiteralPath $newScanner) -or (Test-Path -LiteralPath $envBackup)){throw 'V1 scanner backup target already exists.'}
if(-not (Test-Path $envPath) -or -not (Test-Path (Join-Path $data 'p2p\identity.key')) -or -not (Test-Path (Join-Path $data 'pbft-safety.rocksdb'))){throw 'V1 env, identity or signer journal missing.'}
$envText=[IO.File]::ReadAllText($envPath)
foreach($pair in @(@{k='CREG_OSV_SNAPSHOT_EPOCH';v=$oldEpoch},@{k='CREG_SCANNER_PROFILE_DIGEST';v=$oldProfile},@{k='CREG_NODE_ID';v='validator-1'},@{k='CREG_NODE_ROLE';v='validator'},@{k='CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM';v='false'})){
 if([regex]::Matches($envText,'(?m)^'+[regex]::Escape($pair.k+'='+$pair.v)+'(?=\r?$)').Count -ne 1){throw "$($pair.k) did not match the expected live value."}
}
if($envText -match '(?m)^CREG_PHASE_C_COMMIT_CAPTURE_'){throw 'Commit-capture mode must be unset.'}
$old=@((& docker inspect $container|Out-String)|ConvertFrom-Json)[0]
if($LASTEXITCODE -ne 0 -or $old.State.Status -ne 'running' -or $old.Image -notin @('sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3','sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9')){throw 'V1 is not running the pinned R22 image.'}
$imageId=$old.Image
$repo=@((& docker image inspect $imageId|Out-String)|ConvertFrom-Json)[0].RepoDigests
if($LASTEXITCODE -ne 0 -or $repo -notcontains $registryRef){throw 'V1 R22 registry digest mismatch.'}
$existingPreserved=(& docker container inspect $preserved --format '{{.Id}}' 2>$null|Out-String).Trim()
if($LASTEXITCODE -eq 0 -and $existingPreserved){throw 'V1 preservation container already exists.'}
$dm=@($old.Mounts|Where-Object {$_.Destination -eq '/data'})[0];$sm=@($old.Mounts|Where-Object {$_.Destination -eq '/scanner'})[0]
if($null -eq $dm -or $dm.Source -notlike '*\.creg-phase-c\r7\v1\data' -or $null -eq $sm -or $sm.RW -ne $false){throw 'V1 data/scanner mount differs from accepted config.'}
$stats=Invoke-RestMethod "$api/v1/public/chain/stats" -TimeoutSec 10;$health=Invoke-RestMethod "$api/v1/public/health" -TimeoutSec 10
$metrics=(Invoke-WebRequest "$api/metrics" -TimeoutSec 10).Content;$pbft=Invoke-RestMethod "$api/v1/consensus/pbft" -TimeoutSec 10
if($stats.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or -not $stats.spec_signature_verified -or $stats.genesis_hash -ne $expectedGenesis -or $stats.tip_height -ne 12 -or $stats.finalized_height -ne 12 -or $stats.tip_hash -ne $expectedTip -or $stats.package_count -ne 4){throw 'V1 chain identity or h12 changed.'}
if(-not $health.is_validator -or $stats.validator_count -ne 4 -or $stats.active_validators -ne 4 -or $stats.peer_count -lt 1){throw 'V1 validator readiness failed.'}
if($metrics -notmatch ('profile_digest="'+$oldProfile+'"') -or (Get-Metric $metrics 'creg_scanner_profile_ready') -ne 1){throw 'V1 old scanner profile is not ready.'}
if(@($pbft.rounds|Where-Object {$_.phase -notin @('finalised','failed')}).Count -gt 0){throw 'V1 has an active PBFT round.'}
foreach($name in @('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')){if((Get-Metric $metrics $name) -ne 0){throw "V1 has pending work: $name"}}
$dead=[int](Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth')
if($dead -ne 7){throw 'V1 dead-letter baseline changed.'}
$identityPath=Join-Path $data 'p2p\identity.key';$journal=Join-Path $data 'pbft-safety.rocksdb'
$identity=(Get-FileHash $identityPath -Algorithm SHA256).Hash.ToLowerInvariant()
New-Item -ItemType Directory -Path (Split-Path $newScanner -Parent) -Force|Out-Null
Copy-Item -LiteralPath $bundle -Destination $newScanner -Recurse
Copy-Item -LiteralPath $envPath -Destination $envBackup
$updated=[regex]::Replace($envText,'(?m)^CREG_OSV_SNAPSHOT_EPOCH=.*(?=\r?$)',"CREG_OSV_SNAPSHOT_EPOCH=$newEpoch")
$updated=[regex]::Replace($updated,'(?m)^CREG_SCANNER_PROFILE_DIGEST=.*(?=\r?$)',"CREG_SCANNER_PROFILE_DIGEST=$newProfile")
[IO.File]::WriteAllText($envPath,$updated,[Text.UTF8Encoding]::new($false))
try{
 & docker stop --time 20 $container|Out-Null
 if($LASTEXITCODE -ne 0){throw 'V1 did not stop cleanly.'}
 & docker rename $container $preserved|Out-Null
 if($LASTEXITCODE -ne 0){throw 'Could not preserve prior V1 container.'}
 $run=@('run','-d','--name',$container,'--restart','unless-stopped','--privileged','--env-file',$envPath,'-p','18080:8080','-p','9001:9000','--mount',"type=bind,source=$data,target=/data",'--mount',"type=bind,source=$newScanner,target=/scanner,readonly",'-v','/var/run/docker.sock:/var/run/docker.sock',$imageId)
 & docker @run|Out-Null
 if($LASTEXITCODE -ne 0){throw 'V1 scanner-profile container did not start.'}
 $started=$true
 $deadline=[DateTime]::UtcNow.AddMinutes(6);$health2=$null
 while([DateTime]::UtcNow -lt $deadline){try{$health2=Invoke-RestMethod "$api/v1/public/health" -TimeoutSec 5;if($health2.status -eq 'ok'){break}}catch{};Start-Sleep 2}
 if($null -eq $health2 -or $health2.status -ne 'ok'){throw 'V1 API did not recover.'}
 $deadline=[DateTime]::UtcNow.AddMinutes(3);$stats2=$null
 while([DateTime]::UtcNow -lt $deadline){try{$stats2=Invoke-RestMethod "$api/v1/public/chain/stats" -TimeoutSec 8;if($stats2.peer_count -gt 0){break}}catch{};Start-Sleep 2}
 if($null -eq $stats2 -or $stats2.peer_count -lt 1){throw 'V1 peers did not reconnect.'}
 $m2=(Invoke-WebRequest "$api/metrics" -TimeoutSec 10).Content;$p2=Invoke-RestMethod "$api/v1/consensus/pbft" -TimeoutSec 10
 $c2=@((& docker inspect $container|Out-String)|ConvertFrom-Json)[0]
 $man=Get-Content (Join-Path $newScanner 'manifest.json') -Raw|ConvertFrom-Json;$snap=Get-Content (Join-Path $newScanner 'osv_snapshot.json') -Raw|ConvertFrom-Json
 $checks=[ordered]@{
  same_r22_image=$c2.Image -eq $old.Image
  signed_chain=$stats2.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats2.spec_signature_verified
  same_genesis=$stats2.genesis_hash -eq $expectedGenesis
  h12_preserved=$stats2.tip_height -eq 12 -and $stats2.finalized_height -eq 12 -and $stats2.tip_hash -eq $expectedTip -and $stats2.package_count -eq 4
  four_validators=$stats2.validator_count -eq 4 -and $stats2.active_validators -eq 4
  validator_and_peers=$health2.is_validator -and $stats2.peer_count -gt 0
  scanner_profile=$m2 -match ('profile_digest="'+$newProfile+'"') -and (Get-Metric $m2 'creg_scanner_profile_ready') -eq 1
  scanner_inputs_ready=$man.osv_epoch -eq $newEpoch -and $man.scanner_profile_digest -eq $newProfile -and $snap.epoch -eq $newEpoch -and (Get-Metric $m2 'creg_scanner_osv_lookups') -eq 5 -and (Get-Metric $m2 'creg_scanner_osv_ready') -eq 1 -and (Get-Metric $m2 'creg_scanner_yara_ready') -eq 1 -and (Get-Metric $m2 'creg_scanner_threat_intel_ready') -eq 1 -and (Get-Metric $m2 'creg_scanner_threat_intel_entries') -eq 100
  queues_clear=(Get-Metric $m2 'creg_pending_block_txs') -eq 0 -and (Get-Metric $m2 'creg_durable_finalized_work_depth') -eq 0 -and (Get-Metric $m2 'creg_durable_finalized_work_ready_depth') -eq 0 -and (Get-Metric $m2 'creg_forced_inclusion_pending') -eq 0
  dead_letters_unchanged=(Get-Metric $m2 'creg_durable_finalized_work_dead_letter_depth') -eq $dead
  pbft_clear=@($p2.rounds|Where-Object {$_.phase -notin @('finalised','failed')}).Count -eq 0
  identity_unchanged=(Get-FileHash $identityPath -Algorithm SHA256).Hash.ToLowerInvariant() -eq $identity
  signer_journal_preserved=Test-Path $journal
  previous_container_preserved=(& docker inspect $preserved --format '{{.State.Status}}'|Out-String).Trim() -eq 'exited'
 }
 $ev=[ordered]@{schema='creg-real-r22-v1-scanner-profile-rollout-v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');slot='v1';prior_container=$preserved;old_profile=$oldProfile;scanner_profile_digest=$newProfile;scanner_epoch=$newEpoch;osv_snapshot_sha256=$osvHash;manifest_sha256=$manifestHash;stats=$stats2;health=$health2;checks=$checks;passed=@($checks.Values|Where-Object {$_ -ne $true}).Count -eq 0;public_cutover=$false}
 New-Item -ItemType Directory -Path (Split-Path $EvidencePath -Parent) -Force|Out-Null
 $ev|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $EvidencePath -Encoding utf8
 if(-not $ev.passed){throw "V1 readiness failed; evidence at $EvidencePath"}
}catch{
 if(-not $started){
  Copy-Item -LiteralPath $envBackup -Destination $envPath -Force
  $existingPreserved=(& docker container inspect $preserved --format '{{.Id}}' 2>$null|Out-String).Trim()
  if($LASTEXITCODE -eq 0 -and $existingPreserved){& docker rename $preserved $container|Out-Null;& docker start $container|Out-Null}
 }
 throw
}
Write-Output "V1 scanner profile passed; prior container and scanner bundle preserved. Evidence: $EvidencePath"
