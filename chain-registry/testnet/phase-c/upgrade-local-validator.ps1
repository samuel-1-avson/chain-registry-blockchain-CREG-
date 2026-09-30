param(
    [Parameter(Mandatory)][string]$Image,
    [Parameter(Mandatory)][string]$ExpectedImageId
)

$ErrorActionPreference = 'Stop'
if ($ExpectedImageId -notmatch '^sha256:[0-9a-f]{64}$') { throw 'Expected image ID must be SHA-256.' }
$id = (& docker image inspect $Image --format '{{.Id}}')
if ($LASTEXITCODE -ne 0 -or $id -ne $ExpectedImageId) { throw 'Local candidate image digest mismatch.' }
$state = (& docker inspect creg-phase-c-v1 --format '{{.State.Status}}')
if ($LASTEXITCODE -ne 0 -or $state -ne 'exited') { throw 'First candidate must be stopped.' }
& docker inspect creg-phase-c-v1-r1 --format '{{.Id}}' 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) { throw 'First-run evidence container already exists.' }

$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate\v1'
$dataDir = 'C:\Users\samue\.creg-phase-c\v1\data'
if (-not (Test-Path -LiteralPath (Join-Path $privateRoot 'node.env'))) { throw 'Node environment is missing.' }
if (-not (Test-Path -LiteralPath (Join-Path $dataDir 'p2p\identity.key'))) { throw 'P2P identity is missing.' }
& docker rename creg-phase-c-v1 creg-phase-c-v1-r1
if ($LASTEXITCODE -ne 0) { throw 'Could not preserve first-run container.' }
& docker run -d --name creg-phase-c-v1 --restart unless-stopped --privileged `
    --env-file (Join-Path $privateRoot 'node.env') `
    -p 18080:8080 -p 9001:9000 `
    --mount "type=bind,source=$dataDir,target=/data" `
    -v /var/run/docker.sock:/var/run/docker.sock $Image
if ($LASTEXITCODE -ne 0) { throw 'Repaired laptop candidate did not start.' }
$actual = (& docker inspect creg-phase-c-v1 --format '{{.Image}}')
if ($actual -ne $ExpectedImageId) { throw 'Running laptop candidate image digest mismatch.' }
Write-Output "Started repaired laptop validator with preserved PBFT journal; image $actual"
