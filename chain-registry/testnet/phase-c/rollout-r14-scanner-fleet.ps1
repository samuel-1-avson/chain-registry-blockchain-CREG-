param(
    [ValidateSet('o1','o2','o3','o4','v2','v3','v4')][string[]]$Slots = @('o1','o2','o3','o4','v2','v3','v4'),
    [string]$EvidenceDirectory = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-live-rollout')
)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7+ is required.' }
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$pipelineEvidence = Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover'
$packageEvidence = Join-Path $pipelineEvidence 'next-real-package-w3-h19'
$bundle = Join-Path $pipelineEvidence 'scanner-r14-ansi-regex-bundle'
$baselinePath = Join-Path $pipelineEvidence 'next-real-package-w3-h17/fleet-readiness-current-20260928.json'
$absencePath = Join-Path $packageEvidence 'package-absence-pre-r14-clean.json'
$helper = Join-Path $PSScriptRoot 'rollout-r14-scanner-linux-node.sh'
$o1Postflight = Join-Path $PSScriptRoot 'ensure-r7-o1-data-pipes.sh'
$profile = '7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd'
$epoch = 'osv-2026-09-28-phase-c-ansi-regex-r14-live'
$canonical = 'npm:ansi-regex@6.1.0'
$chainId = 'creg-phase-c-testnet-2026-09-26-r7'
$tipHeight = 18
$tipHash = 'ec64471b8f9c58eb2d9b958db765a98871cb444e032143d313d65b3ab80cd76c'
$oldProfile = '8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e'
$snapshotSha = '5defc864740b53d9a7a2085d8ace2d972922638a836444d2b5c2abed599e543e'
$manifestSha = 'e07463e38d3301a3170b9eaec9fd94c6062b1a9775367a5eaf663a2dad3ae012'
$threatSha = 'c97381b9df48722a1f591c8f81c87e4d9695c0b89877e6b609e883abf7e4a723'
$yaraFileSha = '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
$slotsOrder = @('o1','o2','o3','o4','v2','v3','v4')
$fleet = @(
    @{ slot='v1'; role='validator'; name='operator laptop'; port=18080 },
    @{ slot='v2'; role='validator'; name='creg-validator-1'; project='gen-lang-client-0022105784'; zone='us-east1-b'; port=18080 },
    @{ slot='v3'; role='validator'; name='creg-validator-3'; project='fiffi-500411'; zone='europe-west1-b'; port=8080 },
    @{ slot='v4'; role='validator'; name='creg-validator-4'; project='fiffi-500411'; zone='asia-southeast1-b'; port=8080 },
    @{ slot='o1'; role='observer'; name='creg-testnet-vm'; project='gen-lang-client-0022105784'; zone='us-central1-b'; port=18080 },
    @{ slot='o2'; role='observer'; name='creg-observer-vm'; project='gen-lang-client-0022105784'; zone='us-central1-a'; port=8080 },
    @{ slot='o3'; role='observer'; name='creg-observer-3'; project='fiffi-500411'; zone='us-west1-b'; port=8080 },
    @{ slot='o4'; role='observer'; name='creg-observer-4'; project='fiffi-500411'; zone='africa-south1-a'; port=8080 }
)
if (@($Slots | Select-Object -Unique).Count -ne $Slots.Count -or @($Slots | Where-Object { $_ -notin $slotsOrder }).Count -gt 0) {
    throw 'Remote rollout slots must be unique.'
}
$Slots = @($slotsOrder | Where-Object { $_ -in $Slots })
if (-not $Slots.Count) { throw 'Select at least one remote R7 node.' }

foreach ($pair in @(
    @('osv_snapshot.json',$snapshotSha), @('manifest.json',$manifestSha),
    @('known_malicious_hashes.json',$threatSha), @('rules/supply_chain.yar',$yaraFileSha)
)) {
    $file = Join-Path $bundle $pair[0]
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $pair[1]) { throw "R14 input hash mismatch: $($pair[0])" }
}
$manifest = Get-Content (Join-Path $bundle 'manifest.json') -Raw | ConvertFrom-Json
$snapshot = Get-Content (Join-Path $bundle 'osv_snapshot.json') -Raw | ConvertFrom-Json
$packageReview = Get-Content (Join-Path $packageEvidence 'review.json') -Raw | ConvertFrom-Json
if ($manifest.scanner_profile_digest -ne $profile -or $manifest.osv_epoch -ne $epoch -or
    $snapshot.epoch -ne $epoch -or $null -eq $snapshot.entries.'npm:ansi-regex@6.1.0' -or
    $packageReview.canonical -ne $canonical -or $packageReview.package_executed_or_published -ne $false) {
    throw 'Prepared R14 scanner/package review does not match the rollout.'
}
if (-not (Test-Path -LiteralPath $helper) -or -not (Test-Path -LiteralPath $o1Postflight)) { throw 'Reviewed rollout/postflight helper is missing.' }
$baseline = Get-Content $baselinePath -Raw | ConvertFrom-Json
$absence = Get-Content $absencePath -Raw | ConvertFrom-Json
if (-not $baseline.all_eight_ready -or -not $baseline.all_eight_recovery_clear -or -not $baseline.all_eight_pbft_clear -or
    $baseline.nodes.Count -ne 8 -or -not $absence.passed -or -not $absence.all_eight_absent -or
    -not $absence.all_eight_same_tip -or -not $absence.all_eight_profile_ready -or
    $absence.expected_height -ne $tipHeight -or $absence.expected_hash -ne $tipHash) {
    throw 'All-eight live h18 readiness and package-absence evidence is required.'
}
$tipGroups = @($baseline.nodes | Group-Object { "$($_.stats.tip_height):$($_.stats.tip_hash)" })
if ($tipGroups.Count -ne 1 -or $tipGroups[0].Name -ne "$tipHeight`:$tipHash") { throw 'The all-eight readiness capture no longer shows the exact shared h18 tip.' }
foreach ($node in $baseline.nodes) {
    if ($node.stats.chain_id -ne $chainId -or -not $node.stats.spec_signature_verified -or
        $node.stats.active_validators -ne 4 -or $node.stats.validator_count -ne 4 -or
        -not $node.recovery_clear -or -not $node.pbft_clear -or
        [regex]::Match([string]$node.metrics,'profile_digest="([0-9a-f]{64})"').Groups[1].Value -ne $oldProfile) {
        throw "The baseline node $($node.slot) is no longer on the reviewed R13/R7 state."
    }
}

$bundleDirectory = Join-Path $EvidenceDirectory ([DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'))
New-Item -ItemType Directory -Path $bundleDirectory -Force | Out-Null
$runId = Split-Path $bundleDirectory -Leaf
$scriptHash = (Get-FileHash $helper -Algorithm SHA256).Hash.ToLowerInvariant()
$reports = [System.Collections.Generic.List[object]]::new()
$progressPath = Join-Path $bundleDirectory 'progress.json'
$progress = [ordered]@{
    schema='creg-real-r14-scanner-fleet-rollout-v1'; run_id=$runId
    started_at_utc=[DateTime]::UtcNow.ToString('o'); chain_id=$chainId
    starting_height=$tipHeight; starting_hash=$tipHash; canonical=$canonical
    old_profile=$oldProfile; new_profile=$profile; requested_slots=$Slots
    nodes=@(); remote_rollout_complete=$false; requested_run_complete=$false
    v1_pending_local_docker_recovery=$false; v1_scanner_rollout_pending=$true
    full_fleet_acceptance=$false; public_cutover=$false
}

$validatorProbe = @'
python3 - <<'PY'
import json,re,urllib.request
base='http://127.0.0.1:PORT'
def get(path):
 with urllib.request.urlopen(base+path,timeout=12) as r:return json.load(r) if path!='/metrics' else r.read().decode()
stats=get('/v1/public/chain/stats');health=get('/v1/public/health');pbft=get('/v1/consensus/pbft')
with urllib.request.urlopen(base+'/metrics',timeout=12) as r:metrics=r.read().decode()
def metric(name):
 m=re.search(r'^'+re.escape(name)+r'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+))$',metrics,re.M);return float(m.group(1)) if m else None
unresolved=[x for x in pbft.get('rounds',[]) if x.get('phase')!='finalised' and (x.get('phase')!='failed' or x.get('prepare_count',0)>=x.get('quorum',3))]
checks={'health':health.get('status')=='ok' and health.get('node_id')=='NODE_ID' and health.get('is_validator') is True,'signed_chain':stats.get('chain_id')=='creg-phase-c-testnet-2026-09-26-r7' and stats.get('spec_signature_verified') is True,'shared_tip':stats.get('tip_height')==stats.get('finalized_height')==18 and stats.get('tip_hash','').removeprefix('0x')=='ec64471b8f9c58eb2d9b958db765a98871cb444e032143d313d65b3ab80cd76c','four_active_validators':stats.get('validator_count')==stats.get('active_validators')==4,'peer_connected':stats.get('peer_count',0)>0,'queues_clear':all(metric(n)==0 for n in ('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')),'pbft_clear':not unresolved}
print(json.dumps({'node_id':health.get('node_id'),'stats':stats,'checks':checks,'ready':all(checks.values())}))
PY
'@

function Get-ValidatorState($node) {
    if ($node.slot -eq 'v1') {
        $base = "http://127.0.0.1:$($node.port)"
        $stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 12
        $health = Invoke-RestMethod -Uri "$base/v1/public/health" -TimeoutSec 12
        $pbft = Invoke-RestMethod -Uri "$base/v1/consensus/pbft" -TimeoutSec 12
        $metrics = (Invoke-WebRequest -Uri "$base/metrics" -TimeoutSec 12).Content
        $unresolved = @($pbft.rounds | Where-Object { $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum) })
        $metricValues = @{}
        foreach ($name in @('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')) {
            $m = [regex]::Match($metrics,'(?m)^'+[regex]::Escape($name)+'(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+))$')
            $metricValues[$name] = if ($m.Success) { [double]$m.Groups[1].Value } else { $null }
        }
        $checks = [ordered]@{
            identity = $health.status -eq 'ok' -and $health.node_id -eq 'validator-1' -and $health.is_validator -eq $true
            signed_chain = $stats.chain_id -eq $chainId -and $stats.spec_signature_verified -eq $true
            shared_tip = $stats.tip_height -eq $tipHeight -and $stats.finalized_height -eq $tipHeight -and ([string]$stats.tip_hash).ToLowerInvariant().Replace('0x','') -eq $tipHash
            four_active_validators = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
            peer_connected = $stats.peer_count -gt 0
            queues_clear = @($metricValues.Values | Where-Object { $_ -ne 0 }).Count -eq 0
            pbft_clear = $unresolved.Count -eq 0
        }
        return [pscustomobject]@{ slot='v1'; node_id=$health.node_id; checks=$checks; ready=(@($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0) }
    }
    $command = $validatorProbe.Replace('PORT',[string]$node.port).Replace('NODE_ID',"validator-$($node.slot.Substring(1))")
    $gcp = @("--project=$($node.project)","--zone=$($node.zone)",'--tunnel-through-iap','--quiet')
    $output = (& gcloud compute ssh $node.name @gcp "--command=$command" 2>&1 | Out-String).Trim()
    $exitCode = $LASTEXITCODE
    try { $state = $output | ConvertFrom-Json } catch { throw "Could not capture $($node.slot) validator health (exit $exitCode): $output" }
    if (-not $state.ready) { throw "$($node.slot) validator preflight failed: $($state.checks | ConvertTo-Json -Compress)" }
    return [pscustomobject]@{ slot=$node.slot; node_id=$state.node_id; checks=$state.checks; ready=$state.ready }
}
function Assert-Three-Other-Validators($targetSlot) {
    $states = @()
    foreach ($node in @($fleet | Where-Object { $_.role -eq 'validator' })) {
        try { $states += Get-ValidatorState $node }
        catch { $states += [pscustomobject]@{slot=$node.slot;node_id=$null;checks=@{};ready=$false;error=$_.Exception.Message} }
    }
    $preFile = Join-Path $bundleDirectory "pre-$targetSlot-validators.json"
    $states | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $preFile -Encoding utf8
    $bad = @($states | Where-Object { -not $_.ready })
    if ($states.Count -ne 4 -or $bad.Count -gt 0) {
        $details = @($bad | ForEach-Object { $_.slot + '=' + ($_.checks | ConvertTo-Json -Compress) + ' ' + $_.error }) -join '; '
        throw "Validator readiness failed before ${targetSlot}: $details"
    }
    $others = @($states | Where-Object { $_.slot -ne $targetSlot -and $_.ready })
    if ($others.Count -ne 3) { throw "Updating $targetSlot would leave fewer than three active validators." }
    return $states
}
function Save-Progress {
    $progress.nodes = @($reports)
    $progress|ConvertTo-Json -Depth 30|Set-Content -LiteralPath $progressPath -Encoding utf8
}

Save-Progress
foreach ($slot in $Slots) {
    $node = @($fleet | Where-Object { $_.slot -eq $slot })[0]
    if ($node.role -eq 'validator') {
        $preValidators = Assert-Three-Other-Validators $slot
    }
    $gcp = @("--project=$($node.project)","--zone=$($node.zone)",'--tunnel-through-iap','--quiet')
    $container = "creg-phase-c-r7-$slot"
    $image = (& gcloud compute ssh $node.name @gcp "--command=sudo docker inspect '$container' --format '{{.Image}} {{.State.Status}}'" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $image -notmatch '^(sha256:[0-9a-f]{64}) running$') { throw "$slot does not have an inspectable running reviewed R7 container." }
    $imageId = $Matches[1]
    if ($imageId -notin @('sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3','sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9','sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708','sha256:2a553f1cb8b2ea6c63bf99649c57e0291698e3ac01da5de80863760afcc07a70','sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c')) { throw "$slot node image is outside the reviewed R7 canary set." }

    $remoteStage = "/tmp/creg-scanner-r14-$runId-$slot"
    $remoteHelper = "/tmp/rollout-r14-$runId-$slot.sh"
    $createStage = "test ! -e '$remoteStage' && install -d -m 700 '$remoteStage' '$remoteStage/rules'"
    & gcloud compute ssh $node.name @gcp "--command=$createStage" 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Unique R14 staging directory is unavailable on $slot; inspect the host before retrying." }
    & gcloud compute scp (Join-Path $bundle 'osv_snapshot.json') (Join-Path $bundle 'manifest.json') (Join-Path $bundle 'known_malicious_hashes.json') "$($node.name):$remoteStage/" @gcp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not stage the pinned R14 snapshots on $slot." }
    & gcloud compute scp (Join-Path $bundle 'rules/supply_chain.yar') "$($node.name):$remoteStage/rules/" @gcp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not stage the R14 YARA input on $slot." }
    & gcloud compute scp $helper "$($node.name):$remoteHelper" @gcp | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not stage the R14 rollout helper on $slot." }
    $nodeId = "$($node.role)-$($slot.Substring(1))"
    $p2pPort = if ($node.port -eq 18080) { 9001 } else { 9000 }
    $remoteRun = "sed -i 's/\r$//' '$remoteHelper' && echo '$scriptHash  $remoteHelper' | sha256sum -c - >/dev/null && sudo bash '$remoteHelper' '$slot' '$($node.role)' '$($node.port)' '$p2pPort' '$nodeId' '$imageId' '$profile' '$epoch' '$snapshotSha' '$manifestSha' '$threatSha' '$yaraFileSha' '$remoteStage'"
    $resultText = (& gcloud compute ssh $node.name @gcp "--command=$remoteRun" 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw "$slot R14 rollout failed; the helper attempted a local rollback. Output: $resultText" }
    try { $result = $resultText | ConvertFrom-Json } catch { throw "$slot R14 rollout returned invalid postflight JSON: $resultText" }
    if (-not $result.passed -or $result.scanner_profile_digest -ne $profile -or $result.height -ne $tipHeight -or $result.block_hash -ne $tipHash) { throw "$slot R14 postflight did not match the reviewed h18 candidate." }
    $reportPath = Join-Path $bundleDirectory "$slot-rollout.json"
    $result | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $reportPath -Encoding utf8
    $reports.Add([ordered]@{slot=$slot;role=$node.role;host=$node.name;image_id=$imageId;result='passed';report=(Split-Path $reportPath -Leaf)})
    Save-Progress

    if ($slot -eq 'o1') {
        $pipeText = (& gcloud compute ssh $node.name @gcp '--command=sudo bash /var/lib/creg/phase-c-r7/bin/ensure-r7-o1-data-pipes.sh' 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "O1 R26/history/private-explorer postflight failed: $pipeText" }
        try { $pipe = $pipeText | ConvertFrom-Json } catch { throw "O1 data-pipe helper returned invalid JSON: $pipeText" }
        if ($pipe.status -ne 'passed' -or -not $pipe.checks.real_history -or -not $pipe.checks.private_explorer_same_tip) { throw 'O1 history and private explorer must match R7 h18 after restart.' }
        $pipe | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $bundleDirectory 'o1-data-pipes-postflight.json') -Encoding utf8
    }
    Write-Output "${slot}: R14 profile deployed; node image/data/identity were preserved and h18 postflight passed."
}
$progress.requested_run_complete = $true
$remoteSlots = @('o1','o2','o3','o4','v2','v3','v4')
$progress.remote_rollout_complete = @($reports | Where-Object { $_.slot -in $remoteSlots }).Count -eq $remoteSlots.Count
$progress.completed_at_utc = [DateTime]::UtcNow.ToString('o')
$progress.remote_nodes = @($reports | ForEach-Object { $_.slot })
$progress|ConvertTo-Json -Depth 30|Set-Content -LiteralPath $progressPath -Encoding utf8
Write-Output "Requested remote R14 scanner rollout passed for $($Slots -join ', '). V1 remains R13 until its local scanner rollout passes; do not publish until all eight report R14. Evidence: $progressPath"
