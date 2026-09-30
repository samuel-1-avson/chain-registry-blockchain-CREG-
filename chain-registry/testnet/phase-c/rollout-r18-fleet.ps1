param(
 [Parameter(Mandatory=$true)][string]$ImageManifest,
 [Parameter(Mandatory=$true)][string]$ArchivePath,
 [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
 [Parameter(Mandatory=$true)][string]$FleetReadinessPath,
 [ValidateSet('o1','o2','o3','o4','v1','v2','v3','v4')][string[]]$Slots=@('o1','o2','o3','o4','v2','v3','v4','v1'),
 [switch]$UseStagedArtifacts
)
$ErrorActionPreference='Stop'
$manifest=Get-Content -LiteralPath $ImageManifest -Raw|ConvertFrom-Json
$fleet=Get-Content -LiteralPath $FleetReadinessPath -Raw|ConvertFrom-Json
if (-not $fleet.all_eight_ready -or -not $fleet.all_eight_recovery_clear -or $fleet.nodes.Count -ne 8) { throw 'Accepted eight-node baseline required.' }
if ($manifest.image_tag -ne 'creg-node:phase-c-secure-r18-canonical-round-recovery' -or $manifest.compilation -ne 'passed') { throw 'R18 compilation/pin evidence required.' }
$image=$manifest.image_tag
$accepted=@($manifest.accepted_image_ids)
$archiveHash=(Get-FileHash -LiteralPath $ArchivePath).Hash.ToLowerInvariant()
if ($archiveHash -notin @($manifest.archive_sha256,$manifest.delta_archive_sha256)) { throw 'Local image archive changed.' }
New-Item -ItemType Directory -Path $EvidenceDirectory -Force|Out-Null
$osv=(Resolve-Path (Join-Path $EvidenceDirectory '../osv-snapshot-candidate.json')).Path
$script=(Resolve-Path (Join-Path $PSScriptRoot 'rollout-r18-linux-node.sh')).Path
$signedSpec=(Resolve-Path (Join-Path $PSScriptRoot 'r7/chain-spec.json')).Path
if ((Get-FileHash $signedSpec).Hash.ToLowerInvariant() -ne '331897f5a3f89fe902e4a7d814a9245ef50b384e22cc3d8dc946b4b45ed39e9f') { throw 'Authenticated R7 spec changed.' }
if ((Get-FileHash $osv).Hash.ToLowerInvariant() -ne 'eda8108603d2d674fcdc385635f2e7ac3091a044322214d1c1568b79de8986b9') { throw 'Reviewed OSV snapshot changed.' }
$records=@()
foreach($priorEvidence in Get-ChildItem -LiteralPath $EvidenceDirectory -File -Filter '*-rollout.json') {
 $priorRecord=Get-Content -LiteralPath $priorEvidence.FullName -Raw|ConvertFrom-Json
 if($priorRecord.passed -and $priorRecord.image_id -in $accepted -and $priorRecord.slot -notin $Slots){$records+=$priorRecord}
}
# Observers first; replace one validator at a time, with laptop last.
foreach ($slot in $Slots) {
 $path=Join-Path $EvidenceDirectory "$slot-rollout.json"
 if(Test-Path -LiteralPath $path){throw "Existing evidence preserved: $slot"}
 $hostInfo=@($fleet.nodes|Where-Object slot -eq $slot)[0]
 Write-Output "Starting guarded R18 replacement on $slot."
 if($slot -eq 'v1'){
  $actual=(& docker image inspect $image --format '{{.Id}}'|Out-String).Trim()
  if($actual -notin $accepted){throw 'Local image is not the accepted R18 artifact.'}
  & (Join-Path $PSScriptRoot 'rollout-r18-local-validator.ps1') -ImageId $actual -EvidencePath $path
  if($LASTEXITCODE -ne 0){throw 'Local rollout failed.'}
 }else{
  $gcp=@("--project=$($hostInfo.project)","--zone=$($hostInfo.zone)",'--tunnel-through-iap','--quiet')
  if(-not $UseStagedArtifacts){
  & gcloud compute scp $ArchivePath "$($hostInfo.name):/tmp/creg-node-r18.tar.gz" @gcp
  if($LASTEXITCODE -ne 0){throw "Image transfer failed: $slot"}
  & gcloud compute scp $script "$($hostInfo.name):/tmp/rollout-r18-linux-node.sh" @gcp
  if($LASTEXITCODE -ne 0){throw "Script transfer failed: $slot"}
  & gcloud compute scp $osv "$($hostInfo.name):/tmp/creg-osv-snapshot-r18.json" @gcp
  if($LASTEXITCODE -ne 0){throw "Snapshot transfer failed: $slot"}
  & gcloud compute scp $signedSpec "$($hostInfo.name):/tmp/creg-r7-signed-spec-r18.json" @gcp
  if($LASTEXITCODE -ne 0){throw "Public signed spec transfer failed: $slot"}
  }
  $load="printf '%s  %s\n' '$archiveHash' /tmp/creg-node-r18.tar.gz | sha256sum -c - >/dev/null && sudo docker load -i /tmp/creg-node-r18.tar.gz >/dev/null && sudo docker image inspect '$image' --format '{{.Id}}'"
  $actual=(& gcloud compute ssh $hostInfo.name @gcp "--command=$load" 2>(Join-Path $EvidenceDirectory "$slot-image-load-stderr.txt")|Out-String).Trim()
  if($LASTEXITCODE -ne 0 -or $actual -notin $accepted){throw "Remote loaded image pin mismatch: $slot"}
  $p2p=if($hostInfo.port -eq 18080){9001}else{9000}
  # SCP preserves Windows newlines. Normalize this public script without
  # touching the private environment or persistent data.
  $command="sed -i 's/\r$//' /tmp/rollout-r18-linux-node.sh && bash /tmp/rollout-r18-linux-node.sh $slot $($hostInfo.role) $($hostInfo.port) $p2p $actual"
  $output=(& gcloud compute ssh $hostInfo.name @gcp "--command=$command" 2>(Join-Path $EvidenceDirectory "$slot-rollout-stderr.txt")|Out-String).Trim()
  if($LASTEXITCODE -ne 0){$output|Set-Content (Join-Path $EvidenceDirectory "$slot-failure.txt");throw "Guarded rollout failed on $slot; inspect remote preservation state."}
  $record=$output|ConvertFrom-Json
  if(-not $record.passed){throw "Acceptance failed on $slot"}
  $record|ConvertTo-Json -Depth 25|Set-Content -LiteralPath $path -Encoding utf8
 }
 $records+=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
 @{schema='creg-real-r18-fleet-rollout-v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');all_eight_passed=$records.Count -eq 8 -and @($records|Where-Object{-not $_.passed}).Count -eq 0;nodes=$records;public_cutover=$false}|ConvertTo-Json -Depth 28|Set-Content (Join-Path $EvidenceDirectory 'fleet-rollout.json')
 Write-Output "R18 replacement passed on $slot; data, identity and signer journal preserved."
}
