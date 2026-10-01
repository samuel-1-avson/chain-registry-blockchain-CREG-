param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9/fleet-runtime-readiness.json'),
    [string]$ScannerProfileDigest = '7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd',
    [switch]$LocalStateFromHealth,
    [ValidateSet('v1', 'v2', 'v3', 'v4', 'o1', 'o2', 'o3', 'o4')] [string[]]$Slots = @()
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'PowerShell 7 or newer is required; run this script with pwsh so JSON parsing fails before any fleet evidence is written.'
}

function Get-PrometheusMetricValue {
    param([string]$Metrics, [string]$Name)
    $pattern = '^' + [regex]::Escape($Name) + '(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$'
    $line = $Metrics -split "`r?`n" | Where-Object { $_ -match $pattern } | Select-Object -First 1
    if (-not $line) { return $null }
    return [double]([regex]::Match($line, $pattern).Groups[1].Value)
}

$expectedProfile = $ScannerProfileDigest
$expectedGenesisBlock = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$gcloudCommand = Get-Command gcloud.cmd -ErrorAction SilentlyContinue
if (-not $gcloudCommand) { $gcloudCommand = Get-Command gcloud -ErrorAction Stop }
$gcloudPath = $gcloudCommand.Source
$fleet = @(
    @{ Slot = 'v1'; Role = 'validator'; Name = 'operator laptop'; Port = 18080 },
    @{ Slot = 'v2'; Role = 'validator'; Name = 'creg-validator-1'; Project = 'gen-lang-client-0022105784'; Zone = 'us-east1-b'; Port = 18080 },
    @{ Slot = 'v3'; Role = 'validator'; Name = 'creg-validator-3'; Project = 'fiffi-500411'; Zone = 'europe-west1-b'; Port = 8080 },
    @{ Slot = 'v4'; Role = 'validator'; Name = 'creg-validator-4'; Project = 'fiffi-500411'; Zone = 'asia-southeast1-b'; Port = 8080 },
    @{ Slot = 'o1'; Role = 'observer'; Name = 'creg-testnet-vm'; Project = 'gen-lang-client-0022105784'; Zone = 'us-central1-b'; Port = 18080 },
    @{ Slot = 'o2'; Role = 'observer'; Name = 'creg-observer-vm'; Project = 'gen-lang-client-0022105784'; Zone = 'us-central1-a'; Port = 8080 },
    @{ Slot = 'o3'; Role = 'observer'; Name = 'creg-observer-3'; Project = 'fiffi-500411'; Zone = 'us-west1-b'; Port = 8080 },
    @{ Slot = 'o4'; Role = 'observer'; Name = 'creg-observer-4'; Project = 'fiffi-500411'; Zone = 'africa-south1-a'; Port = 8080 }
)
$remoteTemplate = @'
python3 - <<'PY'
import datetime, json, subprocess, urllib.request
from urllib.parse import urlsplit
base = 'http://127.0.0.1:PORT'
record = {'captured_at_utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}
try:
    result = subprocess.run(['sudo', 'docker', 'inspect', 'creg-phase-c-r7-SLOT'], check=True, capture_output=True, text=True)
    container = json.loads(result.stdout)[0]
    record['container_state'] = {
        'Status': container.get('State', {}).get('Status'),
        'Running': container.get('State', {}).get('Running') is True,
        'RestartCount': container.get('RestartCount'),
    }
    record['image_id'] = container.get('Image')
    record['image_ref'] = container.get('Config', {}).get('Image')
    labels = container.get('Config', {}).get('Labels') or {}
    record['source_revision'] = labels.get('org.opencontainers.image.revision')
    env = {}
    for entry in container.get('Config', {}).get('Env') or []:
        key, separator, value = entry.partition('=')
        if separator:
            env[key] = value
    rpc_urls = [env.get('CREG_ETH_RPC', '')]
    rpc_urls.extend(env.get('CREG_ETH_RPC_FALLBACKS', '').split(','))
    record['rpc_provider_hosts'] = sorted({
        parsed.hostname.lower()
        for value in rpc_urls if value.strip()
        for parsed in [urlsplit(value.strip())]
        if parsed.hostname
    })
    for field, path in [('health', '/v1/public/health'), ('stats', '/v1/public/chain/stats'), ('metrics', '/metrics'), ('pbft', '/v1/consensus/pbft')]:
        with urllib.request.urlopen(base + path, timeout=15) as response:
            payload = response.read().decode('utf8')
        record[field] = '\n'.join(line for line in payload.splitlines() if line.startswith('creg_')) if field == 'metrics' else json.loads(payload)
except Exception as error:
    record['error'] = str(error)
print(json.dumps(record))
PY
'@
$records = @()
if ($Slots.Count -gt 0) {
    if (-not (Test-Path -LiteralPath $EvidencePath)) { throw 'A partial capture requires an existing full fleet evidence file.' }
    $previous = Get-Content -LiteralPath $EvidencePath -Raw | ConvertFrom-Json -AsHashtable
    $records = @($previous.nodes | Where-Object { $_.slot -notin $Slots })
    $fleet = @($fleet | Where-Object { $_.Slot -in $Slots })
}
foreach ($hostInfo in $fleet) {
    if ($hostInfo.Slot -eq 'v1') {
        $record = @{ captured_at_utc = [DateTime]::UtcNow.ToString('o') }
        try {
            $base = "http://127.0.0.1:$($hostInfo.Port)"
            $record.health = Invoke-RestMethod -Uri "$base/v1/public/health" -TimeoutSec 15
            if ($LocalStateFromHealth) {
                if ($record.health.status -ne 'ok') { throw 'Local API health did not confirm the validator is serving.' }
                $record.container_state = @{ Status = 'running'; Running = $true; RestartCount = $null; source = 'live node health endpoint' }
            } else {
                $inspectJson = (& docker inspect 'creg-phase-c-r7-v1' 2>$null | Out-String).Trim()
                if ($LASTEXITCODE -ne 0) { throw 'Candidate container is not inspectable.' }
                $container = @($inspectJson | ConvertFrom-Json -AsHashtable)[0]
                $record.container_state = $container.State
                $record.container_state.RestartCount = $container.RestartCount
                $record.image_id = $container.Image
                    $record.image_ref = $container.Config.Image
                    $labels = $container.Config.Labels
                    if ($labels) { $record.source_revision = $labels['org.opencontainers.image.revision'] }
                $envMap = @{}
                foreach ($entry in $container.Config.Env) {
                    $separator = $entry.IndexOf('=')
                    if ($separator -gt 0) { $envMap[$entry.Substring(0, $separator)] = $entry.Substring($separator + 1) }
                }
                $rpcUrls = @([string]$envMap['CREG_ETH_RPC']) + @(([string]$envMap['CREG_ETH_RPC_FALLBACKS']).Split(','))
                $rpcHosts = @()
                foreach ($rpcUrl in $rpcUrls) {
                    if ([string]::IsNullOrWhiteSpace($rpcUrl)) { continue }
                    try {
                        $rpcHost = ([Uri]$rpcUrl.Trim()).Host.ToLowerInvariant()
                        if ($rpcHost -and $rpcHost -notin $rpcHosts) { $rpcHosts += $rpcHost }
                    } catch { }
                }
                $record.rpc_provider_hosts = @($rpcHosts | Sort-Object)
            }
            $record.stats = Invoke-RestMethod -Uri "$base/v1/public/chain/stats" -TimeoutSec 15
            $record.pbft = Invoke-RestMethod -Uri "$base/v1/consensus/pbft" -TimeoutSec 15
            $record.metrics = (Invoke-WebRequest -Uri "$base/metrics" -TimeoutSec 15).Content
        } catch { $record.error = $_.Exception.Message }
    } else {
        $remoteLines = @($remoteTemplate.Trim() -split "`r?`n")
        if ($remoteLines[0] -ne "python3 - <<'PY'" -or $remoteLines[-1] -ne 'PY') {
            throw 'Fleet capture remote script wrapper is malformed.'
        }
        $pythonSource = ($remoteLines[1..($remoteLines.Count - 2)] -join "`n").Replace('PORT', [string]$hostInfo.Port).Replace('SLOT', $hostInfo.Slot)
        $encodedSource = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($pythonSource))
        # A single line avoids Windows cmd.exe and gcloud.ps1 corrupting a
        # multiline SSH command before it reaches the Linux host.
        $command = "echo $encodedSource | base64 -d | python3"
        $result = (& $gcloudPath compute ssh $hostInfo.Name "--project=$($hostInfo.Project)" "--zone=$($hostInfo.Zone)" --tunnel-through-iap --quiet "--command=$command" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) {
            $record = @{ error = 'Remote capture failed.' }
        } else {
            try { $record = $result | ConvertFrom-Json -AsHashtable }
            catch { $record = @{ error = 'Remote capture was not valid JSON.' } }
        }
    }
    foreach ($key in @('Slot', 'Role', 'Name', 'Project', 'Zone', 'Port')) {
        if ($hostInfo.ContainsKey($key)) { $record[$key.ToLowerInvariant()] = $hostInfo[$key] }
    }
    $rpcProviderHosts = @($record.rpc_provider_hosts | Where-Object {
        -not [string]::IsNullOrWhiteSpace([string]$_)
    } | Sort-Object -Unique)
    $record.rpc_provider_hosts = $rpcProviderHosts
    $checks = [ordered]@{
        container_running = $record.container_state.Running -eq $true
        scanner_ready = $record.metrics -match '(?m)^creg_scanner_profile_ready 1\s*$'
        yara_ready = $record.metrics -match '(?m)^creg_scanner_yara_ready 1\s*$'
        threat_feed_ready = $record.metrics -match '(?m)^creg_scanner_threat_intel_ready 1\s*$'
        threat_feed_entries_100 = $record.metrics -match '(?m)^creg_scanner_threat_intel_entries 100\s*$'
        osv_ready = $record.metrics -match '(?m)^creg_scanner_osv_ready 1\s*$'
        scanner_profile_matches = $record.metrics -match ('profile_digest="' + $expectedProfile + '"')
        l1_rpc_provider_quorum_configured = $hostInfo.Role -ne 'validator' -or $rpcProviderHosts.Count -ge 3
        signature_verified = $record.stats.spec_signature_verified -eq $true
        chain_id_matches = $record.stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7'
        genesis_matches = ($record.stats.genesis_hash -replace '^0x', '') -eq $expectedGenesisBlock
        four_active_validators = $record.stats.active_validators -eq 4 -and $record.stats.validator_count -eq 4 -and $record.metrics -match '(?m)^creg_active_validators 4\s*$'
        role_matches = $record.health.is_validator -eq ($hostInfo.Role -eq 'validator')
        live_peers = $record.stats.peer_count -gt 0
    }
    if ($hostInfo.Role -eq 'validator') {
        $sync = $record.health.validator_set_sync
        $lastPoll = [DateTimeOffset]::MinValue
        $pollParsed = [DateTimeOffset]::TryParse([string]$sync.last_poll_at, [ref]$lastPoll)
        $pollAgeSeconds = if ($pollParsed) { ([DateTimeOffset]::UtcNow - $lastPoll.ToUniversalTime()).TotalSeconds } else { [double]::PositiveInfinity }
        $checks.validator_set_sync_current =
            $sync.enabled -eq $true -and
            $sync.mode -eq 'chain-authoritative' -and
            $sync.state -eq 'synced' -and
            [string]::IsNullOrWhiteSpace([string]$sync.last_error) -and
            $null -ne $sync.cursor_block -and
            $null -ne $sync.last_finalized_source_block -and
            [long]$sync.cursor_block -ge [long]$sync.last_finalized_source_block -and
            $pollAgeSeconds -ge -30 -and $pollAgeSeconds -le 120
        $checks.validator_sandbox_isolated = $record.health.sandbox.isolated -eq $true
        $checks.validator_sandbox_nsjail = $record.health.sandbox.engine -eq 'nsjail'
        $checks.validator_dev_bypass_disabled = $record.health.sandbox.dev_bypass -eq $false
    }
    $record.checks = $checks
    $record.ready = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    $record.unresolved_pbft_rounds = @($record.pbft.rounds | Where-Object {
        $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
    })
    $record.pbft_clear = $null -ne $record.pbft -and $record.unresolved_pbft_rounds.Count -eq 0
    $record.recovery_queue = [ordered]@{
        pending_block_txs = Get-PrometheusMetricValue -Metrics ([string]$record.metrics) -Name 'creg_pending_block_txs'
        durable_finalized_work_depth = Get-PrometheusMetricValue -Metrics ([string]$record.metrics) -Name 'creg_durable_finalized_work_depth'
        durable_finalized_work_ready_depth = Get-PrometheusMetricValue -Metrics ([string]$record.metrics) -Name 'creg_durable_finalized_work_ready_depth'
        durable_finalized_work_dead_letter_depth = Get-PrometheusMetricValue -Metrics ([string]$record.metrics) -Name 'creg_durable_finalized_work_dead_letter_depth'
        forced_inclusion_pending = Get-PrometheusMetricValue -Metrics ([string]$record.metrics) -Name 'creg_forced_inclusion_pending'
    }
    $record.recovery_clear = $null -ne $record.recovery_queue.pending_block_txs -and
        $null -ne $record.recovery_queue.durable_finalized_work_depth -and
        $null -ne $record.recovery_queue.forced_inclusion_pending -and
        $record.recovery_queue.pending_block_txs -eq 0 -and
        $record.recovery_queue.durable_finalized_work_depth -eq 0 -and
        $record.recovery_queue.forced_inclusion_pending -eq 0 -and $record.pbft_clear
    $records += $record
    $failed = @($checks.Keys | Where-Object { $checks[$_] -ne $true }) -join ','
    "$($hostInfo.Slot): ready=$($record.ready); recovery_clear=$($record.recovery_clear); failed checks=$failed"
}
$slotOrder = @{ v1 = 0; v2 = 1; v3 = 2; v4 = 3; o1 = 4; o2 = 5; o3 = 6; o4 = 7 }
$records = @($records | Sort-Object { $slotOrder[$_.slot] })
$presentTips = @($records | Where-Object { $null -ne $_.stats } | ForEach-Object {
    '{0}|{1}|{2}|{3}' -f $_.stats.chain_id, $_.stats.genesis_hash, $_.stats.tip_height, $_.stats.tip_hash
} | Select-Object -Unique)
$allEightSharedTip = $records.Count -eq 8 -and $presentTips.Count -eq 1 -and
    @($records | Where-Object { $null -eq $_.stats }).Count -eq 0
$imageIds = @($records | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.image_id) } | ForEach-Object image_id | Select-Object -Unique)
$evidence = [ordered]@{
    schema = 'creg-phase-c-real-fleet-readiness-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    scanner_profile_digest = $expectedProfile
    all_eight_ready = $records.Count -eq 8 -and @($records | Where-Object { -not $_.ready }).Count -eq 0 -and $allEightSharedTip
    all_eight_recovery_clear = $records.Count -eq 8 -and @($records | Where-Object { -not $_.recovery_clear }).Count -eq 0
    all_eight_pbft_clear = $records.Count -eq 8 -and @($records | Where-Object { -not $_.pbft_clear }).Count -eq 0
    all_eight_shared_finalized_tip = $allEightSharedTip
    image_ids_aligned = $records.Count -eq 8 -and $imageIds.Count -eq 1 -and @($records | Where-Object { [string]::IsNullOrWhiteSpace([string]$_.image_id) }).Count -eq 0
    public_cutover = $false
    nodes = $records
}
$evidence | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
"Fleet evidence saved; all eight ready=$($evidence.all_eight_ready); all eight recovery clear=$($evidence.all_eight_recovery_clear)"
