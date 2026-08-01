# Provision fresh fresh-testnet key material without writing private values to disk.
#
# Private values are generated from the OS CSPRNG and streamed directly to
# Google Secret Manager. The script prints only secret IDs and public metadata.
# It intentionally does not modify the chain spec: validator admission still
# requires an approved roster and independently controlled operators.

[CmdletBinding()]
param(
    [string]$ProjectId,
    [ValidateRange(1, 10)]
    [int]$ValidatorCount = 4,
    [switch]$RotateExisting
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent (Split-Path -Parent $scriptDir)

function Read-HostingProject {
    $hosting = Join-Path $scriptDir "hosting.env"
    if (-not (Test-Path -LiteralPath $hosting)) {
        throw "Missing $hosting; set -ProjectId explicitly or create hosting.env from hosting.env.example"
    }
    $line = Get-Content -LiteralPath $hosting | Where-Object { $_ -match '^\s*GCP_PROJECT\s*=' } | Select-Object -First 1
    if (-not $line) { throw "GCP_PROJECT is not configured in $hosting" }
    $value = ($line -split '=', 2)[1].Trim()
    if ([string]::IsNullOrWhiteSpace($value)) { throw "GCP_PROJECT is empty in $hosting" }
    return $value
}

if ([string]::IsNullOrWhiteSpace($ProjectId)) { $ProjectId = Read-HostingProject }
$gcloud = (Get-Command gcloud.cmd -ErrorAction SilentlyContinue).Source
if (-not $gcloud) { throw "gcloud.cmd is required" }
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { throw "cargo is required to derive Ed25519 public keys" }
$cast = (Get-Command cast -ErrorAction SilentlyContinue).Source
if (-not $cast) {
    $candidate = Join-Path $repoRoot "testnet\.tools\foundry\cast.exe"
    if (Test-Path -LiteralPath $candidate) { $cast = $candidate }
}
if (-not $cast) { throw "cast is required to derive EVM addresses" }

function Invoke-GcloudText {
    param([string[]]$Arguments)
    $output = & $gcloud @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "gcloud failed: $output" }
    return $output.Trim()
}

function Test-SecretExists {
    param([string]$SecretId)
    & $gcloud secrets describe $SecretId --project=$ProjectId 2>$null | Out-Null
    return ($LASTEXITCODE -eq 0)
}

function Publish-SecretBytes {
    param(
        [Parameter(Mandatory)][string]$SecretId,
        [Parameter(Mandatory)][byte[]]$Bytes,
        [Parameter(Mandatory)][string]$Role
    )

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $gcloud
    $psi.Arguments = "secrets versions add $SecretId --project=$ProjectId --data-file=-"
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $psi
    [void]$process.Start()
    try {
        $process.StandardInput.BaseStream.Write($Bytes, 0, $Bytes.Length)
        $process.StandardInput.Close()
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        $exitCode = $process.ExitCode
    } finally {
        $process.Dispose()
    }
    if ($exitCode -ne 0) { throw "Secret Manager write failed for $Role ($SecretId): $stderr$stdout" }
}

function Ensure-Secret {
    param([string]$SecretId, [string]$Role, [byte[]]$Bytes, [hashtable]$Labels)
    $exists = Test-SecretExists -SecretId $SecretId
    if ($exists -and -not $RotateExisting) {
        throw "Secret $SecretId already exists; pass -RotateExisting only after an explicit rotation decision"
    }
    if (-not $exists) {
        $labelArgs = @()
        foreach ($entry in $Labels.GetEnumerator()) { $labelArgs += "{0}={1}" -f $entry.Key, $entry.Value }
        Invoke-GcloudText -Arguments @(
            "secrets", "create", $SecretId,
            "--project=$ProjectId",
            "--replication-policy=automatic",
            "--labels=$($labelArgs -join ',')"
        ) | Out-Null
    }
    Publish-SecretBytes -SecretId $SecretId -Bytes $Bytes -Role $Role
}

function New-RandomBytes([int]$Count) {
    $bytes = [byte[]]::new($Count)
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return $bytes
}

function New-Ed25519Candidate {
    param([string]$Role, [string]$SecretId)
    $seed = New-RandomBytes 32
    $seedHex = [Convert]::ToHexString($seed).ToLowerInvariant()
    $envName = "CREG_KEYGEN_TEMP"
    [Environment]::SetEnvironmentVariable($envName, $seedHex, "Process")
    try {
        Push-Location $repoRoot
        try {
            $lines = & $cargo run -q --example ed25519_pubkey_from_secret -p common -- --key-env $envName 2>&1
            if ($LASTEXITCODE -ne 0) { throw "Ed25519 public-key derivation failed for ${Role}: $($lines -join ' ')" }
            $pubkey = ($lines | Select-Object -Last 1).ToString().Trim().ToLowerInvariant()
        } finally { Pop-Location }
    } finally { [Environment]::SetEnvironmentVariable($envName, $null, "Process") }
    if ($pubkey -notmatch '^[0-9a-f]{64}$') { throw "Unexpected Ed25519 public key for $Role" }
    Ensure-Secret -SecretId $SecretId -Role $Role -Bytes ([Text.Encoding]::UTF8.GetBytes($seedHex)) -Labels @{ network="fresh-testnet"; purpose="validator"; role=$Role }
    return [pscustomobject]@{ role=$Role; secret_id=$SecretId; public_key=$pubkey; algorithm="ed25519" }
}

function New-EvmCandidate {
    param([string]$Role, [string]$SecretId, [string]$Purpose)
    $bytes = New-RandomBytes 32
    $hex = "0x$([Convert]::ToHexString($bytes).ToLowerInvariant())"
    $raw = & $cast wallet address --private-key $hex 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or $raw -notmatch '(0x[a-fA-F0-9]{40})') { throw "EVM address derivation failed for $Role" }
    $address = $matches[1].ToLowerInvariant()
    Ensure-Secret -SecretId $SecretId -Role $Role -Bytes ([Text.Encoding]::UTF8.GetBytes($hex)) -Labels @{ network="fresh-testnet"; purpose=$Purpose; role=$Role }
    return [pscustomobject]@{ role=$Role; secret_id=$SecretId; evm_address=$address; algorithm="secp256k1" }
}

$results = [System.Collections.Generic.List[object]]::new()
for ($i = 1; $i -le $ValidatorCount; $i++) {
    $results.Add((New-Ed25519Candidate -Role "validator-$i-ed25519" -SecretId "creg-fresh-testnet-validator-$i-ed25519"))
    $results.Add((New-EvmCandidate -Role "validator-$i-evm" -SecretId "creg-fresh-testnet-validator-$i-evm" -Purpose "validator-stake"))
}

foreach ($role in @(
    @{ Name="deployer"; Purpose="contract-deployment" },
    @{ Name="governance"; Purpose="governance-signing" },
    @{ Name="faucet"; Purpose="faucet-funding" },
    @{ Name="publisher"; Purpose="publisher-stake" },
    @{ Name="bridge"; Purpose="bridge-relayer" },
    @{ Name="relayer"; Purpose="l1-relayer" }
)) {
    $results.Add((New-EvmCandidate -Role "$($role.Name)-evm" -SecretId "creg-fresh-testnet-$($role.Name)-evm" -Purpose $role.Purpose))
}

$operatorBytes = New-RandomBytes 32
$operatorSecretId = "creg-fresh-testnet-operator-api-key"
Ensure-Secret -SecretId $operatorSecretId -Role "operator-api" -Bytes ([Text.Encoding]::UTF8.GetBytes(([Convert]::ToHexString($operatorBytes).ToLowerInvariant()))) -Labels @{ network="fresh-testnet"; purpose="operator-api"; role="operator-api" }
$results.Add([pscustomobject]@{ role="operator-api"; secret_id=$operatorSecretId; algorithm="random-256-bit" })

$signingKeySecret = "creg-testnet-spec-signing-key"
if (-not (Test-SecretExists -SecretId $signingKeySecret)) { throw "Existing rotated spec-signing secret $signingKeySecret is missing" }

[pscustomobject]@{
    project_id=$ProjectId
    generated_at=(Get-Date).ToUniversalTime().ToString("o")
    custody="CREG operator-controlled Google Secret Manager; independent validator custody not yet established"
    spec_signing_secret=$signingKeySecret
    validator_count=$ValidatorCount
    keys=@($results)
} | ConvertTo-Json -Depth 5
