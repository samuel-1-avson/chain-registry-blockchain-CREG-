param(
    [Parameter(Mandatory=$true)][string]$ImageId,
    [Parameter(Mandatory=$true)][string[]]$AcceptedImageIds,
    [Parameter(Mandatory=$true)][string]$EvidencePath
)
$ErrorActionPreference='Stop'
$image='creg-node:phase-c-secure-r21-revoke-retirement'
$container='creg-phase-c-r7-v1'
$preserved="$container-r20-before-r21"
$failed="$container-r21-failed"
$data='C:\Users\samue\.creg-phase-c\r7\v1\data'
$privateRoot='C:\Users\samue\CREG-backups\phase-c-candidate-r7\v1'
$envPath=Join-Path $privateRoot 'node.env'
$envBackup=Join-Path $privateRoot 'node.env.r20-preserved-for-r21'
$scanner='C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r16-osv-r10'
$api='http://127.0.0.1:18080'
$profile='d09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$tip='e0b40d52f7626f46a0bcf85040ec2c34dc1d576af0b1e7d65396d31c325f0837'
$genesis='0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
if ($ImageId -notin $AcceptedImageIds) { throw 'R21 image ID is not in the build manifest.' }
if ((docker image inspect $image --format '{{.Id}}').Trim() -notin $AcceptedImageIds) { throw 'Local R21 image pin mismatch.' }
if ((docker inspect $container --format '{{.State.Status}}').Trim() -ne 'exited') { throw 'V1 must be stopped for guarded replacement.' }
foreach ($path in @($envPath,(Join-Path $data 'p2p/identity.key'),(Join-Path $data 'chain.rocksdb'),(Join-Path $data 'pbft-safety.rocksdb'),$scanner)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required preserved path missing: $path" }
}
foreach ($name in @($preserved,$failed)) {
    $existing=(& docker container ls -aq --filter "name=^/$name$"|Out-String).Trim()
    if ($existing) { throw "Preserved container name already exists: $name" }
}
foreach ($path in @($envBackup,$EvidencePath)) {
    if (Test-Path -LiteralPath $path) { throw "Preservation path already exists: $path" }
}
$envText=[IO.File]::ReadAllText($envPath)
foreach ($entry in @("CREG_SCANNER_PROFILE_DIGEST=$profile",'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false','CREG_NODE_ID=validator-1','CREG_NODE_ROLE=validator')) {
    if ([regex]::Matches($envText,'(?m)^'+[regex]::Escape($entry)+'\r?$').Count -ne 1) { throw "Expected node setting missing or duplicated: $entry" }
}
$identity=(Get-FileHash (Join-Path $data 'p2p/identity.key')).Hash
$envHash=(Get-FileHash $envPath).Hash
$prior=@((& docker inspect $container | Out-String)|ConvertFrom-Json)[0]
$report=[ordered]@{schema='creg-real-r21-rollout-v1';slot='v1';captured_at_utc=[DateTime]::UtcNow.ToString('o');prior_image=$prior.Image;image_id=$ImageId;preserved_container=$preserved;passed=$false;public_cutover=$false}
$mutated=$false
try {
    Copy-Item -LiteralPath $envPath -Destination $envBackup
    & docker rename $container $preserved | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not preserve prior V1 container.' }
    $mutated=$true
    $args=@('run','-d','--name',$container,'--restart','unless-stopped','--privileged','--env-file',$envPath,
        '-p','18080:8080','-p','9001:9000','--mount',"type=bind,source=$data,target=/data",
        '--mount',"type=bind,source=$scanner,target=/scanner,readonly",'-v','/var/run/docker.sock:/var/run/docker.sock',$image)
    & docker @args | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'R21 V1 container creation failed.' }
    if ((docker inspect $container --format '{{.Image}}').Trim() -notin $AcceptedImageIds) { throw 'Running V1 image differs from R21 pin.' }
    if ((Get-FileHash (Join-Path $data 'p2p/identity.key')).Hash -ne $identity) { throw 'V1 P2P identity changed.' }
    if ((Get-FileHash $envPath).Hash -ne $envHash) { throw 'V1 node environment changed.' }

    $deadline=[DateTime]::UtcNow.AddMinutes(6)
    $last=$null
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $stats=Invoke-RestMethod "$api/v1/public/chain/stats" -TimeoutSec 8
            $health=Invoke-RestMethod "$api/v1/public/health" -TimeoutSec 8
            $pbft=Invoke-RestMethod "$api/v1/consensus/pbft" -TimeoutSec 8
            $package=Invoke-RestMethod "$api/v1/public/packages/npm%3Atoidentifier%401.0.1" -TimeoutSec 8
            $metrics=(Invoke-WebRequest "$api/metrics" -TimeoutSec 8).Content
            $checks=[ordered]@{
                chain_identity=$stats.chain_id -eq 'creg-phase-c-testnet-2026-09-26-r7' -and $stats.spec_signature_verified
                genesis_preserved=($stats.genesis_hash -replace '^0x','') -eq $genesis
                h9_preserved=$stats.tip_height -eq 9 -and $stats.tip_hash -eq $tip
                package_revocation_committed=$package.status -eq 'revoked' -and $package.block_phase -eq 'committed'
                four_validator_roster=$stats.active_validators -eq 4 -and $stats.validator_count -eq 4
                connected_peers=$stats.peer_count -gt 0
                correct_role=$health.is_validator
                scanner_profile_ready=$metrics -match '(?m)^creg_scanner_profile_ready 1\s*$' -and $metrics.Contains($profile)
                pending_and_forced_queues_clear=$metrics -match '(?m)^creg_pending_block_txs 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_depth 0\s*$' -and $metrics -match '(?m)^creg_durable_finalized_work_ready_depth 0\s*$' -and $metrics -match '(?m)^creg_forced_inclusion_pending 0\s*$'
                no_unfinished_pbft_lock=@($pbft.rounds|Where-Object{$_.phase -notin @('finalised','failed') -or ($_.phase -eq 'failed' -and $_.prepare_count -ge $_.quorum)}).Count -eq 0
                isolated_sandbox=$health.sandbox.engine -eq 'nsjail' -and $health.sandbox.isolated -and -not $health.sandbox.dev_bypass
            }
            $last=[ordered]@{checks=$checks;stats=$stats;health=$health;pbft=$pbft}
            if (@($checks.Values|Where-Object{$_ -ne $true}).Count -eq 0) { break }
        } catch { $last=[ordered]@{last_error=$_.Exception.Message} }
        Start-Sleep -Seconds 2
    }
    if (-not $last.checks -or @($last.checks.Values|Where-Object{$_ -ne $true}).Count -gt 0) { throw 'R21 V1 readiness did not pass; prior V1 will be restored.' }
    if ((docker inspect $preserved --format '{{.State.Status}}').Trim() -ne 'exited') { throw 'Prior V1 container is not preserved stopped.' }
    $report.after=$last
    $report.p2p_identity_preserved=$true
    $report.node_env_preserved=$true
    $report.signer_journal_present=Test-Path -LiteralPath (Join-Path $data 'pbft-safety.rocksdb')
    $report.passed=$report.signer_journal_present
    $report|ConvertTo-Json -Depth 24|Set-Content -LiteralPath $EvidencePath -Encoding utf8
    if (-not $report.passed) { throw 'V1 signer journal is missing after R21 start.' }
    $mutated=$false
} catch {
    if ($mutated) {
        try {
            $state=(docker inspect $container --format '{{.State.Status}}' 2>$null|Out-String).Trim()
            if ($state -eq 'running') { docker stop --time 20 $container|Out-Null }
            if ((docker inspect $container --format '{{.State.Status}}' 2>$null|Out-String).Trim()) { docker rename $container $failed }
            docker rename $preserved $container|Out-Null
            docker start $container|Out-Null
        } catch { Write-Warning "V1 rollback needs inspection: $($_.Exception.Message)" }
    }
    throw
}
Write-Output 'R21 V1 replacement passed; chain data, P2P identity, node environment and signer journal were preserved.'
