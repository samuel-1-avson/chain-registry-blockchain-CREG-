param(
    [string]$ReadinessEvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/fleet-r7-h12-after-v1-recovery.json'),
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-events-r16/r21-rollout/fleet-r21-h12-container-restart-recovery.json')
)

$ErrorActionPreference = 'Stop'
$expectedChain = 'creg-phase-c-testnet-2026-09-26-r7'
$expectedGenesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$expectedTip = 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d'
$profile = 'd09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$acceptedImages = @(
    'sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423',
    'sha256:d2b83a6525314b17434bc98c5bf05181db4fd6d73c962eabd2b5407968dfa95c',
    'sha256:60ff1bc767ac84ae119a35b2ac864ebe08e0e4978a788315c3759cafe7d827fd'
)

$readiness = Get-Content -LiteralPath $ReadinessEvidencePath -Raw | ConvertFrom-Json
if (-not $readiness.all_eight_ready -or -not $readiness.all_eight_recovery_clear -or -not $readiness.all_eight_pbft_clear) {
    throw 'The latest eight-node readiness, queue, and PBFT preflight did not pass.'
}
if (@($readiness.nodes | Where-Object { $_.stats.tip_height -ne 12 -or $_.stats.tip_hash -ne $expectedTip }).Count -ne 0) {
    throw 'All eight readiness records must match finalized h12 before any restart.'
}

$fleet = @(
    @{ Slot='v1'; Role='validator'; Name='operator laptop'; Port=18080 },
    @{ Slot='v2'; Role='validator'; Name='creg-validator-1'; Project='gen-lang-client-0022105784'; Zone='us-east1-b'; Port=18080 },
    @{ Slot='v3'; Role='validator'; Name='creg-validator-3'; Project='fiffi-500411'; Zone='europe-west1-b'; Port=8080 },
    @{ Slot='v4'; Role='validator'; Name='creg-validator-4'; Project='fiffi-500411'; Zone='asia-southeast1-b'; Port=8080 },
    @{ Slot='o1'; Role='observer'; Name='creg-testnet-vm'; Project='gen-lang-client-0022105784'; Zone='us-central1-b'; Port=18080 },
    @{ Slot='o2'; Role='observer'; Name='creg-observer-vm'; Project='gen-lang-client-0022105784'; Zone='us-central1-a'; Port=8080 },
    @{ Slot='o3'; Role='observer'; Name='creg-observer-3'; Project='fiffi-500411'; Zone='us-west1-b'; Port=8080 },
    @{ Slot='o4'; Role='observer'; Name='creg-observer-4'; Project='fiffi-500411'; Zone='africa-south1-a'; Port=8080 }
)

$remoteProgram = @'
import hashlib, json, pathlib, re, subprocess, sys, time, urllib.request
container = '__CONTAINER__'
base = 'http://127.0.0.1:__PORT__'
role = '__ROLE__'
expected_chain = '__CHAIN__'
expected_genesis = '__GENESIS__'
expected_tip = '__TIP__'
profile = '__PROFILE__'
data_dir = pathlib.Path('/var/lib/creg/phase-c-r7/data')
env_path = pathlib.Path('/var/lib/creg/phase-c-r7/node.env')

def get(path):
    with urllib.request.urlopen(base + path, timeout=8) as response:
        return json.loads(response.read().decode('utf8'))

def metric(text, name):
    match = re.search(r'^' + re.escape(name) + r'\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)\s*$', text, re.M)
    return float(match.group(1)) if match else None

def digest(path):
    output = subprocess.run(['sudo', 'sha256sum', str(path)], check=True, capture_output=True, text=True).stdout
    return output.split()[0]

def snapshot():
    inspected = json.loads(subprocess.run(['sudo', 'docker', 'inspect', container], check=True, capture_output=True, text=True).stdout)[0]
    stats = get('/v1/public/chain/stats')
    health = get('/v1/public/health')
    pbft = get('/v1/consensus/pbft')
    block = get('/v1/public/blocks/12')
    package = get('/v1/public/packages/npm%3Ams%402.1.3')
    with urllib.request.urlopen(base + '/metrics', timeout=8) as response:
        metrics = response.read().decode('utf8')
    queue_names = ('creg_pending_block_txs', 'creg_durable_finalized_work_depth', 'creg_durable_finalized_work_ready_depth', 'creg_forced_inclusion_pending')
    queues = {name: metric(metrics, name) for name in queue_names}
    unresolved = [round for round in pbft.get('rounds', []) if round.get('phase') not in ('finalised', 'failed') or (round.get('phase') == 'failed' and round.get('prepare_count', 0) >= round.get('quorum', 3))]
    identity = digest(data_dir / 'p2p' / 'identity.key')
    env_hash = digest(env_path)
    signer_journal_present = subprocess.run(['sudo', 'test', '-d', str(data_dir / 'pbft-safety.rocksdb')], capture_output=True).returncode == 0
    image = inspected['Image']
    checks = {
        'container_running': inspected['State']['Running'] is True,
        'r21_image_pinned': image in __ACCEPTED_IMAGES__,
        'chain_identity_and_signed_spec': stats.get('chain_id') == expected_chain and stats.get('spec_signature_verified') is True,
        'genesis_preserved': stats.get('genesis_hash', '').removeprefix('0x') == expected_genesis,
        'same_finalized_h12': stats.get('tip_height') == 12 and stats.get('tip_hash') == expected_tip and block.get('hash') == expected_tip and block.get('finalized') is True and len(block.get('pbft_signatures', [])) >= 3,
        'revocation_visible': package.get('status') == 'revoked' and package.get('block_phase') == 'committed',
        'four_validator_roster': stats.get('active_validators') == 4 and stats.get('validator_count') == 4,
        'connected_peers': stats.get('peer_count', 0) > 0,
        'correct_role': health.get('is_validator') is (role == 'validator'),
        'scanner_profile_ready': metric(metrics, 'creg_scanner_profile_ready') == 1 and f'profile_digest="{profile}"' in metrics,
        'queues_clear': all(queues[name] == 0 for name in queue_names),
        'pbft_clear': pbft.get('active_round_count') == 0 and not unresolved,
    }
    if role == 'validator':
        sandbox = health.get('sandbox', {})
        checks['isolated_validator_sandbox'] = sandbox.get('engine') == 'nsjail' and sandbox.get('isolated') is True and sandbox.get('dev_bypass') is False
        checks['signer_journal_present'] = signer_journal_present
    return {'image_id': image, 'container_state': {'running': inspected['State']['Running'], 'started_at': inspected['State']['StartedAt'], 'restart_count': inspected['RestartCount']}, 'p2p_identity_sha256': identity, 'node_env_sha256': env_hash, 'stats': stats, 'health': health, 'pbft': pbft, 'queues': queues, 'dead_letter_depth': metric(metrics, 'creg_durable_finalized_work_dead_letter_depth'), 'checks': checks, 'passed': all(checks.values())}

before = snapshot()
record = {'slot': '__SLOT__', 'role': role, 'restart_method': 'Docker container restart; GCP VM remains RUNNING', 'before': before, 'passed': False}
if not before['passed']:
    print(json.dumps(record))
    raise SystemExit(2)
started_before = before['container_state']['started_at']
result = subprocess.run(['sudo', 'docker', 'restart', '--time', '20', container], capture_output=True, text=True)
record['restart_command_exit'] = result.returncode
deadline = time.time() + 150
last_error = None
while time.time() < deadline:
    try:
        after = snapshot()
        after['checks']['process_restarted'] = after['container_state']['started_at'] != started_before
        after['checks']['p2p_identity_preserved'] = after['p2p_identity_sha256'] == before['p2p_identity_sha256']
        after['checks']['node_env_preserved'] = after['node_env_sha256'] == before['node_env_sha256']
        after['passed'] = all(after['checks'].values()) and result.returncode == 0
        record['after'] = after
        if after['passed']:
            break
    except Exception as error:
        last_error = str(error)
    time.sleep(3)
record['last_error'] = last_error
record['passed'] = bool(record.get('after', {}).get('passed'))
if not record['passed']:
    subprocess.run(['sudo', 'docker', 'start', container], capture_output=True, text=True)
print(json.dumps(record))
raise SystemExit(0 if record['passed'] else 1)
'@

function Get-LocalSnapshot {
    param([string]$Container, [string]$Role)
    $base = 'http://127.0.0.1:18080'
    $dockerJson = (& docker inspect $Container | Out-String) | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Local validator container could not be inspected.' }
    $inspection = $dockerJson[0]
    $stats = Invoke-RestMethod "$base/v1/public/chain/stats" -TimeoutSec 8
    $health = Invoke-RestMethod "$base/v1/public/health" -TimeoutSec 8
    $pbft = Invoke-RestMethod "$base/v1/consensus/pbft" -TimeoutSec 8
    $block = Invoke-RestMethod "$base/v1/public/blocks/12" -TimeoutSec 8
    $package = Invoke-RestMethod "$base/v1/public/packages/npm%3Ams%402.1.3" -TimeoutSec 8
    $metrics = (Invoke-WebRequest "$base/metrics" -TimeoutSec 8).Content
    $data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
    $envPath = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1\node.env'
    $identity = (Get-FileHash -LiteralPath (Join-Path $data 'p2p\identity.key') -Algorithm SHA256).Hash.ToLowerInvariant()
    $envHash = (Get-FileHash -LiteralPath $envPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $unresolved = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised','failed') -or ($_.phase -eq 'failed' -and $_.prepare_count -ge $_.quorum) })
    $queueNames = @('creg_pending_block_txs','creg_durable_finalized_work_depth','creg_durable_finalized_work_ready_depth','creg_forced_inclusion_pending')
    $queues = [ordered]@{}
    foreach ($name in $queueNames) {
        $match = [regex]::Match($metrics, '(?m)^' + [regex]::Escape($name) + '\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)\s*$')
        $queues[$name] = if ($match.Success) { [double]$match.Groups[1].Value } else { $null }
    }
    $checks = [ordered]@{
        container_running = $inspection.State.Running -eq $true
        r21_image_pinned = $inspection.Image -in $acceptedImages
        chain_identity_and_signed_spec = $stats.chain_id -eq $expectedChain -and $stats.spec_signature_verified -eq $true
        genesis_preserved = ($stats.genesis_hash -replace '^0x','') -eq $expectedGenesis
        same_finalized_h12 = $stats.tip_height -eq 12 -and $stats.tip_hash -eq $expectedTip -and $block.hash -eq $expectedTip -and $block.finalized -eq $true -and $block.pbft_signatures.Count -ge 3
        revocation_visible = $package.status -eq 'revoked' -and $package.block_phase -eq 'committed'
        four_validator_roster = $stats.active_validators -eq 4 -and $stats.validator_count -eq 4
        connected_peers = $stats.peer_count -gt 0
        correct_role = $health.is_validator -eq ($Role -eq 'validator')
        scanner_profile_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($profile)
        queues_clear = @($queues.Values | Where-Object { $_ -ne 0 }).Count -eq 0
        pbft_clear = $pbft.active_round_count -eq 0 -and $unresolved.Count -eq 0
        isolated_validator_sandbox = $health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -and -not $health.sandbox.dev_bypass
        signer_journal_present = Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
    }
    [ordered]@{ image_id=$inspection.Image; container_state=[ordered]@{running=$inspection.State.Running;started_at=$inspection.State.StartedAt;restart_count=$inspection.RestartCount}; p2p_identity_sha256=$identity; node_env_sha256=$envHash; stats=$stats; health=$health; pbft=$pbft; queues=$queues; dead_letter_depth=$null; checks=$checks; passed=@($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0 }
}

function Write-Evidence {
    param([array]$Records)
    $document = [ordered]@{
        schema = 'creg-real-r21-h12-sequential-container-restart-recovery-v1'
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        expected_chain_id = $expectedChain
        expected_genesis_hash = $expectedGenesis
        expected_tip_height = 12
        expected_tip_hash = $expectedTip
        image_tag = 'creg-node:phase-c-secure-r21-revoke-retirement'
        accepted_runtime_image_ids = $acceptedImages
        readiness_preflight_path = [IO.Path]::GetFullPath($ReadinessEvidencePath)
        no_chain_reset = $true
        public_cutover = $false
        no_two_validator_window = $true
        vm_restart_performed = $false
        nodes_restarted = $Records
        all_eight_passed = $Records.Count -eq 8 -and @($Records | Where-Object { -not $_.passed }).Count -eq 0
    }
    $document | ConvertTo-Json -Depth 50 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
}

$records = @()
if (Test-Path -LiteralPath $EvidencePath) {
    $priorEvidence = Get-Content -LiteralPath $EvidencePath -Raw | ConvertFrom-Json -AsHashtable
    if ($priorEvidence.expected_tip_hash -ne $expectedTip -or $priorEvidence.public_cutover -ne $false) {
        throw 'Existing restart evidence does not match this R7 h12 drill; it was preserved.'
    }
    $records = @($priorEvidence.nodes_restarted)
}
foreach ($node in $fleet) {
    $priorRecord = @($records | Where-Object { $_.slot -eq $node.Slot } | Select-Object -First 1)
    if ($priorRecord.Count -gt 0) {
        if (-not $priorRecord[0].passed) { throw "Prior restart evidence for $($node.Slot) is incomplete; inspect it before continuing." }
        "$($node.Slot): already passed in saved evidence; skipped"
        continue
    }
    if ($node.Role -eq 'validator') {
        $otherValidators = @($fleet | Where-Object { $_.Role -eq 'validator' -and $_.Slot -ne $node.Slot })
        if ($otherValidators.Count -ne 3) { throw 'Validator restart would not leave three other validators online.' }
    }
    if ($node.Slot -eq 'v1') {
        $container = 'creg-phase-c-r7-v1'
        $before = Get-LocalSnapshot -Container $container -Role $node.Role
        if (-not $before.passed) { throw 'Local V1 pre-restart checks failed; no restart was attempted.' }
        $startedBefore = $before.container_state.started_at
        & docker restart --time 20 $container | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Local V1 Docker restart failed.' }
        $deadline = [DateTime]::UtcNow.AddSeconds(150)
        $after = $null
        while ([DateTime]::UtcNow -lt $deadline) {
            try {
                $after = Get-LocalSnapshot -Container $container -Role $node.Role
                $after.checks.process_restarted = $after.container_state.started_at -ne $startedBefore
                $after.checks.p2p_identity_preserved = $after.p2p_identity_sha256 -eq $before.p2p_identity_sha256
                $after.checks.node_env_preserved = $after.node_env_sha256 -eq $before.node_env_sha256
                $after.passed = @($after.checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
                if ($after.passed) { break }
            } catch { $lastError = $_.Exception.Message }
            Start-Sleep -Seconds 3
        }
        $record = [ordered]@{ slot=$node.Slot; role=$node.Role; restart_method='Docker container restart; laptop stayed running'; other_validators_online=@($otherValidators.Slot); before=$before; after=$after; last_error=$lastError; passed=($null -ne $after -and $after.passed) }
    } else {
        $program = $remoteProgram.Replace('__CONTAINER__', "creg-phase-c-r7-$($node.Slot)").Replace('__PORT__', [string]$node.Port).Replace('__ROLE__', $node.Role).Replace('__CHAIN__', $expectedChain).Replace('__GENESIS__', $expectedGenesis).Replace('__TIP__', $expectedTip).Replace('__PROFILE__', $profile).Replace('__SLOT__', $node.Slot).Replace('__ACCEPTED_IMAGES__', (ConvertTo-Json -Compress -InputObject $acceptedImages))
        $command = "python3 - <<'PY'`n$program`nPY"
        $stdoutPath = [IO.Path]::GetTempFileName()
        $stderrPath = [IO.Path]::GetTempFileName()
        & gcloud compute ssh $node.Name "--project=$($node.Project)" "--zone=$($node.Zone)" --tunnel-through-iap --quiet "--command=$command" 1> $stdoutPath 2> $stderrPath
        $sshExit = $LASTEXITCODE
        $stdoutText = Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue
        $stderrText = Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue
        $output = if ($null -eq $stdoutText) { '' } else { $stdoutText.ToString().Trim() }
        $sshError = if ($null -eq $stderrText) { '' } else { $stderrText.ToString().Trim() }
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
        if ([string]::IsNullOrWhiteSpace($output)) {
            throw "Remote restart command for $($node.Slot) returned no JSON (exit=$sshExit): $sshError"
        }
        try { $record = $output | ConvertFrom-Json -AsHashtable }
        catch { throw "Restart evidence from $($node.Slot) was not valid JSON: $($_.Exception.Message); stderr: $sshError" }
        if ($null -eq $record -or $record -isnot [System.Collections.IDictionary]) {
            throw "Restart evidence from $($node.Slot) was not a JSON object; stderr: $sshError"
        }
        if ($sshExit -ne 0 -and -not $record.passed) { $record['ssh_stderr'] = $sshError }
        $record['slot'] = $node.Slot
        $record['role'] = $node.Role
        $record['other_validators_online'] = if ($node.Role -eq 'validator') { @($otherValidators.Slot) } else { @('v1','v2','v3','v4') }
        $vmStatus = (& gcloud compute instances describe $node.Name "--project=$($node.Project)" "--zone=$($node.Zone)" --format='value(status)' 2>$null | Out-String).Trim()
        $record['vm_status_after'] = $vmStatus
        $record['vm_remained_running'] = $vmStatus -eq 'RUNNING'
        $record['passed'] = $record.passed -and $record.vm_remained_running
    }
    $records += $record
    Write-Evidence -Records $records
    "$($node.Slot): committed h12 restart passed=$($record.passed)"
    if (-not $record.passed) { throw "Restart acceptance failed at $($node.Slot); stop and inspect the saved evidence before any further action." }
}

if ($records.Count -ne 8 -or @($records | Where-Object { -not $_.passed }).Count -gt 0) {
    throw 'The eight-node restart acceptance is incomplete.'
}
Write-Output "All eight R21 containers restarted sequentially and recovered finalized h12. Evidence: $EvidencePath"
