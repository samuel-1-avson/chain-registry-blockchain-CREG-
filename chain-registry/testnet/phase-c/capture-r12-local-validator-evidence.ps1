$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$container = 'creg-phase-c-r7-v1'
$preserved = 'creg-phase-c-r7-v1-r13-preserved'
$image = 'creg-node:phase-c-secure-r12-bounded-l1-retry'
$expectedImages = @(
    'sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd',
    'sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736'
)
$expectedGenesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$expectedTip = '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$expectedProfile = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f'
$data = 'C:\Users\samue\.creg-phase-c\r7\v1\data'
$scanner = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9'
$evidencePath = Join-Path $repoRoot 'docs/testnet-heal-evidence/2026-09-26-phase-c-queue-recovery-r12/restart-after-r12-v4/v1-r12-rollout.json'
$api = 'http://127.0.0.1:18080'

$containerState = (& docker inspect $container --format '{{.State.Status}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw 'R12 V1 container is missing.' }
$containerImage = (& docker inspect $container --format '{{.Image}}' | Out-String).Trim()
$configuredImage = (& docker inspect $container --format '{{.Config.Image}}' | Out-String).Trim()
$tagImageId = (& docker image inspect $image --format '{{.Id}}' | Out-String).Trim()
$mountJson = (& docker inspect $container --format '{{json .Mounts}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect V1 persistent mounts.' }
$mounts = @($mountJson | ConvertFrom-Json)
$dataMount = @($mounts | Where-Object { $_.Destination -eq '/data' })[0]
$scannerMount = @($mounts | Where-Object { $_.Destination -eq '/scanner' })[0]

$health = Invoke-RestMethod -Uri "$api/v1/public/health" -TimeoutSec 8
$stats = Invoke-RestMethod -Uri "$api/v1/public/chain/stats" -TimeoutSec 8
$package = Invoke-RestMethod -Uri "$api/v1/public/packages/npm%3Ais-number%407.0.0" -TimeoutSec 8
$metrics = (Invoke-WebRequest -Uri "$api/metrics" -TimeoutSec 8).Content
$pbft = Invoke-RestMethod -Uri "$api/v1/consensus/pbft" -TimeoutSec 8
$identityPath = Join-Path $data 'p2p\identity.key'
$journalPath = Join-Path $data 'pbft-safety.rocksdb'
$preservedState = (& docker inspect $preserved --format '{{.State.Status}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Preserved R13 V1 container is missing.' }

$assetChecks = [ordered]@{
    known_malicious_hashes = (Get-FileHash -LiteralPath (Join-Path $scanner 'known_malicious_hashes.json') -Algorithm SHA256).Hash.ToLowerInvariant() -eq '5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef'
    osv_snapshot = (Get-FileHash -LiteralPath (Join-Path $scanner 'osv_snapshot.json') -Algorithm SHA256).Hash.ToLowerInvariant() -eq '39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade'
    yara_rules = (Get-FileHash -LiteralPath (Join-Path $scanner 'rules\supply_chain.yar') -Algorithm SHA256).Hash.ToLowerInvariant() -eq '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e'
}
$checks = [ordered]@{
    container_running = $containerState -eq 'running'
    image_tag = $configuredImage -eq $image
    image_pinned = ($containerImage -in $expectedImages) -and ($tagImageId -in $expectedImages)
    prior_r13_container_preserved = $preservedState -eq 'exited'
    persistent_data_mount = $null -ne $dataMount -and $dataMount.Source.Replace('/', '\').TrimEnd('\').EndsWith('.creg-phase-c\r7\v1\data', [StringComparison]::OrdinalIgnoreCase)
    scanner_readonly_mount = $null -ne $scannerMount -and $scannerMount.RW -eq $false
    p2p_identity_present = Test-Path -LiteralPath $identityPath
    signing_journal_present = Test-Path -LiteralPath $journalPath
    same_tip = $stats.tip_height -eq 4 -and $stats.tip_hash -eq $expectedTip
    same_chain = $stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats.genesis_hash -eq $expectedGenesis -and $stats.spec_signature_verified
    validator_roster = $stats.validator_count -eq 4 -and $stats.active_validators -eq 4
    peers_connected = $stats.peer_count -gt 0
    package_state = $package.status -eq 'revoked' -and $package.block_phase -eq 'committed'
    scanner_ready = $metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($expectedProfile)
    queue_clear = $metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_ready_depth 0\s*$' -and $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
    no_unfinished_pbft_round = @($pbft.rounds | Where-Object { $_.phase -notin @('finalised', 'failed') }).Count -eq 0
    isolated_sandbox = $health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -and -not $health.sandbox.dev_bypass
    scanner_assets_pinned = @($assetChecks.Values | Where-Object { $_ -ne $true }).Count -eq 0
}
$evidence = [ordered]@{
    schema = 'creg-real-r12-local-validator-rollout-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    slot = 'v1'
    image_id = $containerImage
    image_tag_id = $tagImageId
    configured_image = $configuredImage
    previous_container = $preserved
    previous_container_state = $preservedState
    mounts = $mounts | Select-Object Type, Source, Destination, RW
    p2p_identity_sha256 = if (Test-Path -LiteralPath $identityPath) { (Get-FileHash -LiteralPath $identityPath -Algorithm SHA256).Hash.ToLowerInvariant() } else { $null }
    stats = $stats
    package_state = $package | Select-Object canonical, status, block_phase, block_hash, revocation_reason
    health = $health
    scanner_asset_checks = $assetChecks
    checks = $checks
    passed = @($checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    public_cutover = $false
}
$evidence | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $evidencePath -Encoding utf8
if (-not $evidence.passed) { throw "R12 V1 checks did not all pass. Evidence: $evidencePath" }
Write-Output "R12 V1 live evidence passed: $evidencePath"
