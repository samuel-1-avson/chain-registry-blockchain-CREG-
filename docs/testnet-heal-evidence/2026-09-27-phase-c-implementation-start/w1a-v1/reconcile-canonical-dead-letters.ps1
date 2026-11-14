param(
    [string]$EvidenceRoot = 'docs\testnet-heal-evidence\2026-09-27-phase-c-implementation-start',
    [string]$OutputDirectory = 'docs\testnet-heal-evidence\2026-09-27-phase-c-implementation-start\w1a-v1\canonical-ledger-review-20260927',
    [string]$BaseUrl = 'http://127.0.0.1:18080'
)

$ErrorActionPreference = 'Stop'
$expectedChain = 'creg-phase-c-testnet-2026-09-26-r7'
$expectedTip = 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d'
$packages = @('npm:is-number@7.0.0', 'npm:escape-html@1.0.3', 'npm:ms@2.1.3', 'npm:toidentifier@1.0.1')

$stats = Invoke-RestMethod -Uri "$BaseUrl/v1/public/chain/stats" -TimeoutSec 15
if ($stats.chain_id -ne $expectedChain -or $stats.tip_height -ne 12 -or $stats.tip_hash -ne $expectedTip -or -not $stats.spec_signature_verified) {
    throw 'Live V1 chain identity or tip does not match the accepted R7 baseline.'
}

$blocks = @()
for ($height = 0; $height -le 12; $height++) {
    $block = Invoke-RestMethod -Uri "$BaseUrl/v1/public/blocks/$height" -TimeoutSec 15
    if ($block.header.height -ne $height -or -not $block.finalized) { throw "Height $height is not finalized." }
    $blocks += $block
}
for ($height = 1; $height -le 12; $height++) {
    if ($blocks[$height].header.prev_hash -ne $blocks[$height - 1].hash) { throw "Parent continuity failed at height $height." }
}

$packageStates = @{}
foreach ($canonical in $packages) {
    $encoded = [Uri]::EscapeDataString($canonical)
    $packageStates[$canonical] = Invoke-RestMethod -Uri "$BaseUrl/v1/public/packages/$encoded" -TimeoutSec 15
}

$entries = @()
foreach ($slot in @('v1', 'v2', 'v3', 'v4')) {
    $slotRoot = Join-Path $EvidenceRoot "w1a-$slot"
    $export = Get-ChildItem -LiteralPath $slotRoot -Directory -Filter 'w1a-v*-export-*' |
        Where-Object { Test-Path (Join-Path $_.FullName 'diagnostics.json') } |
        Sort-Object Name -Descending | Select-Object -First 1
    if ($null -eq $export) { throw "No complete diagnostics export found for $slot." }
    $diagnostics = Get-Content -LiteralPath (Join-Path $export.FullName 'diagnostics.json') -Raw | ConvertFrom-Json
    if ($diagnostics.active_count -ne 0) { throw "$slot has active retained work; W1a cannot pass." }
    foreach ($deadLetter in $diagnostics.dead_letters) {
        $match = $null
        if ($deadLetter.transaction.type -eq 'publish_authorized_v1') {
            foreach ($candidateBlock in $blocks) {
                foreach ($transaction in $candidateBlock.transactions) {
                    if ($transaction.type -ne 'publish_authorized_v1') { continue }
                    $id = $transaction.record.id
                    $canonical = "$($id.ecosystem):$($id.name)@$($id.version)"
                    if ($canonical -eq $deadLetter.transaction.canonical -and
                        $transaction.authorization.nonce -eq $deadLetter.transaction.publisher_nonce -and
                        $transaction.authorization.chain_id -eq $deadLetter.transaction.authorization_chain_id -and
                        $transaction.authorization.publisher_address -eq $deadLetter.transaction.publisher) {
                        $match = $candidateBlock
                    }
                }
            }
            $kind = 'publish'
            $reasonExpected = 'publisher authorization nonce is already committed on this chain'
            $state = $packageStates[$deadLetter.transaction.canonical].status
            $statusExpected = 'revoked'
        } elseif ($deadLetter.transaction.type -eq 'revoke') {
            foreach ($candidateBlock in $blocks) {
                foreach ($transaction in $candidateBlock.transactions) {
                    if ($transaction.type -eq 'revoke' -and $transaction.package_canonical -eq $deadLetter.transaction.canonical) {
                        $match = $candidateBlock
                    }
                }
            }
            $kind = 'revoke'
            $reasonExpected = 'revocation target is already revoked on this chain'
            $state = $packageStates[$deadLetter.transaction.canonical].status
            $statusExpected = 'revoked'
        } else {
            throw "Unrecognized dead-letter transaction type $($deadLetter.transaction.type) on $slot."
        }

        $matched = $null -ne $match -and $state -eq $statusExpected -and $deadLetter.terminal_reason -eq $reasonExpected
        $entries += [ordered]@{
            slot = $slot
            work_id = $deadLetter.work_id
            kind = $kind
            canonical = $deadLetter.transaction.canonical
            publisher_nonce = $deadLetter.transaction.publisher_nonce
            authorization_chain_id = $deadLetter.transaction.authorization_chain_id
            terminal_reason = $deadLetter.terminal_reason
            canonical_block_height = if ($match) { $match.header.height } else { $null }
            canonical_block_hash = if ($match) { $match.hash } else { $null }
            package_status = $state
            terminal_duplicate_confirmed = $matched
        }
    }
}

$uniqueWorkIds = @($entries.work_id | Sort-Object -Unique)
$unmatched = @($entries | Where-Object { -not $_.terminal_duplicate_confirmed })
$review = [ordered]@{
    schema = 'creg-phase-c-w1a-dead-letter-review-v1'
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    source = 'live public canonical block and package routes on local V1 after fresh eight-node readiness capture'
    chain_id = $stats.chain_id
    tip_height = $stats.tip_height
    tip_hash = $stats.tip_hash
    genesis_block_hash = $stats.genesis_hash
    spec_signature_verified = $stats.spec_signature_verified
    chain_blocks_captured = $blocks.Count
    block_parent_continuity_verified = $true
    validator_records = $entries.Count
    unique_work_ids = $uniqueWorkIds.Count
    active_work_records = 0
    confirmed_terminal_duplicates = @($entries | Where-Object { $_.terminal_duplicate_confirmed }).Count
    unresolved_records = $unmatched.Count
    requeued_or_cleared = $false
    pass = $entries.Count -eq 25 -and $unmatched.Count -eq 0
    entries = $entries
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
[ordered]@{ captured_at_utc = $review.captured_at_utc; stats = $stats; blocks = $blocks; package_states = $packageStates } |
    ConvertTo-Json -Depth 90 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'canonical-chain-capture.json') -Encoding utf8
$review | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'dead-letter-review.json') -Encoding utf8
$review | ConvertTo-Json -Depth 8
if (-not $review.pass) { throw 'One or more validator dead-letter records did not match terminal canonical state.' }
