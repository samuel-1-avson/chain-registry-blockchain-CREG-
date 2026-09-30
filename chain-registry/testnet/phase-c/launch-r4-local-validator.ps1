param([Parameter(Mandatory)][string]$GenesisHash)
$ErrorActionPreference = 'Stop'
$image = 'creg-node:phase-c-secure-r4'
$expectedImage = 'sha256:ca4d04e77e9680cb878800f2ea72c1ce7dac6bb3575a55c9921a7314975fb1ef'
$expectedConfig = 'sha256:6c6861bc4c8efc6870cea646abc880c383cff1fb57267c699438844611ff543d'
$privateRoot = 'C:\Users\samue\CREG-backups\phase-c-r4\v1'
$dataDir = 'C:\Users\samue\.creg-phase-c\v1-r4\data'
$container = 'creg-phase-c-r4-v1'
if ($GenesisHash -notmatch '^0x[0-9a-f]{64}$') { throw 'Invalid signed genesis hash.' }
$imageId = & docker image inspect $image --format '{{.Id}}'
if ($LASTEXITCODE -ne 0 -or $imageId -notin @($expectedImage, $expectedConfig)) { throw 'Repair image digest mismatch.' }
& docker inspect $container --format '{{.Id}}' 2>$null | Out-Null
if ($LASTEXITCODE -eq 0 -or (Test-Path -LiteralPath $dataDir)) { throw 'Fresh run container or data already exists; preserve it.' }
$envPath = Join-Path $privateRoot 'node.env'
$lines = Get-Content -LiteralPath $envPath
foreach ($required in @('CREG_CHAIN_ID=creg-phase-c-testnet-2026-09-26', "CREG_GENESIS_HASH=$GenesisHash", 'CREG_IS_VALIDATOR=true', 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false', 'CREG_DEV_SANDBOX=false')) {
    if ($lines -notcontains $required) { throw 'Fresh run environment does not match signed chain and safety settings.' }
}
$identityPath = Join-Path $privateRoot 'identity.key'
if (!(Test-Path -LiteralPath $identityPath)) { throw 'Private P2P identity missing.' }
[IO.Directory]::CreateDirectory((Join-Path $dataDir 'p2p')) | Out-Null
Copy-Item -LiteralPath $identityPath -Destination (Join-Path $dataDir 'p2p/identity.key')
& docker run -d --name $container --restart unless-stopped --privileged --env-file $envPath `
    -p 18080:8080 -p 9001:9000 --mount "type=bind,source=$dataDir,target=/data" `
    -v /var/run/docker.sock:/var/run/docker.sock $image
if ($LASTEXITCODE -ne 0) { throw 'Laptop fresh run did not start; preserve its data before retrying.' }
Write-Output "Started $container with fresh signing storage and image $expectedImage"
