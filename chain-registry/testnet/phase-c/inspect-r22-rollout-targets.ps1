param(
    [string]$FleetPath = 'docs\testnet-heal-evidence\2026-09-27-phase-c-implementation-start\r22-rollout\preflight.json',
    [string]$EvidencePath = 'docs\testnet-heal-evidence\2026-09-27-phase-c-implementation-start\r22-rollout\remote-target-inventory.json'
)

$ErrorActionPreference = 'Stop'
$profile = 'd09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$fleet = Get-Content -LiteralPath $FleetPath -Raw | ConvertFrom-Json
if (-not $fleet.all_eight_ready -or $fleet.nodes.Count -ne 8) { throw 'Fresh all-eight preflight is required.' }

$template = @'
sudo python3 - <<'PY'
import hashlib, json, pathlib, subprocess
slot = 'SLOT_PLACEHOLDER'
role = 'ROLE_PLACEHOLDER'
container = 'creg-phase-c-r7-' + slot
base = pathlib.Path('/var/lib/creg/phase-c-r7')
env_path = base / 'node.env'
data = base / 'data'
def run(args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()
image, running = run(['sudo', 'docker', 'inspect', container, '--format', '{{.Image}} {{.State.Running}}']).split()
env_bytes = env_path.read_bytes()
env = {}
for line in env_bytes.decode('utf8').splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        key, value = line.split('=', 1)
        if key in {'CREG_NODE_ID', 'CREG_NODE_ROLE', 'CREG_SCANNER_PROFILE_DIGEST', 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM'} or key.startswith('CREG_PHASE_C_COMMIT_CAPTURE_'):
            env[key] = value
identity_path = data / 'p2p' / 'identity.key'
journal_path = data / 'pbft-safety.rocksdb'
record = {
    'slot': slot,
    'role': role,
    'container': container,
    'container_running': running == 'true',
    'image_id': image,
    'node_id': env.get('CREG_NODE_ID'),
    'node_role': env.get('CREG_NODE_ROLE'),
    'scanner_profile_digest': env.get('CREG_SCANNER_PROFILE_DIGEST'),
    'small_cluster_quorum_disabled': env.get('CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM') == 'false',
    'commit_capture_arm_unset': not any(k.startswith('CREG_PHASE_C_COMMIT_CAPTURE_') for k in env),
    'environment_sha256': hashlib.sha256(env_bytes).hexdigest(),
    'identity_present': identity_path.is_file(),
    'identity_sha256': hashlib.sha256(identity_path.read_bytes()).hexdigest() if identity_path.is_file() else None,
    'chain_store_present': (data / 'chain.rocksdb').is_dir(),
    'signer_journal_present': journal_path.is_dir(),
    'scanner_bundle_present': (base / 'scanner-r18-osv-r10').is_dir(),
}
print(json.dumps(record))
PY
'@

$records = @()
foreach ($item in $fleet.nodes | Where-Object { $_.slot -ne 'v1' } | Sort-Object slot) {
    $command = $template.Replace('SLOT_PLACEHOLDER', $item.slot).Replace('ROLE_PLACEHOLDER', $item.role)
    $output = (& gcloud compute ssh $item.name "--project=$($item.project)" "--zone=$($item.zone)" --tunnel-through-iap --quiet "--command=$command" 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        $failurePath = Join-Path (Split-Path -Parent $EvidencePath) "$($item.slot)-inventory-failure-$([DateTime]::UtcNow.ToString('HHmmss')).txt"
        New-Item -ItemType Directory -Path (Split-Path -Parent $failurePath) -Force | Out-Null
        $output | Set-Content -LiteralPath $failurePath -Encoding utf8
        throw "Read-only target inspection failed for $($item.slot); see $failurePath."
    }
    try { $record = $output | ConvertFrom-Json }
    catch { throw "Target inspection output was not JSON for $($item.slot)." }
    $record | Add-Member -NotePropertyName name -NotePropertyValue $item.name
    $record | Add-Member -NotePropertyName project -NotePropertyValue $item.project
    $record | Add-Member -NotePropertyName zone -NotePropertyValue $item.zone
    $record | Add-Member -NotePropertyName port -NotePropertyValue $item.port
    $expectedNodeId = if ($item.role -eq 'validator') { "validator-$($item.slot.Substring(1))" } else { "observer-$($item.slot.Substring(1))" }
    $record | Add-Member -NotePropertyName expected_node_id -NotePropertyValue $expectedNodeId
    $record | Add-Member -NotePropertyName health_role_matches -NotePropertyValue ($item.health.is_validator -eq ($item.role -eq 'validator'))
    $records += $record
    "$($item.slot): running=$($record.container_running); role=$($record.node_id)/$($record.node_role); image=$($record.image_id)"
}

$bad = @($records | Where-Object {
    -not $_.container_running -or
    $_.node_id -ne $_.expected_node_id -or
    -not $_.health_role_matches -or
    ($_.role -eq 'validator' -and $_.node_role -ne 'validator') -or
    ($_.role -eq 'observer' -and $_.node_role -and $_.node_role -ne 'observer') -or
    $_.scanner_profile_digest -ne $profile -or
    -not $_.small_cluster_quorum_disabled -or
    -not $_.commit_capture_arm_unset -or
    -not $_.identity_present -or
    -not $_.chain_store_present -or
    -not $_.scanner_bundle_present -or
    ($_.role -eq 'validator' -and -not $_.signer_journal_present)
})
$result = [ordered]@{
    schema = 'creg-phase-c-r22-rollout-target-inventory-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    preflight_path = $FleetPath
    expected_scanner_profile = $profile
    target_count = $records.Count
    all_targets_ready_for_sequential_rollout = $bad.Count -eq 0 -and $records.Count -eq 7
    targets = $records
}
New-Item -ItemType Directory -Path (Split-Path -Parent $EvidencePath) -Force | Out-Null
$result | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $result.all_targets_ready_for_sequential_rollout) { throw 'One or more remote R22 rollout targets failed the preflight.' }
Write-Output 'All seven remote targets passed read-only image, identity, storage, role, and scanner checks.'
