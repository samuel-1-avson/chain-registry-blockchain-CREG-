param(
    [string]$EvidenceRoot = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9'),
    [string]$ExpectedProfileDigest = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
)

$ErrorActionPreference = 'Stop'
$quorum = Get-Content -LiteralPath (Join-Path $EvidenceRoot 'blocks-after-revoke/quorum-check.json') -Raw | ConvertFrom-Json
if (-not $quorum.shared_finality -or $quorum.node_count -ne 8 -or $quorum.quorum -ne 3) { throw 'All eight real hosts must have verified shared revocation finality before this restart drill.' }
$fleet = Get-Content -LiteralPath (Join-Path $EvidenceRoot 'fleet-runtime-readiness.json') -Raw | ConvertFrom-Json
$expectedTip = '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$expectedProfile = $ExpectedProfileDigest
$evidencePath = Join-Path $EvidenceRoot 'fleet-committed-restart.json'
if (Test-Path -LiteralPath $evidencePath) { throw 'Restart evidence already exists; preserve this drill before starting another.' }
$template = @'
python3 - <<'PY'
import datetime, json, subprocess, time, urllib.request
base = 'http://127.0.0.1:PORT'
container = 'creg-phase-c-r7-SLOT'
def get(path):
    with urllib.request.urlopen(base + path, timeout=3) as response:
        return response.read().decode('utf8')
def started():
    return subprocess.run(['sudo', 'docker', 'inspect', container, '--format', '{{.State.StartedAt}}'], check=True, capture_output=True, text=True).stdout.strip()
before = json.loads(get('/v1/public/chain/stats'))
if before['tip_height'] != 4 or before['tip_hash'] != 'EXPECTED_TIP' or before['chain_id'] != 'creg-phase-c-testnet-2026-09-26-r7':
    raise RuntimeError('Unexpected pre-restart chain state')
before_started = started()
subprocess.run(['sudo', 'docker', 'restart', '--time', '20', container], check=True, capture_output=True, text=True)
record = {'captured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'before_started_at': before_started, 'after_started_at': started(), 'before_tip_hash': before['tip_hash']}
for attempt in range(60):
    try:
        stats = json.loads(get('/v1/public/chain/stats'))
        package = json.loads(get('/v1/public/packages/npm%3Ais-number%407.0.0'))
        health = json.loads(get('/v1/public/health'))
        metrics = get('/metrics')
        pbft = json.loads(get('/v1/consensus/pbft'))
        checks = {
            'process_restarted': record['before_started_at'] != record['after_started_at'],
            'same_tip': stats['tip_height'] == 4 and stats['tip_hash'] == 'EXPECTED_TIP',
            'same_chain': stats['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7' and stats['spec_signature_verified'],
            'same_package_state': package['status'] == 'revoked' and package['block_phase'] == 'committed',
            'four_validators': stats['active_validators'] == 4 and stats['validator_count'] == 4,
            'live_peers': stats['peer_count'] > 0,
            'scanner_ready': 'creg_scanner_profile_ready 1\n' in metrics and 'profile_digest="EXPECTED_PROFILE"' in metrics,
            'retired_queue_empty': 'creg_pending_block_txs 0\n' in metrics and 'creg_durable_finalized_work_depth 0\n' in metrics and 'creg_durable_finalized_work_ready_depth 0\n' in metrics and 'creg_forced_inclusion_pending 0\n' in metrics,
            'no_unfinished_pbft_round': all(r['phase'] in ('finalised', 'failed') for r in pbft['rounds']),
        }
        if 'ROLE' == 'validator':
            checks['isolated_sandbox'] = health['sandbox']['engine'] == 'nsjail' and health['sandbox']['isolated'] and not health['sandbox']['dev_bypass']
        record.update({'stats':stats, 'package_state':{k:package.get(k) for k in ['canonical','status','block_phase','block_hash','revocation_reason']}, 'health':health, 'checks':checks, 'passed':all(checks.values())})
        if record['passed']:
            break
    except Exception as error:
        record['last_capture_error'] = str(error)
    time.sleep(2)
print(json.dumps(record))
PY
'@
$records = @()
foreach ($item in $fleet.nodes | Sort-Object slot) {
    if ($item.slot -eq 'v1') {
        $base = 'http://127.0.0.1:18080'
        $before = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 10
        if ($before.tip_height -ne 4 -or $before.tip_hash -ne $expectedTip) { throw 'Unexpected local pre-restart state.' }
        $beforeStarted = (& docker inspect 'creg-phase-c-r7-v1' --format '{{.State.StartedAt}}' | Out-String).Trim()
        & docker restart --time 20 'creg-phase-c-r7-v1' | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Local real process restart failed.' }
        $afterStarted = (& docker inspect 'creg-phase-c-r7-v1' --format '{{.State.StartedAt}}' | Out-String).Trim()
        $record = @{ captured_at_utc = [DateTime]::UtcNow.ToString('o'); before_started_at = $beforeStarted; after_started_at = $afterStarted; before_tip_hash = $before.tip_hash; passed = $false }
        for ($attempt = 0; $attempt -lt 60; $attempt++) {
            try {
                $stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 3
                $package = Invoke-RestMethod -Uri "$base/v1/public/packages/npm%3Ais-number%407.0.0" -TimeoutSec 3
                $health = Invoke-RestMethod -Uri "$base/v1/public/health" -TimeoutSec 3
                $metrics = (Invoke-WebRequest -Uri "$base/metrics" -TimeoutSec 3).Content
                $pbft = Invoke-RestMethod -Uri "$base/v1/consensus/pbft" -TimeoutSec 3
                $checks = [ordered]@{
                    process_restarted = $beforeStarted -ne $afterStarted
                    same_tip = $stats.tip_height -eq 4 -and $stats.tip_hash -eq $expectedTip
                    same_chain = $stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats.spec_signature_verified
                    same_package_state = $package.status -eq 'revoked' -and $package.block_phase -eq 'committed'
                    four_validators = $stats.active_validators -eq 4 -and $stats.validator_count -eq 4
                    live_peers = $stats.peer_count -gt 0
                    scanner_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($expectedProfile)
                    retired_queue_empty = $metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and
                        $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and
                        $metrics -match '(?m)^creg_durable_finalized_work_ready_depth 0\s*$' -and
                        $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
                    no_unfinished_pbft_round = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') }).Count -eq 0
                    isolated_sandbox = $health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -and -not $health.sandbox.dev_bypass
                }
                $record.stats = $stats
                $record.package_state = $package | Select-Object canonical, status, block_phase, block_hash, revocation_reason
                $record.health = $health
                $record.checks = $checks
                $record.passed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
                if ($record.passed) { break }
            } catch { $record.last_capture_error = $_.Exception.Message }
            Start-Sleep -Seconds 2
        }
    } else {
        $command = $template.Replace('PORT', [string]$item.port).Replace('SLOT', $item.slot).Replace('ROLE', $item.role).Replace('EXPECTED_TIP', $expectedTip).Replace('EXPECTED_PROFILE', $expectedProfile)
        $output = (& gcloud compute ssh $item.name "--project=$($item.project)" "--zone=$($item.zone)" --tunnel-through-iap --quiet "--command=$command" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Real restart/capture failed for $($item.slot); inspect this host before repeating." }
        $record = $output | ConvertFrom-Json -AsHashtable
    }
    $record.slot = $item.slot
    $records += $record
    $evidence = [ordered]@{ schema = 'creg-real-fleet-committed-restart-v1'; captured_at_utc = [DateTime]::UtcNow.ToString('o'); all_eight_passed = $records.Count -eq 8 -and @($records | Where-Object { -not $_.passed }).Count -eq 0; prepared_inflight_restart_tested = $false; nodes = $records }
    $evidence | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $evidencePath -Encoding utf8
    "$($item.slot): real restart passed=$($record.passed)"
    if (-not $record.passed) { throw 'Committed-state restart checks failed; inspect evidence before more mutations.' }
}
