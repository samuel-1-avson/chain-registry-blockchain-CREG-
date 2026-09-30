param(
    [string]$EvidencePath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r12-live-rollout/v1-local-rollout.json')
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$evidenceDirectory = Split-Path $EvidencePath -Parent
$initialPath = Join-Path $evidenceDirectory 'v1-local-rollout-initial-postflight.json'
$initial = Get-Content -LiteralPath $initialPath -Raw | ConvertFrom-Json
$container = 'creg-phase-c-r7-v1'
$preserved = [string]$initial.preserved_container
$envPath = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1\node.env'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$oldScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r11-osv-kleur'
$newScanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r12-live-escape'
$identityPath = Join-Path $data 'p2p\identity.key'
$journalPath = Join-Path $data 'pbft-safety.rocksdb'
$api = 'http://127.0.0.1:18080'
$image = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$chainId = 'creg-phase-c-testnet-2026-09-26-r7'
$genesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$tipHash = '2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa'
$profile = 'e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2'
$epoch = 'osv-2026-09-27-phase-c-escape-string-regexp-r12-live'

function Get-Metric([string]$Text, [string]$Name) {
    $pattern = '^' + [regex]::Escape($Name) + '(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$'
    $line = $Text -split '\r?\n' | Where-Object { $_ -match $pattern } | Select-Object -First 1
    if (-not $line) { return $null }
    return [double]([regex]::Match($line, $pattern).Groups[1].Value)
}

if (-not (Test-Path -LiteralPath $initialPath) -or -not $initial.checks.identity_unchanged -or
    -not $initial.checks.signer_journal_preserved -or -not $initial.checks.signed_r7_tip_unchanged) {
    throw 'The initial V1 postflight report does not prove its identity, journal, and signed-tip checks.'
}
$containerJson = & docker inspect $container
if ($LASTEXITCODE -ne 0) { throw 'Current V1 container is not inspectable.' }
$current = @($containerJson | ConvertFrom-Json)[0]
$previousJson = & docker inspect $preserved
if ($LASTEXITCODE -ne 0) { throw 'Preserved R11 V1 container is not inspectable.' }
$previous = @($previousJson | ConvertFrom-Json)[0]
$health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 10
$stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 10
$metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 10).Content
$pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 10
$unresolved = @($pbft.rounds | Where-Object {
    $_.phase -ne 'finalised' -and ($_.phase -ne 'failed' -or $_.prepare_count -ge $_.quorum)
})
$mounts = @($current.Mounts)
$envText = [IO.File]::ReadAllText($envPath)
$deadLetters = Get-Metric $metrics 'creg_durable_finalized_work_dead_letter_depth'
$checks = [ordered]@{
    same_image = $current.Image -eq $image
    new_container_running_healthy = $current.State.Running -and $current.State.Health.Status -eq 'healthy' -and $health.status -eq 'ok'
    previous_r11_container_preserved = $previous.State.Status -eq 'exited'
    data_mount_unchanged = @($mounts | Where-Object { $_.Destination -eq '/data' -and $_.Source -eq $data -and $_.RW }).Count -eq 1
    new_scanner_mount_read_only = @($mounts | Where-Object { $_.Destination -eq '/scanner' -and $_.Source -eq $newScanner -and -not $_.RW }).Count -eq 1
    signed_r7_h14 = $stats.chain_id -eq $chainId -and $stats.genesis_hash -eq $genesis -and $stats.spec_signature_verified -and $stats.tip_height -eq 14 -and $stats.finalized_height -eq 14 -and $stats.tip_hash -eq $tipHash
    four_validator_roster = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
    validator_identity_and_peers = $health.node_id -eq 'validator-1' -and $health.is_validator -and $stats.peer_count -gt 0
    scanner_profile_ready = $metrics -match ('profile_digest="' + $profile + '"') -and (Get-Metric $metrics 'creg_scanner_profile_ready') -eq 1
    scanner_inputs_ready = (Get-Metric $metrics 'creg_scanner_osv_ready') -eq 1 -and (Get-Metric $metrics 'creg_scanner_osv_lookups') -eq 6 -and (Get-Metric $metrics 'creg_scanner_yara_ready') -eq 1 -and (Get-Metric $metrics 'creg_scanner_threat_intel_ready') -eq 1 -and (Get-Metric $metrics 'creg_scanner_threat_intel_entries') -eq 100
    queues_clear = (Get-Metric $metrics 'creg_pending_block_txs') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_depth') -eq 0 -and (Get-Metric $metrics 'creg_durable_finalized_work_ready_depth') -eq 0 -and (Get-Metric $metrics 'creg_forced_inclusion_pending') -eq 0
    dead_letters_unchanged = $deadLetters -eq [double]$initial.dead_letter_depth
    pbft_clear = $unresolved.Count -eq 0
    p2p_identity_preserved = $initial.checks.identity_unchanged -and (Test-Path -LiteralPath $identityPath)
    signer_journal_preserved = Test-Path -LiteralPath $journalPath
    r11_scanner_preserved = Test-Path -LiteralPath $oldScanner
    r12_environment_active = $envText -match '(?m)^CREG_OSV_SNAPSHOT_EPOCH=' + [regex]::Escape($epoch) + '(?=\r?$)' -and $envText -match '(?m)^CREG_SCANNER_PROFILE_DIGEST=' + [regex]::Escape($profile) + '(?=\r?$)'
}
$checksPassed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
$report = [ordered]@{
    schema = 'creg-real-r12-live-v1-scanner-rollout-postflight-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    slot = 'v1'
    previous_initial_postflight = (Split-Path $initialPath -Leaf)
    initial_postflight_health_race = $initial.checks.new_container_running -eq $false
    image_id = $current.Image
    signed_tip = [ordered]@{ height = $stats.tip_height; hash = $stats.tip_hash; genesis = $stats.genesis_hash; chain_id = $stats.chain_id }
    scanner_profile_digest = $profile
    scanner_epoch = $epoch
    dead_letter_depth = $deadLetters
    preserved_container = $preserved
    health = $health
    stats = $stats
    checks = $checks
    passed = $checksPassed
    public_cutover = $false
}
New-Item -ItemType Directory -Path $evidenceDirectory -Force | Out-Null
$report | ConvertTo-Json -Depth 24 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
if (-not $checksPassed) { throw "V1 follow-up postflight failed; see $EvidencePath" }
Write-Output "V1 R12 Docker health converged and the fresh postflight passed: $EvidencePath"
