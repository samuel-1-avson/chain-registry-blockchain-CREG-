param(
    [Parameter(Mandatory = $true)] [string]$FleetReadinessPath,
    [Parameter(Mandatory = $true)] [string]$EvidencePath
)

$ErrorActionPreference = 'Stop'
$fleet = Get-Content -LiteralPath $FleetReadinessPath -Raw | ConvertFrom-Json -AsHashtable
$expectedTip = '0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0'
$records = @()

foreach ($node in $fleet.nodes | Sort-Object slot) {
    $record = [ordered]@{ slot = $node.slot; ready_capture_tip = $node.stats.tip_hash }
    try {
        if ($node.slot -eq 'v1') {
            $pbft = Invoke-RestMethod -Uri 'http://127.0.0.1:18080/v1/consensus/pbft' -TimeoutSec 8
        } else {
            $command = "sudo docker exec creg-phase-c-r7-$($node.slot) curl -fsS --max-time 6 http://127.0.0.1:8080/v1/consensus/pbft"
            $output = (& gcloud compute ssh $node.name "--project=$($node.project)" "--zone=$($node.zone)" --tunnel-through-iap --quiet "--command=$command" 2>$null | Out-String).Trim()
            if ($LASTEXITCODE -ne 0) { throw 'PBFT state endpoint could not be read.' }
            $pbft = $output | ConvertFrom-Json -AsHashtable
        }
        $rounds = @($pbft.rounds)
        $active = @($rounds | Where-Object { $_.phase -notin @('finalised', 'failed') })
        $record.round_count = $rounds.Count
        $record.active_round_count = $active.Count
        $record.active_rounds = @($active | ForEach-Object {
            [ordered]@{ height = $_.height; phase = $_.phase; view = $_.view }
        })
        $record.checks = [ordered]@{
            readiness_capture_matches_h4 = $node.stats.tip_height -eq 4 -and $node.stats.tip_hash -eq $expectedTip
            no_active_pbft_round = $active.Count -eq 0
            recovery_clear = $node.recovery_clear -eq $true
        }
        $record.passed = @($record.checks.Values | Where-Object { $_ -ne $true }).Count -eq 0
    } catch {
        $record.passed = $false
        $record.error = $_.Exception.Message
    }
    $records += [pscustomobject]$record
    $evidence = [ordered]@{
        schema = 'creg-real-fleet-pre-restart-safety-v1'
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        node_count = $records.Count
        all_nodes_safe = $records.Count -eq 8 -and @($records | Where-Object { -not $_.passed }).Count -eq 0
        nodes = $records
    }
    $evidence | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $EvidencePath -Encoding utf8
    "$($node.slot): active PBFT rounds=$($record.active_round_count) passed=$($record.passed)"
}

if ($records.Count -ne 8 -or @($records | Where-Object { -not $_.passed }).Count -gt 0) {
    throw 'Fleet restart safety preflight failed; inspect evidence before restarting any node.'
}
Write-Output "All eight nodes are safe for committed-state restart checks; evidence: $EvidencePath"
