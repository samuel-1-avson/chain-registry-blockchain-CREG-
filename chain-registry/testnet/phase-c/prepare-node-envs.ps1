param(
    [string]$PrivateRoot = 'C:\Users\samue\CREG-backups\phase-c-candidate',
    [string]$SpecPath = (Join-Path $PSScriptRoot 'chain-spec.json'),
    [string]$ExpectedGenesisHash = '0x6255f69667b13727e618b634993571587adf2101b70ac3dcd73b3fce325ba18a',
    [string]$SpecBaseUrl = 'https://spec.testnet.cregnet.dev/phase-c'
)

$ErrorActionPreference = 'Stop'
$project = 'gen-lang-client-0022105784'
$spec = Get-Content -LiteralPath $SpecPath -Raw | ConvertFrom-Json
if ($spec.consensus_params.consensus_encoding -ne 'creg-sorted-json-v1') {
    throw 'New Phase C environments require the signed sorted JSON consensus encoding.'
}
if ($spec.chain_id -notmatch '^creg-phase-c-testnet-[0-9]{4}-[0-9]{2}-[0-9]{2}(?:-[a-z0-9]+)?$') {
    throw 'Unexpected Phase C chain identity.'
}
if ($ExpectedGenesisHash -notmatch '^0x[0-9a-fA-F]{64}$' -or $spec.genesis_hash -ne $ExpectedGenesisHash) {
    throw 'Unexpected Phase C genesis hash.'
}
if ($SpecBaseUrl -notmatch '^https://spec\.testnet\.cregnet\.dev/[a-z0-9-]+(?:/[a-z0-9-]+)*$' -or
    $spec.signing.detached_signature_url -ne "$SpecBaseUrl/chain-spec.json.sig") {
    throw 'Spec endpoint must match the signed signature URL.'
}
if ($spec.validator_set.validators.Count -ne 4 -or $spec.bootnodes.Count -ne 2) {
    throw 'Phase C needs four signed validators and two signed bootnodes.'
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

foreach ($node in $slots) {
    $slotDir = Join-Path $PrivateRoot $node.Slot
    $identityPath = Join-Path $slotDir 'identity.key'
    if (-not (Test-Path -LiteralPath $identityPath)) { throw "Missing persistent identity for $($node.Slot)." }
    if (Test-Path -LiteralPath (Join-Path $slotDir 'node.env')) { throw "Private environment already exists for $($node.Slot)." }
    $peerReader = Join-Path $PSScriptRoot 'read-peer-identity.js'
    $selfPeerId = (& node $peerReader $identityPath 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $selfPeerId -notmatch '^12D3KooW[A-Za-z0-9]+$') {
        throw "Could not validate the public P2P identity for $($node.Slot)."
    }
    $slotSeeds = @($spec.bootnodes | Where-Object {
        $seedPeerId = ($_.multiaddr -split '/p2p/')[-1]
        $seedPeerId -ne $selfPeerId
    })
    if ($slotSeeds.Count -lt 1) { throw "No external signed bootnode remains for $($node.Slot)." }
    $seeds = ($slotSeeds | ForEach-Object { $_.multiaddr }) -join ','
    if (@($slotSeeds | Where-Object { (($_.multiaddr -split '/p2p/')[-1]) -eq $selfPeerId }).Count -gt 0) {
        throw "Self peer remained in the seed list for $($node.Slot)."
    }
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('CREG_TESTNET=true')
    $lines.Add("CREG_NODE_ID=$($node.Id)")
    $lines.Add('CREG_DATA_DIR=/data')
    $lines.Add("CREG_IS_VALIDATOR=$($node.Validator.ToString().ToLowerInvariant())")
    $role = if ($node.Validator) { 'validator' } else { 'observer' }
    $lines.Add("CREG_NODE_ROLE=$role")
    $lines.Add('CREG_ETH_RPC=https://ethereum-sepolia-rpc.publicnode.com')
    $lines.Add('CREG_EXPECTED_L1_CHAIN_ID=11155111')
    $lines.Add("CREG_CHAIN_ID=$($spec.chain_id)")
    $lines.Add("CREG_GENESIS_HASH=$($spec.genesis_hash)")
    $lines.Add("CREG_CHAIN_SPEC_URL=$SpecBaseUrl/chain-spec.json")
    $lines.Add("CREG_SPEC_SIGNATURE_URL=$SpecBaseUrl/chain-spec.json.sig")
    $lines.Add('CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev')
    $lines.Add('CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000')
    $lines.Add("CREG_P2P_SEEDS=$seeds")
    $lines.Add("CREG_PEERS=$($node.Peers)")
    $lines.Add('CREG_LISTEN=0.0.0.0:8080')
    $lines.Add('CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false')
    $lines.Add('CREG_PBFT_TIMEOUT=30')
    $lines.Add('CREG_DEV_SANDBOX=false')
    $lines.Add("CREG_PUBLIC_VALIDATOR=$($node.Validator.ToString().ToLowerInvariant())")
    $lines.Add("CREG_OPERATOR_API_KEY=$([Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant())")
    if ($node.Validator) {
        $validatorKey = & gcloud secrets versions access latest --secret=$($node.Secret) --project=$project
        if ($LASTEXITCODE -ne 0) { throw "Cannot read signing key for $($node.Id)." }
        $validatorKey = ([string]$validatorKey).Trim()
        if ($validatorKey -notmatch '^(0x)?[0-9a-fA-F]{64}$') { throw "Invalid signing key format for $($node.Id)." }
        $lines.Add("CREG_VALIDATOR_KEY=$validatorKey")
        $validatorKey = $null
    }
    $envPath = Join-Path $slotDir 'node.env'
    [IO.File]::WriteAllText($envPath, (($lines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
    Write-Output "$($node.Slot): prepared private node.env and persistent P2P identity"
}
