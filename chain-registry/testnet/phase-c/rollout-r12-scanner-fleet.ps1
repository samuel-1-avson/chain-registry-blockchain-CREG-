param(
    [string]$EvidenceDirectory=(Join-Path $PSScriptRoot '../../../docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-rollout'),
    [ValidateSet('o1','o2','o3','o4','v2','v3','v4')][string[]]$Slots=@('o1','o2','o3','o4','v2','v3','v4')
)
$ErrorActionPreference='Stop'
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7+ is required.'}
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$bundle=Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-bundle'
$helper=Join-Path $PSScriptRoot 'rollout-r12-scanner-linux-node.sh'
$readiness=Join-Path $PSScriptRoot 'capture-r7-fleet-readiness.ps1'
$oldProfile='f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c'
$newProfile='e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2'
$newEpoch='osv-2026-09-27-phase-c-escape-string-regexp-r12-live'
$hashes=@{
 'osv_snapshot.json'='981e3388bc2b79b71daebb63f0c467bd11e06ff481766e9b9f12615de7d77928'
 'manifest.json'='ad5720ece2376ebf3bace85456988a74463a599f3081b49c924da93d84cc0efa'
 'known_malicious_hashes.json'='5b3aeb024b6e6bb0b82257aae81cb680b8d2aaff48b7753056493559e43861b1'
 'rules/supply_chain.yar'='217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
}
$fleet=@(
 @{slot='v2';role='validator';name='creg-validator-1';project='gen-lang-client-0022105784';zone='us-east1-b';port=18080},
 @{slot='v3';role='validator';name='creg-validator-3';project='fiffi-500411';zone='europe-west1-b';port=8080},
 @{slot='v4';role='validator';name='creg-validator-4';project='fiffi-500411';zone='asia-southeast1-b';port=8080},
 @{slot='o1';role='observer';name='creg-testnet-vm';project='gen-lang-client-0022105784';zone='us-central1-b';port=18080},
 @{slot='o2';role='observer';name='creg-observer-vm';project='gen-lang-client-0022105784';zone='us-central1-a';port=8080},
 @{slot='o3';role='observer';name='creg-observer-3';project='fiffi-500411';zone='us-west1-b';port=8080},
 @{slot='o4';role='observer';name='creg-observer-4';project='fiffi-500411';zone='africa-south1-a';port=8080}
)
$order=@('o1','o2','o3','o4','v2','v3','v4')
if(@($Slots|Select-Object -Unique).Count -ne $Slots.Count -or @($Slots|Where-Object {$_ -notin $order}).Count -gt 0){throw 'Slots must be unique remote R7 nodes.'}
$Slots=@($order|Where-Object {$_ -in $Slots})
if(-not $Slots.Count){throw 'Select at least one remote node.'}
foreach($relative in $hashes.Keys){
 $path=Join-Path $bundle $relative
 if((Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hashes[$relative]){throw "R12 input hash mismatch: $relative"}
}
$manifest=Get-Content (Join-Path $bundle 'manifest.json') -Raw|ConvertFrom-Json
$snapshot=Get-Content (Join-Path $bundle 'osv_snapshot.json') -Raw|ConvertFrom-Json
if($manifest.scanner_profile_digest -ne $newProfile -or $manifest.osv_epoch -ne $newEpoch -or $snapshot.epoch -ne $newEpoch -or $null -eq $snapshot.entries.'npm:escape-string-regexp@5.0.0'){throw 'R12 bundle manifest or package coverage differs.'}
New-Item -ItemType Directory -Path $EvidenceDirectory -Force|Out-Null
$run=[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$scriptHash=(Get-FileHash $helper -Algorithm SHA256).Hash.ToLowerInvariant()
$expected=@{}
$latestReadinessPath=$null
function Get-Profile([string]$Metrics){$m=[regex]::Match($Metrics,'profile_digest="([0-9a-f]{64})"');if($m.Success){return $m.Groups[1].Value};return $null}
function Get-Fleet([string]$Label,[string[]]$CaptureSlots=@()){
 $path=Join-Path $EvidenceDirectory "readiness-$Label-$run.json"
 if($CaptureSlots.Count -gt 0){
  if(-not $latestReadinessPath -or -not (Test-Path -LiteralPath $latestReadinessPath)){throw 'Partial fleet capture requires a prior full fleet snapshot.'}
  Copy-Item -LiteralPath $latestReadinessPath -Destination $path
  & $readiness -EvidencePath $path -ScannerProfileDigest $oldProfile -Slots $CaptureSlots|Out-Null
 }else{
  & $readiness -EvidencePath $path -ScannerProfileDigest $oldProfile|Out-Null
 }
 if($LASTEXITCODE -ne 0){throw "Fleet capture failed: $Label ($path)"}
 $cap=Get-Content $path -Raw|ConvertFrom-Json
 if($cap.nodes.Count -ne 8 -or -not $cap.all_eight_recovery_clear -or -not $cap.all_eight_pbft_clear){throw "Eight-node queue/PBFT gate failed: $Label ($path)"}
 foreach($node in $cap.nodes){
  $slot=[string]$node.slot
  if((Get-Profile ([string]$node.metrics)) -ne $expected[$slot]){throw "$slot scanner profile differs from rollout map: $Label"}
  $expectedNodeId="$($node.role)-$($slot.Substring(1))"
  if($node.health.node_id -ne $expectedNodeId){throw "$slot node identity differs from the reviewed fleet map: $Label"}
  if($node.stats.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or -not $node.stats.spec_signature_verified -or $node.stats.tip_height -ne 14 -or $node.stats.finalized_height -ne 14 -or $node.stats.tip_hash -ne '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa' -or $node.stats.validator_count -ne 4 -or $node.stats.active_validators -ne 4 -or $node.stats.peer_count -lt 1 -or -not $node.recovery_clear -or -not $node.pbft_clear){throw "$slot chain/recovery gate failed: $Label"}
  $failed=@($node.checks.PSObject.Properties|Where-Object {$_.Value -ne $true}|ForEach-Object {$_.Name})
  if(@($failed|Where-Object {$_ -ne 'scanner_profile_matches'}).Count -gt 0){throw "$slot non-profile readiness check failed: $Label"}
 }
 $script:latestReadinessPath=$path
 return $cap
}

$seedPath=Join-Path $EvidenceDirectory "readiness-seed-$run.json"
& $readiness -EvidencePath $seedPath -ScannerProfileDigest $oldProfile|Out-Null
if($LASTEXITCODE -ne 0){throw "Initial fleet capture failed: $seedPath"}
$initial=Get-Content $seedPath -Raw|ConvertFrom-Json
if($initial.nodes.Count -ne 8 -or -not $initial.all_eight_recovery_clear -or -not $initial.all_eight_pbft_clear){throw "Initial eight-node recovery/PBFT gate failed: $seedPath"}
foreach($node in $initial.nodes){
 $slot=[string]$node.slot
 $expected[$slot]=Get-Profile ([string]$node.metrics)
 if($node.health.node_id -ne "$($node.role)-$($slot.Substring(1))"){throw "$slot node identity differs from the reviewed fleet map in the initial snapshot."}
 if($node.stats.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or -not $node.stats.spec_signature_verified -or $node.stats.tip_height -ne 14 -or $node.stats.finalized_height -ne 14 -or $node.stats.tip_hash -ne '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa' -or $node.stats.validator_count -ne 4 -or $node.stats.active_validators -ne 4 -or $node.stats.peer_count -lt 1 -or -not $node.recovery_clear -or -not $node.pbft_clear){throw "$slot initial signed-chain/recovery gate failed."}
 $failed=@($node.checks.PSObject.Properties|Where-Object {$_.Value -ne $true}|ForEach-Object {$_.Name})
 if(@($failed|Where-Object {$_ -ne 'scanner_profile_matches'}).Count -gt 0){throw "$slot initial non-profile readiness check failed: $($failed -join ',')"}
}
$latestReadinessPath=$seedPath
$progress=[ordered]@{schema='creg-real-r12-scanner-fleet-rollout-v1';run_id=$run;started_at_utc=[DateTime]::UtcNow.ToString('o');old_profile=$oldProfile;new_profile=$newProfile;osv_epoch=$newEpoch;requested_slots=$Slots;nodes=@();passed=$false;public_cutover=$false}
foreach($slot in $Slots){
 if($expected[$slot] -eq $newProfile){Write-Output "$slot already reports R12; eight-node readiness passed.";continue}
 if($slot -like 'v*'){
  $initial=Get-Fleet "pre-$slot" @('v1','v2','v3','v4')
  if(@($initial.nodes|Where-Object {$_.role -eq 'validator' -and $_.container_state.Running -and $_.recovery_clear -and $_.pbft_clear}).Count -ne 4){throw "Four-validator fresh readiness gate failed before $slot restart."}
 }
 $hostInfo=@($fleet|Where-Object {$_.slot -eq $slot})[0]
 $current=$initial.nodes|Where-Object {$_.slot -eq $slot}|Select-Object -First 1
 $gcp=@("--project=$($hostInfo.project)","--zone=$($hostInfo.zone)",'--tunnel-through-iap','--quiet')
 $container="creg-phase-c-r7-$slot"
 $imageId=(& gcloud compute ssh $hostInfo.name @gcp "--command=sudo docker inspect '$container' --format '{{.Image}}'" 2>$null|Out-String).Trim()
 if($LASTEXITCODE -ne 0 -or $imageId -notin @('sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3','sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9','sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708','sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70','sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c')){throw "$slot image is outside the reviewed R7 canary set."}
 $stage="/tmp/creg-scanner-r12-$slot"
 $setup="install -d -m 700 '$stage' '$stage/rules'"
 & gcloud compute ssh $hostInfo.name @gcp "--command=$setup" 2>$null|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not make scanner staging directory on $slot."}
 & gcloud compute scp $helper "$($hostInfo.name):/tmp/rollout-r12-scanner-linux-node.sh" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage rollout helper on $slot."}
 $files=@((Join-Path $bundle 'osv_snapshot.json'),(Join-Path $bundle 'manifest.json'),(Join-Path $bundle 'known_malicious_hashes.json'))
 & gcloud compute scp @files "$($hostInfo.name):$stage/" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage R12 scanner bundle on $slot."}
 & gcloud compute scp (Join-Path $bundle 'rules/supply_chain.yar') "$($hostInfo.name):$stage/rules/" @gcp|Out-Null
 if($LASTEXITCODE -ne 0){throw "Could not stage R12 YARA rules on $slot."}
 $p2p=if($hostInfo.port -eq 18080){9001}else{9000};$nodeId="$($hostInfo.role)-$($slot.Substring(1))"
 $remote="printf '%s  %s\n' '$scriptHash' /tmp/rollout-r12-scanner-linux-node.sh | sha256sum -c - >/dev/null && sed -i 's/\r$//' /tmp/rollout-r12-scanner-linux-node.sh && sudo bash /tmp/rollout-r12-scanner-linux-node.sh '$slot' '$($hostInfo.role)' '$($hostInfo.port)' '$p2p' '$nodeId' '$imageId'"
 $output=(& gcloud compute ssh $hostInfo.name @gcp "--command=$remote" 2>&1|Out-String).Trim()
 if($LASTEXITCODE -ne 0){throw "R12 scanner rollout failed on $slot. $output"}
 $item=$output|ConvertFrom-Json
 if(-not $item.passed -or $item.scanner_profile_digest -ne $newProfile){throw "Node postflight failed on $slot."}
 $local=Join-Path $EvidenceDirectory "$slot-rollout-$run.json"
 $item|ConvertTo-Json -Depth 20|Set-Content -LiteralPath $local -Encoding utf8
 $expected[$slot]=$newProfile
 $progress.nodes+= [ordered]@{slot=$slot;host=$hostInfo.name;role=$hostInfo.role;image_id=$imageId;scanner_profile_digest=$newProfile;dead_letter_depth=$item.dead_letter_depth;report=(Split-Path $local -Leaf)}
 $progress|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceDirectory "progress-$run.json") -Encoding utf8
 Write-Output "$slot R12 profile passed its real-node health, signed-tip, queue, and PBFT checks."
}
$initial=Get-Fleet 'complete'
$progress.passed=$true;$progress.completed_at_utc=[DateTime]::UtcNow.ToString('o')
$progress.final_readiness=(Split-Path $latestReadinessPath -Leaf)
$progress|ConvertTo-Json -Depth 20|Set-Content (Join-Path $EvidenceDirectory "progress-$run.json") -Encoding utf8
Write-Output "Remote R12 scanner rollout complete: $($Slots -join ', '); final all-eight readiness passed."
