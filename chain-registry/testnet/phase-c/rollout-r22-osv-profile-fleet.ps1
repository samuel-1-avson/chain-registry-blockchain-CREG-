param(
 [string]$EvidenceDirectory=(Join-Path $PSScriptRoot '../../../docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/r22-rollout/scanner-r11'),
 [string[]]$Slots=@('o1','o2','o3','o4','v2','v3','v4')
)
$ErrorActionPreference='Stop'
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7+ required.'}
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$bundle=Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/cli-write/scanner-r11-bundle'
$nodeScript=Join-Path $PSScriptRoot 'rollout-r22-osv-profile-linux-node.sh'
$readiness=Join-Path $PSScriptRoot 'capture-r7-fleet-readiness.ps1'
$oldEpoch='osv-2026-09-26-phase-c-r10-event-check'
$oldProfile='d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$newEpoch='osv-2026-09-27-phase-c-kleur-r11'
$newProfile='f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
$osvHash='452b326cc8d7bea568f21e2fd0d42d9b45503f481a8efe26a174ecb09cabe930'
$manifestHash='bbaca082dbcece3fd4ef808b17057a27e7862135921bf694a1d2ff458518d932'
$registryRef='us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$acceptedImageIds=@('sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3','sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9')
$hashes=@{
 'osv_snapshot.json'=$osvHash
 'manifest.json'=$manifestHash
 'known_malicious_hashes.json'='5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef'
 'rules/supply_chain.yar'='217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
}
$fleet=@(
 @{slot='v1';role='validator';name='operator laptop';project='';zone='';port=18080},
 @{slot='v2';role='validator';name='creg-validator-1';project='gen-lang-client-0022105784';zone='us-east1-b';port=18080},
 @{slot='v3';role='validator';name='creg-validator-3';project='fiffi-500411';zone='europe-west1-b';port=8080},
 @{slot='v4';role='validator';name='creg-validator-4';project='fiffi-500411';zone='asia-southeast1-b';port=8080},
 @{slot='o1';role='observer';name='creg-testnet-vm';project='gen-lang-client-0022105784';zone='us-central1-b';port=18080},
 @{slot='o2';role='observer';name='creg-observer-vm';project='gen-lang-client-0022105784';zone='us-central1-a';port=8080},
 @{slot='o3';role='observer';name='creg-observer-3';project='fiffi-500411';zone='us-west1-b';port=8080},
 @{slot='o4';role='observer';name='creg-observer-4';project='fiffi-500411';zone='africa-south1-a';port=8080}
)
$order=@('o1','o2','o3','o4','v2','v3','v4')
if(@($Slots|Select-Object -Unique).Count -ne $Slots.Count -or @($Slots|Where-Object {$_ -notin $order}).Count -gt 0){throw 'Slots must be unique remote Phase C slots.'}
$Slots=@($order|Where-Object {$_ -in $Slots})
if($Slots.Count -eq 0){throw 'Select at least one remote node.'}
foreach($relative in $hashes.Keys){
 $path=Join-Path $bundle $relative
 if(-not (Test-Path -LiteralPath $path)){throw "Scanner file missing: $relative"}
 if((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hashes[$relative]){throw "Scanner file hash mismatch: $relative"}
}
$m=Get-Content (Join-Path $bundle 'manifest.json') -Raw|ConvertFrom-Json
$s=Get-Content (Join-Path $bundle 'osv_snapshot.json') -Raw|ConvertFrom-Json
if($m.osv_epoch -ne $newEpoch -or $m.scanner_profile_digest -ne $newProfile -or $s.epoch -ne $newEpoch -or $null -eq $s.entries.'npm:kleur@4.1.5'){throw 'Scanner bundle differs from reviewed candidate.'}
New-Item -ItemType Directory -Path $EvidenceDirectory -Force|Out-Null
$run=[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$progressPath=Join-Path $EvidenceDirectory "scanner-r11-remote-progress-$run.json"
$scriptHash=(Get-FileHash -LiteralPath $nodeScript -Algorithm SHA256).Hash.ToLowerInvariant()
$expected=@{}
$progress=[ordered]@{schema='creg-real-r22-scanner-r11-rollout-v1';run_id=$run;started_at_utc=[DateTime]::UtcNow.ToString('o');requested_slots=$Slots;old_profile=$oldProfile;new_profile=$newProfile;osv_epoch=$newEpoch;osv_snapshot_sha256=$osvHash;manifest_sha256=$manifestHash;public_cutover=$false;nodes=@();passed=$false}
function Get-ProfileDigest([string]$Metrics){
 $m=[regex]::Match($Metrics,'profile_digest="([0-9a-f]{64})"')
 if(-not $m.Success){return $null};return $m.Groups[1].Value
}
function Assert-CheckedFleet($cap,[string]$Label,[string]$path){
 if($cap.nodes.Count -ne 8 -or -not $cap.all_eight_recovery_clear -or -not $cap.all_eight_pbft_clear){throw "Fleet queues/PBFT gate failed at $Label. $path"}
 foreach($node in $cap.nodes){
  $slot=[string]$node.slot;$digest=Get-ProfileDigest ([string]$node.metrics)
  if($digest -ne $expected[$slot]){throw "$slot profile differs from the expected rollout map at $Label."}
  if($node.stats.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or -not $node.stats.spec_signature_verified -or $node.stats.tip_height -ne 12 -or $node.stats.finalized_height -ne 12 -or $node.stats.tip_hash -ne 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d' -or $node.stats.package_count -ne 4 -or -not $node.recovery_clear -or -not $node.pbft_clear){throw "$slot chain/recovery gate failed at $Label."}
  $failed=@($node.checks.PSObject.Properties|Where-Object {$_.Value -ne $true}|ForEach-Object {$_.Name})
  $allowed=@();if($digest -ne $oldProfile){$allowed=@('scanner_profile_matches')}
  if(@($failed|Where-Object {$_ -notin $allowed}).Count -gt 0){throw "$slot non-profile readiness check failed at $Label."}
 }
 if(@($cap.nodes|Where-Object {$_.role -eq 'validator' -and $_.stats.active_validators -eq 4 -and $_.stats.validator_count -eq 4}).Count -ne 4){throw "Four-validator roster failed at $Label."}
 return $cap
}
function Get-CheckedFleet([string]$Label){
 $path=Join-Path $EvidenceDirectory "readiness-$Label-$run.json"
 & $readiness -EvidencePath $path -ScannerProfileDigest $oldProfile|Out-Null
 $cap=Get-Content $path -Raw|ConvertFrom-Json
 return Assert-CheckedFleet $cap $Label $path
}
$initialPath=Join-Path $EvidenceDirectory "readiness-initial-$run.json"
& $readiness -EvidencePath $initialPath -ScannerProfileDigest $oldProfile|Out-Null
$initial=Get-Content $initialPath -Raw|ConvertFrom-Json
if($initial.nodes.Count -ne 8 -or -not $initial.all_eight_recovery_clear -or -not $initial.all_eight_pbft_clear){throw "Initial fleet recovery/PBFT gate failed. $initialPath"}
foreach($node in $initial.nodes){
 $actualProfile=Get-ProfileDigest ([string]$node.metrics)
 if($actualProfile -notin @($oldProfile,$newProfile)){throw "Unexpected scanner profile on $($node.slot)."}
 $expected[[string]$node.slot]=$actualProfile
}
$currentCapture=Assert-CheckedFleet $initial 'initial' $initialPath
foreach($slot in $Slots){
 $cap=$currentCapture
 if($expected[$slot] -eq $newProfile){
  $progress.nodes+= [ordered]@{slot=$slot;already_on_new_profile=$true;fleet_state='fresh eight-node capture';scanner_profile_digest=$newProfile}
  $progress|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $progressPath -Encoding utf8
  Write-Output "$slot already reports the pinned scanner profile; fresh fleet gates passed."
  continue
 }
 $hostInfo=@($fleet|Where-Object {$_.slot -eq $slot})[0]
 $target=@($cap.nodes|Where-Object {$_.slot -eq $slot})[0]
 if($target.role -eq 'validator' -and @($cap.nodes|Where-Object {$_.role -eq 'validator' -and $_.recovery_clear -and $_.pbft_clear}).Count -ne 4){throw 'All four validators must be ready before validator restart.'}
 $dead=[int]$target.recovery_queue.durable_finalized_work_dead_letter_depth
 if($null -eq $target.recovery_queue.durable_finalized_work_dead_letter_depth){throw "Dead-letter depth missing on $slot."}
 $gcp=@("--project=$($hostInfo.project)","--zone=$($hostInfo.zone)",'--tunnel-through-iap','--quiet')
 $container="creg-phase-c-r7-$slot"
 $imageId=(& gcloud compute ssh $hostInfo.name @gcp "--command=sudo docker inspect '$container' --format '{{.Image}}'" 2>$null|Out-String).Trim()
 if($LASTEXITCODE -ne 0 -or $imageId -notin $acceptedImageIds){throw "$slot is not using reviewed R22."}
 $stage="/tmp/creg-scanner-r11-$slot"
 & gcloud compute ssh $hostInfo.name @gcp "--command=install -d -m 700 '$stage' && install -d -m 700 '$stage/rules'" 2>$null|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not create private scanner staging directory on $slot."}
 $remoteScript='/tmp/rollout-r22-osv-profile-linux-node.sh'
 & gcloud compute scp $nodeScript "$($hostInfo.name):$remoteScript" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage node helper on $slot."}
 $files=@((Join-Path $bundle 'known_malicious_hashes.json'),(Join-Path $bundle 'manifest.json'),(Join-Path $bundle 'osv_snapshot.json'))
 & gcloud compute scp @files "$($hostInfo.name):$stage/" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage scanner files on $slot."}
 & gcloud compute scp (Join-Path $bundle 'rules/supply_chain.yar') "$($hostInfo.name):$stage/rules/" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage YARA rules on $slot."}
 $p2p=if($hostInfo.port -eq 18080){9001}else{9000}
 $nodeId="$($hostInfo.role)-$($slot.Substring(1))"
 $cmd="printf '%s  %s\n' '$scriptHash' '$remoteScript' | sha256sum -c - >/dev/null && sed -i 's/\r$//' '$remoteScript' && sudo bash '$remoteScript' '$slot' '$($hostInfo.role)' '$($hostInfo.port)' '$p2p' '$nodeId' '$dead' '$oldEpoch' '$oldProfile' '$newEpoch' '$newProfile' '$osvHash' '$manifestHash' '$imageId' '$registryRef'"
 $output=(& gcloud compute ssh $hostInfo.name @gcp "--command=$cmd" 2>&1|Out-String).Trim()
 if($LASTEXITCODE -ne 0){throw "Scanner rollout failed on $slot. $output"}
 $remoteEvidence="/var/lib/creg/phase-c-r7/evidence/scanner-r11-$slot-rollout.json"
 $remoteJson=(& gcloud compute ssh $hostInfo.name @gcp "--command=sudo cat '$remoteEvidence'" 2>$null|Out-String).Trim()
 if($LASTEXITCODE -ne 0 -or -not $remoteJson){throw "No rollout evidence returned from $slot."}
 $item=$remoteJson|ConvertFrom-Json
 if(-not $item.passed -or $item.scanner_profile_digest -ne $newProfile){throw "Node-side readiness failed on $slot."}
 $localEvidence=Join-Path $EvidenceDirectory "$slot-rollout-$run.json"
 $item|ConvertTo-Json -Depth 32|Set-Content -LiteralPath $localEvidence -Encoding utf8
 $expected[$slot]=$newProfile
 $currentCapture=Get-CheckedFleet "after-$slot"
 $progress.nodes+= [ordered]@{slot=$slot;host=$hostInfo.name;role=$hostInfo.role;image_id=$imageId;dead_letters_preserved=$dead;rollout_evidence=(Split-Path $localEvidence -Leaf);fleet_postflight="readiness-after-$slot-$run.json"}
 $progress|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $progressPath -Encoding utf8
 Write-Output "$slot scanner r11 ready; eight-node chain/recovery/PBFT postflight passed."
}
$progress.passed=$true;$progress.completed_at_utc=[DateTime]::UtcNow.ToString('o')
$progress|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $progressPath -Encoding utf8
Write-Output "Remote scanner r11 rollout passed: $($Slots -join ', '). Evidence: $progressPath"
