param(
    [string]$Validator3PeerId,
    [string]$Observer4PeerId,
    [string]$ChainId = 'creg-phase-c-testnet-2026-09-25',
    [string]$SpecBaseUrl = 'https://spec.testnet.cregnet.dev/phase-c',
    [string]$DeploymentManifestPath = (Join-Path $PSScriptRoot 'deployment-manifest.json'),
    [string]$OutputPath = (Join-Path $PSScriptRoot 'chain-spec.json')
)

$ErrorActionPreference = 'Stop'
if ($ChainId -notmatch '^creg-phase-c-testnet-[0-9]{4}-[0-9]{2}-[0-9]{2}(?:-[a-z0-9]+)?$' -or
    $SpecBaseUrl -notmatch '^https://spec\.testnet\.cregnet\.dev/[a-z0-9-]+(?:/[a-z0-9-]+)*$') {
    throw 'Provide an explicit Phase C chain identity and HTTPS spec directory.'
}
if (Test-Path -LiteralPath $OutputPath) { throw 'Preserve the existing signed spec; choose a new output path.' }
if ($Validator3PeerId -notmatch '^12D3KooW[1-9A-HJ-NP-Za-km-z]+$' -or
    $Observer4PeerId -notmatch '^12D3KooW[1-9A-HJ-NP-Za-km-z]+$' -or
    $Validator3PeerId -eq $Observer4PeerId) {
    throw 'Provide two distinct real libp2p Ed25519 bootnode peer IDs.'
}

$testnetDir = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $testnetDir
$manifest = Get-Content -LiteralPath (Join-Path $testnetDir 'archive/fresh-testnet-key-manifest-2026-08-01.json') -Raw | ConvertFrom-Json
$spec = Get-Content -LiteralPath (Join-Path $testnetDir 'chain-spec.sepolia.json') -Raw | ConvertFrom-Json
if (@($manifest.validators).Count -ne 4) { throw 'Phase C requires exactly four validator keys.' }
if (-not (Test-Path -LiteralPath $DeploymentManifestPath)) {
    throw 'Fresh Phase C Sepolia deployment manifest is missing; refusing to reuse the old contracts.'
}
$deployment = Get-Content -LiteralPath $DeploymentManifestPath -Raw | ConvertFrom-Json
if ([string]$deployment.chainId -ne '11155111') { throw 'Phase C deployment manifest must target Sepolia.' }
$spec.contracts.appeal = [string]$deployment.appeal
$spec.contracts.creg_token = [string]$deployment.cregToken
$spec.contracts.governance = [string]$deployment.governance
$spec.contracts.registry = [string]$deployment.registry
$spec.contracts.reputation = [string]$deployment.reputation
$spec.contracts.staking = [string]$deployment.staking
$spec.contracts.validator_rewards = [string]$deployment.validatorRewards
$spec.contracts.vrf = [string]$deployment.vrf
$spec.contracts.zk_verifier = [string]$deployment.zkVerifier
if ($spec.feature_flags.zk_validation -or $spec.contracts.vrf -ne '0x0000000000000000000000000000000000000000' -or
    $spec.contracts.zk_verifier -ne '0x0000000000000000000000000000000000000000') {
    throw 'Phase C requires disabled ZK validation and no placeholder VRF/ZK deployment.'
}

function Invoke-SepoliaRpc([string]$Method, [object[]]$Parameters) {
    $body = @{ jsonrpc = '2.0'; method = $Method; params = $Parameters; id = 1 } | ConvertTo-Json -Compress -Depth 10
    $response = Invoke-RestMethod -Method Post -Uri $rpcUrl -ContentType 'application/json' -Body $body
    if ($response.error -or $null -eq $response.result) {
        throw "Sepolia RPC $Method failed: $($response.error.message)"
    }
    return [string]$response.result
}

function ConvertFrom-HexWord([string]$HexWord) {
    if ($HexWord -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid uint256 response from Sepolia.' }
    return [System.Numerics.BigInteger]::Parse(
        '0' + $HexWord,
        [System.Globalization.NumberStyles]::AllowHexSpecifier
    )
}

function Invoke-SepoliaCall([string]$Contract, [string]$Data, [string]$BlockTag) {
    $result = Invoke-SepoliaRpc 'eth_call' @(@{ to = $Contract; data = $Data }, $BlockTag)
    if ($result -notmatch '^0x(?:[0-9a-fA-F]{64})+$') { throw "Malformed eth_call response from $Contract." }
    return $result.Substring(2)
}

$rpcUrl = if ($env:SEPOLIA_RPC_URL) { $env:SEPOLIA_RPC_URL } else { 'https://ethereum-sepolia-rpc.publicnode.com' }
$l1ChainId = Invoke-SepoliaRpc 'eth_chainId' @()
if ($l1ChainId -ne '0xaa36a7') { throw "Expected Sepolia chain ID 11155111, got $l1ChainId." }
$headHex = Invoke-SepoliaRpc 'eth_blockNumber' @()
if ($headHex -notmatch '^0x[0-9a-fA-F]+$') { throw 'Could not read the real Sepolia head.' }
$head = [Convert]::ToUInt64($headHex.Substring(2), 16)
$finalityDepth = [ulong]$spec.l1.min_finality_blocks
if ($head -lt $finalityDepth) { throw 'Sepolia head is below the required finality depth.' }
$finalizedStart = $head - $finalityDepth
$blockTag = '0x' + $finalizedStart.ToString('x')
$staking = [string]$spec.contracts.staking
if ($staking -notmatch '^0x[0-9a-fA-F]{40}$') { throw 'Invalid staking contract address in chain spec.' }
$minimumWei = [System.Numerics.BigInteger]::Parse([string]$spec.consensus_params.min_validator_stake_wei)
$minimumHex = Invoke-SepoliaCall $staking '0x017ddd35' $blockTag
$onchainMinimum = ConvertFrom-HexWord $minimumHex.Substring(0, 64)
if ($minimumWei -ne $onchainMinimum) {
    throw "Chain-spec validator minimum $minimumWei differs from live Sepolia staking minimum $onchainMinimum at block $finalizedStart."
}

$tokenUnit = [System.Numerics.BigInteger]::Pow([System.Numerics.BigInteger]::new(10), 18)
$validatorEntries = @()
$addresses = @{}
foreach ($validator in $manifest.validators) {
    $address = [string]$validator.evm_address
    if ($address -notmatch '^0x[0-9a-fA-F]{40}$') { throw "Invalid EVM address for $($validator.id)." }
    if ($addresses.ContainsKey($address)) { throw "Duplicate validator EVM address: $address" }
    $addresses[$address] = $true
    $data = '0xfa52c7d8' + $address.Substring(2).ToLowerInvariant().PadLeft(64, '0')
    $entry = Invoke-SepoliaCall $staking $data $blockTag
    if ($entry.Length -ne 384) { throw "Malformed staking entry for $($validator.id)." }
    $stakeWei = ConvertFrom-HexWord $entry.Substring(0, 64)
    $state = ConvertFrom-HexWord $entry.Substring(64, 64)
    if ($state -ne 2 -or $stakeWei -lt $minimumWei) {
        throw "$($validator.id) ($address) is not an active, sufficiently staked Sepolia validator at block $finalizedStart (state=$state, stake_wei=$stakeWei). Candidate genesis refused."
    }
    $wholeTokens = [System.Numerics.BigInteger]::Divide($stakeWei, $tokenUnit)
    if ($wholeTokens -gt [ulong]::MaxValue) { throw "Stake for $($validator.id) exceeds the chain-spec u64 limit." }
    $validatorEntries += [ordered]@{
        id = [string]$validator.id
        alias = "Phase C $([string]$validator.id)"
        pubkey = [string]$validator.ed25519_public_key
        eth_address = $address
        stake = [ulong]$wholeTokens
        reputation = 100
        status = 'active'
    }
}

$now = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')

$spec.chain_id = $ChainId
$spec.consensus_params | Add-Member -NotePropertyName consensus_encoding -NotePropertyValue 'creg-sorted-json-v1' -Force
$spec.genesis_time = $now
$spec.genesis_hash = '0x' + ('0' * 64)
$spec.bootnodes = @(
    [ordered]@{
        id = 'phase-c-validator-3'
        operator = 'CREG GCP testnet operator'
        region = 'europe-west1-b'
        multiaddr = "/ip4/34.156.122.19/tcp/9000/p2p/$Validator3PeerId"
    },
    [ordered]@{
        id = 'phase-c-observer-4'
        operator = 'CREG GCP testnet operator'
        region = 'africa-south1-a'
        multiaddr = "/ip4/34.35.191.131/tcp/9000/p2p/$Observer4PeerId"
    }
)
$spec.signing.signing_key_pubkey_hex = [string]$manifest.spec_signing.public_key
$spec.signing.detached_signature_url = "$SpecBaseUrl/chain-spec.json.sig"
$spec.validator_set.version = 1
$spec.validator_set.last_updated = $now
$spec.validator_set.epoch_block_height = $finalizedStart
$spec.validator_set.validators = $validatorEntries

$encoding = [System.Text.UTF8Encoding]::new($false)
[System.IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
[System.IO.File]::WriteAllText($OutputPath, ($spec | ConvertTo-Json -Depth 30 -Compress), $encoding)
Push-Location $repoRoot
try {
    $hashOutput = & cargo run -q --example compute_genesis_hash --package common -- $OutputPath
    if ($LASTEXITCODE -ne 0) { throw 'Real chain-spec genesis hash computation failed.' }
} finally {
    Pop-Location
}
$genesisHash = ($hashOutput | Select-Object -Last 1).Trim()
if ($genesisHash -notmatch '^0x[0-9a-fA-F]{64}$') { throw "Unexpected genesis hash output: $genesisHash" }
$spec.genesis_hash = $genesisHash
[System.IO.File]::WriteAllText($OutputPath, ($spec | ConvertTo-Json -Depth 30 -Compress), $encoding)
Write-Output "Candidate spec: $OutputPath"
Write-Output "Genesis hash: $genesisHash"
Write-Output "Sepolia event start: $finalizedStart"
