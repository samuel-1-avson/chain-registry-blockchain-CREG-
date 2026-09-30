param([string]$SpecPath = (Join-Path $PSScriptRoot 'chain-spec.json'))

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not (Test-Path -LiteralPath $SpecPath)) { throw 'Candidate spec is missing.' }
$key = (& gcloud secrets versions access latest --secret=creg-testnet-spec-signing-key --project=gen-lang-client-0022105784)
if ($LASTEXITCODE -ne 0) { throw 'Could not load the chain-spec signing key.' }
$key = ([string]$key).Trim()
if ($key -notmatch '^(0x)?[0-9a-fA-F]{64}$') { throw 'Chain-spec signing key has invalid format.' }
$env:SPEC_SIGNING_KEY = $key
$key = $null
try {
    Push-Location $repoRoot
    $signature = (& cargo run -q --example sign_chain_spec --package common -- $SpecPath --key-env SPEC_SIGNING_KEY | Select-Object -Last 1)
    if ($LASTEXITCODE -ne 0 -or $signature -notmatch '^[0-9a-fA-F]{128}$') {
        throw 'Candidate chain-spec signature generation failed.'
    }
    $signaturePath = "$SpecPath.sig"
    [System.IO.File]::WriteAllText($signaturePath, $signature.Trim(), [System.Text.UTF8Encoding]::new($false))
    & cargo run -q --example verify_chain_spec --package common -- $SpecPath $signaturePath
    if ($LASTEXITCODE -ne 0) { throw 'Signed candidate failed Rust signature verification.' }
    Write-Output "Signed candidate: $SpecPath"
} finally {
    [Environment]::SetEnvironmentVariable('SPEC_SIGNING_KEY', $null, 'Process')
    Pop-Location
}
