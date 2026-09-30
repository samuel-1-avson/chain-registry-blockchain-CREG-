param([Parameter(Mandatory=$true)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$evidence=(Resolve-Path $EvidenceDirectory).Path
$repo=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$manifest=Join-Path $evidence 'r20-image-build.json'
$accepted=Get-Content (Join-Path $evidence 'rollout-r20/fleet-rollout.json') -Raw|ConvertFrom-Json
if(-not $accepted.all_eight_passed){throw 'All eight real R20 replacements must pass first.'}
$node=(Get-Command node).Source
$report=[ordered]@{schema='creg-real-r20-quorum-loss-recovery-v1';started_at_utc=[DateTime]::UtcNow.ToString('o');public_cutover=$false;passed=$false;samples=@();stopped_nodes=@();restored_nodes=@()}
$path=Join-Path $evidence 'quorum-loss-h7-r20.json'
if(Test-Path $path){throw 'Prior recovery evidence is preserved; choose a new directory.'}
function Save-Report { $report|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $path -Encoding utf8 }
function Cloud([string]$Slot,[string]$Command) {
 $name=if($Slot -eq 'v2'){'creg-validator-1'}else{"creg-validator-$($Slot.Substring(1))"}
 $project=if($Slot -eq 'v2'){'gen-lang-client-0022105784'}else{'fiffi-500411'}
 $zone=switch($Slot){v2{'us-east1-b'}v3{'europe-west1-b'}v4{'asia-southeast1-b'}}
 $output=(& gcloud compute ssh $name "--project=$project" "--zone=$zone" --tunnel-through-iap --quiet "--command=$Command" 2>> (Join-Path $evidence 'quorum-loss-h7-r20-ssh.log')|Out-String).Trim()
 if($LASTEXITCODE -ne 0){throw "Cloud operation failed on $Slot; protected diagnostics retained."}
 return $output
}
function Start-NodeHelper([string]$Script,[string[]]$Arguments,[string]$LogStem,[string]$Workdir=$repo){
 $args=@(('"'+$Script+'"'))+$Arguments
 Start-Process -FilePath $node -ArgumentList $args -WorkingDirectory $Workdir -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $evidence "$LogStem.stdout.log") -RedirectStandardError (Join-Path $evidence "$LogStem.stderr.log")
}
$mustRestore=[Collections.Generic.List[string]]::new()
try {
 $before=Invoke-RestMethod http://127.0.0.1:18080/v1/public/chain/stats -TimeoutSec 8
 if($before.tip_height -ne 6 -or $before.tip_hash -ne 'd8814d091c8a50c2c7bfd535b7524d09bfce17b8c63b6b72c33699b8e52f9fb6'){throw 'Accepted real h6 baseline changed.'}
 $report.before=$before;Save-Report
 foreach($slot in @('v3','v4')){
  # An independent host timer restores the original container if this task is
  # interrupted. It does not alter its image, volume, key or consensus settings.
  $mustRestore.Add($slot)
  $unit="creg-r20-$slot-drill-restore"
  $command="sudo docker exec creg-phase-c-r7-$slot curl -fsS http://127.0.0.1:8080/v1/consensus/pbft; sudo systemd-run --unit=$unit --on-active=5m /usr/bin/docker start creg-phase-c-r7-$slot >/dev/null && sudo docker stop --time 10 creg-phase-c-r7-$slot >/dev/null && sudo docker inspect creg-phase-c-r7-$slot --format '{{.State.Status}}'"
  $result=Cloud $slot $command
  if(-not $result.EndsWith('exited')){throw "Candidate stop not confirmed on $slot"}
  $report.stopped_nodes+=@{slot=$slot;captured_at_utc=[DateTime]::UtcNow.ToString('o');result=$result;restore_timer=$unit};Save-Report
 }
 $env:CREG_NODE_URL='http://127.0.0.1:18081'
 $browser=Start-NodeHelper (Join-Path $repo 'chain-registry/explorer/scripts/capture-real-explorer-events.mjs') @('http://127.0.0.1:3008',('"'+(Join-Path $evidence 'browser-toidentifier-revoke-r20.json')+'"'),'npm:toidentifier@1.0.1','6','revoke') 'browser-toidentifier-revoke-r20' (Join-Path $repo 'chain-registry/explorer')
 $monitor=Start-NodeHelper (Join-Path $PSScriptRoot 'capture-real-prepared-restart.cjs') @(('"'+(Join-Path $evidence 'prepared-crash-h7-r20.json')+'"'),'7',('"'+$manifest+'"')) 'prepared-crash-h7-r20'
 $deadline=[DateTime]::UtcNow.AddSeconds(45)
 while([DateTime]::UtcNow -lt $deadline){
  if((Test-Path (Join-Path $evidence 'browser-toidentifier-revoke-r20.json.armed.json')) -and (Test-Path (Join-Path $evidence 'prepared-crash-h7-r20.json'))){break}
  if($browser.HasExited -or $monitor.HasExited){throw 'Live event/recovery helpers exited before arming.'}
  Start-Sleep -Milliseconds 100
 }
 if(-not(Test-Path (Join-Path $evidence 'browser-toidentifier-revoke-r20.json.armed.json'))){throw 'Actual observer browser stream did not arm.'}
 & $node (Join-Path $PSScriptRoot 'revoke-acceptance-package.js') 'C:/Users/samue/CREG-backups/phase-c-candidate-r7/publisher/publisher-ed25519.key' (Join-Path $evidence 'next-package/signed-publish-toidentifier-r7.json') 'http://127.0.0.1:18080' > (Join-Path $evidence 'next-package/toidentifier-revocation-r20.txt') 2>&1
 if($LASTEXITCODE -ne 0){throw 'Actual chain-bound owner revocation failed.'}
 $report.revocation_submitted_at_utc=[DateTime]::UtcNow.ToString('o');Save-Report
 Write-Output 'Actual owner revocation submitted once with only V1 and selected primary V2 online.'
 $quorumDeadline=[DateTime]::UtcNow.AddSeconds(90)
 while([DateTime]::UtcNow -lt $quorumDeadline){
  $s=Invoke-RestMethod http://127.0.0.1:18080/v1/public/chain/stats -TimeoutSec 8
  $o=Invoke-RestMethod http://127.0.0.1:18081/v1/public/chain/stats -TimeoutSec 8
  $p=Invoke-RestMethod http://127.0.0.1:18080/v1/consensus/pbft -TimeoutSec 8
  $report.samples+=@{captured_at_utc=[DateTime]::UtcNow.ToString('o');validator_tip=$s.tip_height;observer_tip=$o.tip_height;validator_tip_hash=$s.tip_hash;pbft=$p};Save-Report
  if($s.tip_height -ne 6 -or $o.tip_height -ne 6){throw 'Quorum-loss invariant failed: new finality with two validators online.'}
  Start-Sleep -Seconds 5
 }
 $v2=Cloud v2 "sudo docker exec creg-phase-c-r7-v2 curl -fsS http://127.0.0.1:8080/v1/public/chain/stats; sudo docker exec creg-phase-c-r7-v2 curl -fsS http://127.0.0.1:8080/v1/consensus/pbft; sudo docker exec creg-phase-c-r7-v2 curl -fsS http://127.0.0.1:8080/metrics | grep -E '^creg_durable_finalized_work_depth '"
 $report.primary_v2_before_restore=$v2
 $report.checks=@{two_online_refused_finality=@($report.samples|Where-Object{$_.validator_tip -ne 6 -or $_.observer_tip -ne 6}).Count -eq 0;real_proposal_observed=@($report.samples|Where-Object{@($_.pbft.rounds|Where-Object block_height -eq 7).Count -gt 0}).Count -gt 0};Save-Report
 $v3=Cloud v3 'sudo docker start creg-phase-c-r7-v3 >/dev/null && sudo docker inspect creg-phase-c-r7-v3 --format "{{.State.Status}}"'
 if($v3 -ne 'running'){throw 'V3 restore failed'}
 $mustRestore.Remove('v3')|Out-Null;$report.restored_nodes+=@{slot='v3';at_utc=[DateTime]::UtcNow.ToString('o');state=$v3};Save-Report
 Write-Output 'No h7 finalized during the real two-validator window; V3 restored to supply a quorum.'
 $recoveryDeadline=[DateTime]::UtcNow.AddMinutes(5)
 while([DateTime]::UtcNow -lt $recoveryDeadline -and -not $monitor.HasExited){Start-Sleep -Seconds 2}
 if(-not $monitor.HasExited){throw 'Prepared crash monitor did not finish within its recovery bound.'}
 $crash=Get-Content (Join-Path $evidence 'prepared-crash-h7-r20.json') -Raw|ConvertFrom-Json
 $report.prepared_recovery=$crash;$report.checks.prepared_commit_crash_recovered=$crash.passed
 $v4=Cloud v4 'sudo docker start creg-phase-c-r7-v4 >/dev/null && sudo docker inspect creg-phase-c-r7-v4 --format "{{.State.Status}}"'
 if($v4 -ne 'running'){throw 'V4 restore failed'}
 $mustRestore.Remove('v4')|Out-Null;$report.restored_nodes+=@{slot='v4';at_utc=[DateTime]::UtcNow.ToString('o');state=$v4}
 $report.completed_at_utc=[DateTime]::UtcNow.ToString('o');$report.passed=@($report.checks.Values|Where-Object{$_ -ne $true}).Count -eq 0;Save-Report
 if(-not $report.passed){throw 'One or more real recovery gates remain unaccepted; inspect retained evidence.'}
 Write-Output 'Real two-validator refusal and prepared/local-COMMIT crash recovery passed; V3 and V4 restored.'
} catch {
 $report.error=$_.Exception.Message;Save-Report;throw
} finally {
 foreach($slot in $mustRestore){try{Cloud $slot "sudo docker start creg-phase-c-r7-$slot >/dev/null"|Out-Null}catch{Write-Warning "$slot will also be restored by its independent host timer."}}
 foreach($slot in @('v3','v4')){try{Cloud $slot "sudo systemctl stop creg-r20-$slot-drill-restore.timer >/dev/null 2>&1 || true"|Out-Null}catch{}}
}
