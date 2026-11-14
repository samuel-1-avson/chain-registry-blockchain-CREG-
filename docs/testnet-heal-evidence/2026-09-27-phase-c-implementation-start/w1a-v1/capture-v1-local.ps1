$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$container = 'creg-phase-c-r7-v1'
$expectedImage = 'sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423'
$sourceDb = 'C:\Users\samue\.creg-phase-c\r7\v1\data\chain.rocksdb'
$evidenceRoot = Join-Path (Get-Location).Path 'docs\testnet-heal-evidence\2026-09-27-phase-c-implementation-start\w1a-v1'
$stamp = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$export = Join-Path $evidenceRoot "w1a-v1-export-$stamp"

New-Item -ItemType Directory -Path $export -Force | Out-Null

$running = docker inspect -f '{{.State.Running}}' $container
if ($LASTEXITCODE -ne 0 -or $running -ne 'true') { throw 'V1 container is not running; capture cancelled.' }
$image = docker inspect -f '{{.Image}}' $container
if ($LASTEXITCODE -ne 0 -or $image -ne $expectedImage) { throw "Unexpected V1 image: $image" }
if (-not (Test-Path -LiteralPath $sourceDb -PathType Container)) { throw 'V1 chain.rocksdb bind-mount source is missing.' }

$before = Invoke-RestMethod -Uri 'http://127.0.0.1:18080/v1/chain/stats' -TimeoutSec 10
if ($before.tip_height -ne 12 -or $before.tip_hash -ne 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d') {
    throw 'V1 tip differs from the accepted R7 h12 baseline; capture cancelled.'
}

$containerId = docker inspect -f '{{.Id}}' $container
$stoppedAt = $null
$restartedAt = $null
try {
    docker stop --time 15 $container | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not stop V1 container for a consistent RocksDB copy.' }
    $stoppedAt = [DateTime]::UtcNow.ToString('o')
    Copy-Item -LiteralPath $sourceDb -Destination $export -Recurse
}
finally {
    $runningNow = docker inspect -f '{{.State.Running}}' $container 2>$null
    if ($runningNow -eq 'false') {
        docker start $container | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'V1 container failed to restart after the local data copy.' }
        $restartedAt = [DateTime]::UtcNow.ToString('o')
    }
}

$deadline = [DateTime]::UtcNow.AddMinutes(2)
$after = $null
while ([DateTime]::UtcNow -lt $deadline) {
    try {
        $after = Invoke-RestMethod -Uri 'http://127.0.0.1:18080/v1/chain/stats' -TimeoutSec 5
        break
    }
    catch { Start-Sleep -Seconds 3 }
}
if ($null -eq $after) { throw 'V1 did not return to API readiness within two minutes.' }
if ($after.tip_height -ne $before.tip_height -or $after.tip_hash -ne $before.tip_hash) {
    throw 'V1 tip changed during the capture; review the snapshot before using it.'
}

$dbCopy = Join-Path $export 'chain.rocksdb'
$hashLines = Get-ChildItem -LiteralPath $dbCopy -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($dbCopy.Length).TrimStart('\') -replace '\\', '/'
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $relative"
}
if (-not $hashLines) { throw 'Copied chain database is empty.' }
$hashLines | Set-Content -LiteralPath (Join-Path $export 'chain-file-hashes.txt') -Encoding ascii

$diagnostics = docker run --rm --network none --mount "type=bind,source=$dbCopy,target=/snapshot/chain.rocksdb,readonly" --entrypoint /app/creg_recovery_inspect $expectedImage /snapshot
if ($LASTEXITCODE -ne 0) { throw 'Pinned read-only recovery inspector failed on the copied V1 database.' }
$diagnostics | Set-Content -LiteralPath (Join-Path $export 'diagnostics.json') -Encoding utf8

$metadata = [ordered]@{
    slot = 'v1'
    capture_mode = 'local validator container stopped only for consistent chain database copy'
    copied_path = 'chain.rocksdb only; external signer journal and key paths were not copied; RocksDB IDENTITY metadata is included'
    stopped_at_utc = $stoppedAt
    restarted_at_utc = $restartedAt
    container_id = $containerId
    image_id = $image
    tip_height_before = $before.tip_height
    tip_hash_before = $before.tip_hash
    tip_height_after = $after.tip_height
    tip_hash_after = $after.tip_hash
}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $export 'capture-metadata.json') -Encoding utf8

Write-Output "capture_directory=$export"
Write-Output "tip_height=$($after.tip_height)"
Write-Output "tip_hash=$($after.tip_hash)"
Write-Output "copied_chain_files=$($hashLines.Count)"
