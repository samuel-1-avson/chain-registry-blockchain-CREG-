# Apply the public validator candidates from the approved Secret Manager
# manifest to the Sepolia chain spec before the signing ceremony.
#
# This writes only public metadata. Private keys remain in Secret Manager and
# are never read by this script. The resulting spec is still a private/alpha
# candidate until independent operators accept custody and bootnode evidence is
# attached to the roster.

[CmdletBinding()]
param(
    [string]$ManifestPath = (Join-Path $PSScriptRoot "archive\fresh-testnet-key-manifest-2026-08-01.json"),
    [string]$SpecPath = (Join-Path $PSScriptRoot "chain-spec.sepolia.json")
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) { throw "Missing key manifest: $ManifestPath" }
if (-not (Test-Path -LiteralPath $SpecPath -PathType Leaf)) { throw "Missing chain spec: $SpecPath" }

$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
$spec = Get-Content -LiteralPath $SpecPath -Raw | ConvertFrom-Json

if ($manifest.custody -notmatch "operator-controlled") {
    throw "Refusing to apply a manifest without explicit operator-controlled custody status"
}
if (@($manifest.validators).Count -lt 4) { throw "Fresh genesis requires at least four validator candidates" }
$seenEd = [System.Collections.Generic.HashSet[string]]::new()
$seenEvm = [System.Collections.Generic.HashSet[string]]::new()
$validators = @()
foreach ($candidate in @($manifest.validators)) {
    $pub = ([string]$candidate.ed25519_public_key).ToLowerInvariant()
    $evm = ([string]$candidate.evm_address).ToLowerInvariant()
    if ($pub -notmatch '^[0-9a-f]{64}$') { throw "Invalid Ed25519 public key for $($candidate.id)" }
    if ($evm -notmatch '^0x[0-9a-f]{40}$') { throw "Invalid EVM address for $($candidate.id)" }
    if (-not $seenEd.Add($pub)) { throw "Duplicate Ed25519 public key in candidate roster" }
    if (-not $seenEvm.Add($evm)) { throw "Duplicate EVM address in candidate roster" }
    $validators += [ordered]@{
        id = [string]$candidate.id
        alias = "Fresh Testnet $([string]$candidate.id)"
        pubkey = $pub
        eth_address = $evm
        stake = 100
        reputation = 100
        status = "active"
    }
}

$spec.validator_set.version = 1
$spec.validator_set.last_updated = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
$spec.validator_set.epoch_block_height = 0
$spec.validator_set.validators = $validators

$json = $spec | ConvertTo-Json -Depth 30 -Compress
Set-Content -LiteralPath $SpecPath -Value $json -Encoding utf8 -NoNewline
Write-Output "Applied $($validators.Count) public validator candidates to $SpecPath"
Write-Output "Private keys remain in Google Secret Manager; run finalize-sepolia-spec.ps1 with SPEC_SIGNING_KEY loaded from that store."
