param(
    [Parameter(Mandatory = $true)]
    [string]$SpecPath,
    [string]$EnvironmentVariable = "SPEC_SIGNING_KEY"
)

$ErrorActionPreference = "Stop"

if ($EnvironmentVariable -notmatch '^[A-Z][A-Z0-9_]{0,63}$') {
    throw "Spec-signing secret environment-variable name is invalid"
}

$signingKey = [Environment]::GetEnvironmentVariable($EnvironmentVariable, "Process")
if ([string]::IsNullOrWhiteSpace($signingKey)) {
    throw "$EnvironmentVariable is required. Load it from an approved secret manager; never place the key in this repository or a command argument."
}
$normalizedKey = $signingKey.Trim()
if ($normalizedKey -notmatch '^(0x)?[0-9a-fA-F]{64}$') {
    throw "$EnvironmentVariable must contain exactly one 32-byte Ed25519 key encoded as hexadecimal"
}

if (-not (Test-Path -LiteralPath $SpecPath -PathType Leaf)) {
    throw "Missing chain spec: $SpecPath"
}
$spec = Get-Content -LiteralPath $SpecPath -Raw | ConvertFrom-Json
$configuredPubkey = [string]$spec.signing.signing_key_pubkey_hex

# This public key corresponds to development key material previously disclosed
# in repository scripts and validator fixtures. It is permanently unsuitable
# for signing an operational chain specification.
$disclosedDevelopmentPubkeys = @(
    "0437e4adac481519cd6ae66907294c40cfcbf0bdeadd47806f6233be4bd5f82d"
)
if ($disclosedDevelopmentPubkeys -contains $configuredPubkey.ToLowerInvariant()) {
    throw "The chain spec still names a repository-disclosed development signing key. Rotate signing.signing_key_pubkey_hex through the reviewed key ceremony before signing."
}

# Do not return or print the secret. Callers pass only this variable name to
# sign_chain_spec's --key-env mode.
$EnvironmentVariable
