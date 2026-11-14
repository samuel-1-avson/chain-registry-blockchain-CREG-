param([Parameter(Mandatory=$true)][string]$ReviewedImageId)
$ErrorActionPreference='Stop'
$candidate='creg-phase-c-r7-v1'
$backup='creg-phase-c-r7-v1-before-explorer-r11'
$image='creg-node:phase-c-secure-r11-explorer'
$private='C:/Users/samue/CREG-backups/phase-c-candidate-r7/v1'
$data='C:/Users/samue/.creg-phase-c/r7/v1/data'
$scanner='C:/Users/samue/CREG-backups/phase-c-scanner/bundle-r9'
$expectedGenesis='0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$expectedTip='0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$expectedProfile='648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$build=Get-Content (Join-Path $root 'docs/testnet-heal-evidence/2026-09-26-explorer-node-review-r11/image-build.json') -Raw|ConvertFrom-Json
if($build.image_id -ne $ReviewedImageId){throw 'Build evidence does not match requested image'}
$actual=& docker image inspect $image --format '{{.Id}}'
if($LASTEXITCODE -ne 0 -or $actual.Trim() -ne $ReviewedImageId){throw 'Image is not the reviewed build'}
$fleet=Get-Content (Join-Path $PSScriptRoot 'fleet-before-node-update.json') -Raw|ConvertFrom-Json
if(@($fleet.nodes).Count -ne 8 -or @($fleet.nodes|Where-Object{-not $_.ready}).Count -ne 0){throw 'All eight real nodes must pass the readiness check'}
$envPath=Join-Path $private 'node.env'
if(-not(Test-Path $envPath) -or -not(Test-Path $data) -or -not(Test-Path $scanner)){throw 'Existing data/environment/scanner missing'}
$stats=Invoke-RestMethod http://127.0.0.1:18080/v1/public/chain/stats -TimeoutSec 15
if($stats.genesis_hash -ne $expectedGenesis -or $stats.tip_hash -ne $expectedTip){throw 'Unexpected chain state before update'}
$active=Invoke-RestMethod http://127.0.0.1:18080/v1/validator/consensus/state -TimeoutSec 15
if(@($active.active_rounds).Count -ne 0){throw 'Wait until the local round is idle'}
$previous=@(& docker inspect $candidate)|ConvertFrom-Json
if($LASTEXITCODE -ne 0){throw 'Original candidate is missing'}
if($previous[0].Config.Image -ne 'creg-node:phase-c-secure-r9-review'){throw 'Unexpected existing image'}
if(@($previous[0].Mounts|Where-Object{$_.Destination -eq '/data'}).Count -ne 1){throw 'Expected original data bind mount'}
$existingBackup=& docker inspect $backup --format '{{.Id}}' 2>$null
if($LASTEXITCODE -eq 0){throw 'Update backup already exists; inspect first'}
$switched=$false
try {
  & docker stop $candidate|Out-Null
  if($LASTEXITCODE -ne 0){throw 'Could not stop original candidate'}
  & docker rename $candidate $backup
  if($LASTEXITCODE -ne 0){& docker start $candidate|Out-Null;throw 'Could not preserve original container'}
  $switched=$true
  & docker run -d --name $candidate --restart unless-stopped --privileged --env-file $envPath `
    -p 18080:8080 -p 9001:9000 --mount "type=bind,source=$data,target=/data" `
    --mount "type=bind,source=$scanner,target=/scanner,readonly" `
    -v /var/run/docker.sock:/var/run/docker.sock $ReviewedImageId|Out-Null
  if($LASTEXITCODE -ne 0){throw 'Could not start reviewed candidate'}
  $confirmed=$false
  for($attempt=0;$attempt -lt 24;$attempt++) {
    try {
      $stats=Invoke-RestMethod http://127.0.0.1:18080/v1/public/chain/stats -TimeoutSec 5
      $health=Invoke-RestMethod http://127.0.0.1:18080/v1/public/health -TimeoutSec 5
      if($stats.genesis_hash -eq $expectedGenesis -and $stats.tip_hash -eq $expectedTip -and $stats.validator_count -eq 4 -and $stats.finalization_source -eq 'creg-canonical-store' -and $stats.finalized_height -eq 4 -and $health.sandbox.isolated -eq $true -and $health.sandbox.dev_bypass -eq $false) {$confirmed=$true;break}
    } catch {}
    Start-Sleep -Seconds 2
  }
  if(-not $confirmed){throw 'Real API/state checks failed after update'}
  $metrics=(Invoke-WebRequest http://127.0.0.1:18080/metrics -TimeoutSec 10).Content
  if($metrics -notmatch 'creg_scanner_profile_ready 1' -or $metrics -notmatch $expectedProfile){throw 'Scanner readiness/profile changed'}
  [ordered]@{captured_at_utc=[DateTime]::UtcNow.ToString('o');image_id=$ReviewedImageId;slot='v1';stats=$stats;health=$health;preserved_data=$true;preserved_signing_journals=$true;public_cutover=$false}|ConvertTo-Json -Depth 10|Set-Content (Join-Path $PSScriptRoot 'local-node-update.json') -Encoding utf8
  'Updated real V1; its existing data, signing journals, scanner and chain hashes were preserved.'
} catch {
  if($switched) {
    $current=& docker inspect $candidate --format '{{.Image}}' 2>$null
    if($LASTEXITCODE -eq 0) {
      if($current.Trim() -ne $ReviewedImageId){throw 'Unexpected container identity during rollback'}
      & docker rm -f $candidate|Out-Null
    }
    & docker rename $backup $candidate
    & docker start $candidate|Out-Null
  }
  throw
}
