param(
    [string]$PrivateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7',
    [string]$IdentitySourceRoot = 'C:\Users\samue\CREG-backups\phase-c-r4',
    [string]$SpecPath = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-26-phase-c-genesis-recovery-review/chain-spec.json'),
    [string]$ExpectedGenesisHash = '0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234',
    [string]$SpecBaseUrl = 'https://spec.testnet.cregnet.dev/phase-c/r7',
    [string]$ScannerBundlePath = 'C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9',
    [string]$ScannerProfileDigest = '648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f',
    [string]$OsvSnapshotEpoch = 'osv-2026-09-26-phase-c-r8',
    [string]$ValidatorSecretProject = 'gen-lang-client-0022105784',
    [string]$RpcSecretProject = 'fiffi-500411',
    [string]$RpcProfileSecret = 'creg-sepolia-rpc-profile'
)

$ErrorActionPreference = 'Stop'
$spec = Get-Content -LiteralPath $SpecPath -Raw | ConvertFrom-Json
if ($spec.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or
    $spec.genesis_hash -ne $ExpectedGenesisHash -or
    $spec.consensus_params.consensus_encoding -ne 'creg-sorted-json-v1') {
    throw 'The supplied spec is not the reviewed r7 chain identity.'
}
if ($spec.signing.detached_signature_url -ne "$SpecBaseUrl/chain-spec.json.sig" -or
    $SpecBaseUrl -ne 'https://spec.testnet.cregnet.dev/phase-c/r7') {
    throw 'The r7 signed spec URL does not match this environment profile.'
}
if ($spec.validator_set.validators.Count -ne 4 -or $spec.bootnodes.Count -ne 2) {
    throw 'The r7 spec must contain four validators and two bootnodes.'
}
$stakingAddress = [string]$spec.contracts.staking
if ($stakingAddress -notmatch '^0x[0-9a-fA-F]{40}$') {
    throw 'The r7 signed spec must contain a valid Sepolia staking contract address.'
}
if ($ScannerProfileDigest -notmatch '^[0-9a-f]{64}$' -or
    $OsvSnapshotEpoch -notmatch '^[A-Za-z0-9._-]{1,128}$') {
    throw 'Invalid pinned scanner profile or OSV epoch.'
}

function Read-SecretValue([string]$Project, [string]$Secret) {
    $value = (& gcloud secrets versions access latest "--secret=$Secret" "--project=$Project" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($value)) {
        throw "Could not securely read required secret $Secret from project $Project."
    }
    return $value
}

function Invoke-Rpc([string]$Url, [string]$Method, [object[]]$Params) {
    $body = @{ jsonrpc = '2.0'; id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 8 -Compress
    try {
        $response = Invoke-RestMethod -Uri $Url -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 20
    } catch {
        throw "Configured Sepolia RPC provider failed $Method."
    }
    if ($response.error -or ($null -eq $response.result -and $Method -ne 'eth_getLogs')) {
        throw "Configured Sepolia RPC provider rejected $Method."
    }
    return [pscustomobject]@{ Result = $response.result }
}

$rpcProfileRaw = Read-SecretValue -Project $RpcSecretProject -Secret $RpcProfileSecret
try { $rpcProfile = $rpcProfileRaw | ConvertFrom-Json } catch { throw 'The saved R7 RPC profile secret is not valid JSON.' }
$rpcPrimary = [string]$rpcProfile.primary
$rpcFallbacks = @($rpcProfile.fallbacks | ForEach-Object { [string]$_ })
$gethUrl = [string]$rpcProfile.geth_url
if ($rpcFallbacks.Count -ne 2 -or [string]::IsNullOrWhiteSpace($gethUrl)) {
    throw 'The R7 RPC profile requires a primary, two fallbacks, and its self-hosted Geth endpoint.'
}
$rpcUrls = @($rpcPrimary) + $rpcFallbacks
$rpcUriList = @()
foreach ($rpcUrl in $rpcUrls) {
    $parsed = $null
    if (-not [Uri]::TryCreate($rpcUrl, [UriKind]::Absolute, [ref]$parsed) -or $parsed.Scheme -notin @('https', 'http')) {
        throw 'Every R7 Sepolia RPC endpoint must be a valid HTTP(S) URL.'
    }
    $rpcUriList += $parsed
}
$rpcHosts = @($rpcUriList | ForEach-Object { $_.Host.ToLowerInvariant() })
$gethUri = $null
$isGethUriValid = [Uri]::TryCreate($gethUrl, [UriKind]::Absolute, [ref]$gethUri)
$gethAddress = $null
$isGethIp = $isGethUriValid -and [System.Net.IPAddress]::TryParse($gethUri.Host, [ref]$gethAddress)
$httpUrls = @($rpcUriList | Where-Object { $_.Scheme -eq 'http' } | ForEach-Object { $_.AbsoluteUri.TrimEnd('/') })
if (@($rpcHosts | Select-Object -Unique).Count -ne 3 -or
    $rpcPrimary -ne 'https://ethereum-sepolia-rpc.publicnode.com' -or
    ($rpcFallbacks -notcontains 'https://sepolia.gateway.tenderly.co') -or
    ($rpcFallbacks -notcontains $gethUrl) -or
    -not $isGethIp -or $gethUri.Scheme -ne 'http' -or $gethUri.Port -ne 8545 -or
    $httpUrls.Count -ne 1 -or $httpUrls[0] -ne $gethUrl.TrimEnd('/')) {
    throw 'The R7 RPC profile must contain PublicNode, Tenderly and its HTTP-IP-filtered GCP Geth endpoint.'
}

# Verify live provider agreement before writing private node environments.
# URLs, including any Google API key, are never written to the console.
$heads = @()
foreach ($rpcUrl in $rpcUrls) {
    $chainHex = [string](Invoke-Rpc -Url $rpcUrl -Method 'eth_chainId' -Params @()).Result
    if ($chainHex -ne '0xaa36a7') { throw 'An RPC provider returned a non-Sepolia chain ID.' }
    $headHex = [string](Invoke-Rpc -Url $rpcUrl -Method 'eth_blockNumber' -Params @()).Result
    $heads += [Convert]::ToInt64($headHex.Substring(2), 16)
}
if (($heads | Measure-Object -Maximum).Maximum - ($heads | Measure-Object -Minimum).Minimum -gt 64) {
    throw 'The self-hosted Sepolia RPC is more than 64 blocks behind the public providers.'
}
$commonBlock = [long](($heads | Measure-Object -Minimum).Minimum) - 2
if ($commonBlock -lt 50) { throw 'Sepolia RPC providers returned an invalid common block height.' }
$commonHex = '0x' + [Convert]::ToString($commonBlock, 16)
$fromHex = '0x' + [Convert]::ToString(($commonBlock - 49), 16)
$blockHashes = @()
$stakingLogs = @()
foreach ($rpcUrl in $rpcUrls) {
    $block = (Invoke-Rpc -Url $rpcUrl -Method 'eth_getBlockByNumber' -Params @($commonHex, $false)).Result
    $blockHashes += ([string]$block.hash).ToLowerInvariant()
    $logs = @((Invoke-Rpc -Url $rpcUrl -Method 'eth_getLogs' -Params @(@{
        fromBlock = $fromHex
        toBlock = $commonHex
        address = $stakingAddress
    })).Result)
    $stakingLogs += ,(@($logs | ForEach-Object {
        '{0}:{1}:{2}:{3}:{4}' -f $_.blockNumber, $_.logIndex, $_.transactionHash, $_.data, ($_.topics -join ',')
    } | Sort-Object) -join '|')
}
if (@($blockHashes | Select-Object -Unique).Count -ne 1 -or
    @($stakingLogs | Select-Object -Unique).Count -ne 1) {
    throw 'Sepolia RPC providers did not agree on the common block and staking logs.'
}
"R7 RPC readiness: three distinct providers agree on Sepolia block $commonBlock and a 50-block staking-log range."

$bundle = (Resolve-Path -LiteralPath $ScannerBundlePath).Path
$profileOutput = @(& (Join-Path $PSScriptRoot 'compute-scanner-profile.ps1') `
    -RulesDirectory (Join-Path $bundle 'rules') `
    -ThreatDatabasePath (Join-Path $bundle 'known_malicious_hashes.json') `
    -OsvSnapshotPath (Join-Path $bundle 'osv_snapshot.json') `
    -OsvEpoch $OsvSnapshotEpoch -BlockCriticalOsv $true)
$computedProfile = ($profileOutput | Where-Object { $_ -like 'CREG_SCANNER_PROFILE_DIGEST=*' } | Select-Object -First 1) -replace '^CREG_SCANNER_PROFILE_DIGEST=', ''
if ($computedProfile -ne $ScannerProfileDigest) {
    throw 'Local scanner bundle does not match the pinned r7 profile digest.'
}

$slots = @(
    @{ Slot = 'v1'; Id = 'validator-1'; Validator = $true; Secret = 'creg-fresh-testnet-validator-1-ed25519'; Peers = 'http://34.156.122.19:8080,http://34.124.165.97:8080' },
    @{ Slot = 'v2'; Id = 'validator-2'; Validator = $true; Secret = 'creg-fresh-testnet-validator-2-ed25519'; Peers = 'http://34.156.122.19:8080,http://34.124.165.97:8080' },
    @{ Slot = 'v3'; Id = 'validator-3'; Validator = $true; Secret = 'creg-fresh-testnet-validator-3-ed25519'; Peers = 'http://10.96.2.2:8080,http://10.96.4.2:8080' },
    @{ Slot = 'v4'; Id = 'validator-4'; Validator = $true; Secret = 'creg-fresh-testnet-validator-4-ed25519'; Peers = 'http://10.96.1.2:8080,http://10.96.4.2:8080' },
    @{ Slot = 'o1'; Id = 'observer-1'; Validator = $false; Peers = 'http://34.156.122.19:8080,http://34.124.165.97:8080' },
    @{ Slot = 'o2'; Id = 'observer-2'; Validator = $false; Peers = 'http://34.156.122.19:8080,http://34.124.165.97:8080' },
    @{ Slot = 'o3'; Id = 'observer-3'; Validator = $false; Peers = 'http://10.96.1.2:8080,http://10.96.2.2:8080' },
    @{ Slot = 'o4'; Id = 'observer-4'; Validator = $false; Peers = 'http://10.96.1.2:8080,http://10.96.2.2:8080' }
)

$slotDirectories = @{}
$peerIds = @{}
$peerReader = Join-Path $PSScriptRoot 'read-peer-identity.js'
foreach ($slot in $slots) {
    $slotDir = Join-Path $PrivateRoot $slot.Slot
    $sourceIdentity = Join-Path (Join-Path $IdentitySourceRoot $slot.Slot) 'identity.key'
    $targetIdentity = Join-Path $slotDir 'identity.key'
    if (-not (Test-Path -LiteralPath $sourceIdentity)) {
        throw "Missing preserved P2P identity for $($slot.Slot)."
    }
    $peerId = (& node $peerReader $sourceIdentity 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $peerId -notmatch '^12D3KooW[A-Za-z0-9]+$') {
        throw "Could not validate the public P2P identity for $($slot.Slot)."
    }
    if ($peerIds.Values -contains $peerId) { throw 'Every r7 node must have a distinct P2P identity.' }
    $peerIds[$slot.Slot] = $peerId
    $bootnodeId = switch ($slot.Slot) { 'v3' { 'phase-c-validator-3' }; 'o4' { 'phase-c-observer-4' }; default { $null } }
    if ($bootnodeId) {
        $signedBootnode = @($spec.bootnodes | Where-Object { $_.id -eq $bootnodeId })
        if ($signedBootnode.Count -ne 1 -or ($signedBootnode[0].multiaddr -split '/p2p/')[-1] -ne $peerId) {
            throw "Preserved P2P identity does not match the signed r7 bootnode for $($slot.Slot)."
        }
    }
    if (Test-Path -LiteralPath (Join-Path $slotDir 'node.env')) {
        throw "Private environment already exists for $($slot.Slot); refusing to overwrite it."
    }
    if (Test-Path -LiteralPath $targetIdentity) {
        $sourceHash = (Get-FileHash -LiteralPath $sourceIdentity -Algorithm SHA256).Hash
        $targetHash = (Get-FileHash -LiteralPath $targetIdentity -Algorithm SHA256).Hash
        if ($sourceHash -ne $targetHash) {
            throw "P2P identity differs from preserved identity for $($slot.Slot)."
        }
    }
    $slotDirectories[$slot.Slot] = $slotDir
}

$validatorKeys = @{}
$publicKeyScript = Join-Path $PSScriptRoot 'ed25519-public-from-env.js'
foreach ($slot in $slots | Where-Object { $_.Validator }) {
    $key = (& gcloud secrets versions access latest "--secret=$($slot.Secret)" "--project=$ValidatorSecretProject" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $key -notmatch '^(0x)?[0-9a-fA-F]{64}$') {
        $key = $null
        throw "Could not securely validate the signing secret for $($slot.Id)."
    }
    $savedSeed = $env:CREG_PHASE_C_SEED
    try {
        $env:CREG_PHASE_C_SEED = $key
        $derivedPublicKey = (& node $publicKeyScript 2>$null | Out-String).Trim().ToLowerInvariant()
    } finally {
        $env:CREG_PHASE_C_SEED = $savedSeed
    }
    $key = $key.Trim()
    $expectedPublicKey = ($spec.validator_set.validators | Where-Object { $_.id -eq $slot.Id }).pubkey.ToLowerInvariant()
    if ($derivedPublicKey -ne $expectedPublicKey) {
        $key = $null
        throw "Signing secret does not match the r7 roster for $($slot.Id)."
    }
    $validatorKeys[$slot.Slot] = $key
    $key = $null
}

foreach ($slot in $slots) {
    $slotDir = $slotDirectories[$slot.Slot]
    New-Item -ItemType Directory -Path $slotDir -Force | Out-Null
    $identityPath = Join-Path $slotDir 'identity.key'
    if (-not (Test-Path -LiteralPath $identityPath)) {
        Copy-Item -LiteralPath (Join-Path (Join-Path $IdentitySourceRoot $slot.Slot) 'identity.key') -Destination $identityPath
    }

    # A signed bootnode must not try to dial itself. Keep the chain-spec
    # bootnodes as the source of truth, while removing this node's own public
    # libp2p identity from its seed list.
    $selfPeerId = $peerIds[$slot.Slot]
    $slotSeeds = @($spec.bootnodes | Where-Object {
        $seedPeerId = ($_.multiaddr -split '/p2p/')[-1]
        $seedPeerId -ne $selfPeerId
    })
    if ($slotSeeds.Count -lt 1) {
        throw "No external signed bootnode remains for $($slot.Slot)."
    }
    $seeds = ($slotSeeds | ForEach-Object { $_.multiaddr }) -join ','
    if (@($slotSeeds | Where-Object {
        (($_.multiaddr -split '/p2p/')[-1]) -eq $selfPeerId
    }).Count -gt 0) {
        throw "Self peer remained in the seed list for $($slot.Slot)."
    }

    $role = if ($slot.Validator) { 'validator' } else { 'observer' }
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('CREG_TESTNET=true')
    $lines.Add("CREG_NODE_ID=$($slot.Id)")
    $lines.Add('CREG_DATA_DIR=/data')
    $lines.Add("CREG_IS_VALIDATOR=$($slot.Validator.ToString().ToLowerInvariant())")
    $lines.Add("CREG_NODE_ROLE=$role")
    $lines.Add("CREG_ETH_RPC=$rpcPrimary")
    $lines.Add("CREG_ETH_RPC_FALLBACKS=$($rpcFallbacks -join ',')")
    $lines.Add('CREG_EXPECTED_L1_CHAIN_ID=11155111')
    $lines.Add("CREG_CHAIN_ID=$($spec.chain_id)")
    $lines.Add("CREG_GENESIS_HASH=$($spec.genesis_hash)")
    $lines.Add("CREG_CHAIN_SPEC_URL=$SpecBaseUrl/chain-spec.json")
    $lines.Add("CREG_SPEC_SIGNATURE_URL=$SpecBaseUrl/chain-spec.json.sig")
    $lines.Add('CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev')
    $lines.Add('CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000')
    $lines.Add("CREG_P2P_SEEDS=$seeds")
    $lines.Add("CREG_PEERS=$($slot.Peers)")
    $lines.Add('CREG_LISTEN=0.0.0.0:8080')
    $lines.Add('CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false')
    $lines.Add('CREG_PBFT_TIMEOUT=30')
    $lines.Add('CREG_DEV_SANDBOX=false')
    $lines.Add("CREG_PUBLIC_VALIDATOR=$($slot.Validator.ToString().ToLowerInvariant())")
    $lines.Add('CREG_YARA_RULES_DIR=/scanner/rules')
    $lines.Add('CREG_THREAT_DB_PATH=/scanner/known_malicious_hashes.json')
    $lines.Add('CREG_OSV_CONSENSUS=true')
    $lines.Add('CREG_OSV_BLOCK_CRITICAL=true')
    $lines.Add('CREG_OSV_LIVE_FALLBACK=false')
    $lines.Add('CREG_OSV_SNAPSHOT_PATH=/scanner/osv_snapshot.json')
    $lines.Add("CREG_OSV_SNAPSHOT_EPOCH=$OsvSnapshotEpoch")
    $lines.Add("CREG_SCANNER_PROFILE_DIGEST=$ScannerProfileDigest")
    $lines.Add("CREG_OPERATOR_API_KEY=$([Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant())")
    if ($slot.Validator) { $lines.Add("CREG_VALIDATOR_KEY=$($validatorKeys[$slot.Slot])") }

    $envPath = Join-Path $slotDir 'node.env'
    [IO.File]::WriteAllText($envPath, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
    "{0}: prepared private r7 environment; public signer roster matched; scanner profile pinned" -f $slot.Slot
}
$validatorKeys.Clear()
