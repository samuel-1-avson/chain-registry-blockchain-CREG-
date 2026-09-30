param(
    [string]$EvidenceDirectory = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h19'),
    [string]$PrivateDirectory = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\w3-h19-ansi-regex-r14-live'
)

$ErrorActionPreference = 'Stop'
$canonical = 'npm:ansi-regex@6.1.0'
$archiveHash = '046f6fe17602b686aa522de193d6046a75e557933365ba51e3e3d8e1fa694f3c'
$cliPath = 'E:\creg-phase-c-build-20260927\portable-cli-r25-target\release\creg'
$cliHash = '95569797c4c0c3d767641cc7df07307874da4673fc32daa7f25ceb63776f2c69'
$runtimeImage = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$publisherKey = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\publisher-ed25519.key'
$secretName = 'creg-fresh-testnet-publisher-evm'
$specDirectory = (Resolve-Path (Join-Path $PSScriptRoot 'r7')).Path
$sourcePath = Join-Path $EvidenceDirectory 'package.tgz'
$signedPath = Join-Path $PrivateDirectory 'signed-publish-ansi-regex-r14.json'
$evidenceSignedPath = Join-Path $EvidenceDirectory 'signed-publish-ansi-regex-r14.json'
$reportPath = Join-Path $EvidenceDirectory 'offline-sign-r14.json'
$manifestPath = Join-Path $PrivateDirectory 'creg-manifest.json'
$evidenceManifestPath = Join-Path $EvidenceDirectory 'creg-manifest-r14.json'
$logPath = Join-Path $EvidenceDirectory 'cli-offline-sign-r14.log'
$privateTarballPath = Join-Path $PrivateDirectory 'ansi-regex-6.1.0.tgz'

foreach ($path in @($signedPath, $evidenceSignedPath, $reportPath, $manifestPath, $evidenceManifestPath, $logPath, $privateTarballPath)) {
    if (Test-Path -LiteralPath $path) { throw "Existing acceptance artifact found; refusing to overwrite: $path" }
}
if (Test-Path -LiteralPath $PrivateDirectory) { throw "Private staging directory already exists; refusing to reuse: $PrivateDirectory" }
if ((Get-FileHash -LiteralPath $cliPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cliHash) { throw 'Completed Linux CLI binary hash differs from the reviewed R25 build.' }
if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $archiveHash) { throw 'Reviewed real npm archive hash changed.' }

$review = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'review.json') -Raw | ConvertFrom-Json
$absence = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'package-absence-pre-r14-clean.json') -Raw | ConvertFrom-Json
$fleet = Get-Content -LiteralPath (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-live-rollout/r14-fleet-acceptance.json') -Raw | ConvertFrom-Json
if ($review.canonical -ne $canonical -or $review.archive_sha256 -ne $archiveHash -or $review.package_executed_or_published -ne $false) { throw 'Package review does not match the exact real npm archive.' }
if (-not $absence.passed -or -not $absence.all_eight_absent -or $absence.expected_height -ne 18) { throw 'Fresh package absence evidence is missing or stale.' }
if (-not $fleet.all_eight_acceptance_ready -or $fleet.height -ne 18 -or $fleet.block_hash -ne $absence.expected_hash) { throw 'All-eight R14 readiness does not match the package absence baseline.' }
if (-not (Test-Path -LiteralPath $publisherKey) -or -not (Test-Path -LiteralPath (Join-Path $specDirectory 'chain-spec.json.sig'))) { throw 'Protected publisher key or signed R7 chain spec is missing.' }

$spec = Get-Content -LiteralPath (Join-Path $specDirectory 'chain-spec.json') -Raw | ConvertFrom-Json
if ($spec.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or [string]::IsNullOrWhiteSpace([string]$spec.contracts.staking)) { throw 'Signed R7 publication context is unavailable.' }
$manifest = '{"allowed_network_hosts":[],"allowed_fs_writes":[],"spawns_processes":false,"description":"Pure ANSI escape-sequence matcher; no runtime dependencies or install scripts."}'
New-Item -ItemType Directory -Path $PrivateDirectory -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath $sourcePath -Destination $privateTarballPath
[IO.File]::WriteAllText($manifestPath, $manifest + "`n", [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($evidenceManifestPath, $manifest + "`n", [Text.UTF8Encoding]::new($false))

$environmentNames = @('CREG_PUBLISHER_EVM_KEY','CREG_CHAIN_ID','CREG_GENESIS_HASH','CREG_STAKING_ADDR','CREG_CHAIN_SPEC_PATH','CREG_CHAIN_SPEC_SIG_PATH','CREG_PRODUCTION','XDG_CACHE_HOME')
$savedEnvironment = @{}
foreach ($name in $environmentNames) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $evmKey = (& gcloud secrets versions access latest "--secret=$secretName" '--project=gen-lang-client-0022105784' 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $evmKey -notmatch '^0x[0-9a-fA-F]{64}$') { throw 'Could not securely load the configured publisher EVM signer.' }
    $env:CREG_PUBLISHER_EVM_KEY = $evmKey
    $evmKey = $null
    $env:CREG_CHAIN_ID = [string]$spec.chain_id
    $env:CREG_GENESIS_HASH = [string]$spec.genesis_hash
    $env:CREG_STAKING_ADDR = [string]$spec.contracts.staking
    $env:CREG_CHAIN_SPEC_PATH = '/spec/chain-spec.json'
    $env:CREG_CHAIN_SPEC_SIG_PATH = '/spec/chain-spec.json.sig'
    $env:CREG_PRODUCTION = 'true'
    $env:XDG_CACHE_HOME = '/tmp/creg-cache'
    $dockerArgs = @(
        'run','--rm','--entrypoint','/bin/sh',
        '--env','CREG_PUBLISHER_EVM_KEY',
        '--env',"CREG_CHAIN_ID=$($spec.chain_id)",
        '--env',"CREG_GENESIS_HASH=$($spec.genesis_hash)",
        '--env',"CREG_STAKING_ADDR=$($spec.contracts.staking)",
        '--env','CREG_CHAIN_SPEC_PATH','--env','CREG_CHAIN_SPEC_SIG_PATH','--env','CREG_PRODUCTION','--env','XDG_CACHE_HOME',
        '--mount',"type=bind,source=$cliPath,target=/tmp/creg,readonly",
        '--mount',"type=bind,source=$PrivateDirectory,target=/submission",
        '--mount',"type=bind,source=$publisherKey,target=/publisher.key,readonly",
        '--mount',"type=bind,source=$specDirectory,target=/spec,readonly",
        $runtimeImage,'-c',
        '/tmp/creg publish /submission/ansi-regex-6.1.0.tgz --manifest /submission/creg-manifest.json --key-file /publisher.key --publisher-address 0xa40D79527314efD4964D49c4a0673C87b612348B --ipfs-url https://ipfs.testnet.cregnet.dev --offline /submission/signed-publish-ansi-regex-r14.json'
    )
    $output = & docker @dockerArgs 2>&1 | Tee-Object -FilePath $logPath
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $signedPath)) { throw 'Real Linux CLI did not produce the offline signed request; inspect the saved log.' }
} finally {
    $evmKey = $null
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process') }
}

$request = Get-Content -LiteralPath $signedPath -Raw | ConvertFrom-Json
$signedGenesis = ([string]$spec.genesis_hash -replace '^0x','').ToLowerInvariant()
$requestGenesis = ([string]$request.publisher_genesis_hash -replace '^0x','').ToLowerInvariant()
$checks = [ordered]@{
    exact_package = $request.id.ecosystem -eq 'npm' -and $request.id.name -eq 'ansi-regex' -and $request.id.version -eq '6.1.0'
    actual_archive_hash = $request.content_hash -eq $archiveHash
    signed_r7_domain = $request.publisher_chain_id -eq $spec.chain_id -and $requestGenesis -eq $signedGenesis
    staking_contract = ([string]$request.publisher_staking_contract).ToLowerInvariant() -eq ([string]$spec.contracts.staking).ToLowerInvariant()
    authorization_v4 = $request.publisher_authorization_version -eq 4
    publisher_identity_present = -not [string]::IsNullOrWhiteSpace([string]$request.publisher_pubkey) -and -not [string]::IsNullOrWhiteSpace([string]$request.publisher_evm_signature)
    ipfs_cid_present = -not [string]::IsNullOrWhiteSpace([string]$request.ipfs_cid)
}
if (@($checks.Values | Where-Object { $_ -ne $true }).Count -gt 0) { throw "Offline R14 authorization checks failed: $($checks | ConvertTo-Json -Compress)" }
Copy-Item -LiteralPath $signedPath -Destination $evidenceSignedPath
$report = [ordered]@{
    schema='creg-r14-real-linux-cli-offline-publish-v1'
    captured_at_utc=[DateTime]::UtcNow.ToString('o')
    canonical=$canonical
    cli_path=$cliPath
    cli_sha256=$cliHash
    runtime_image_id=$runtimeImage
    archive_sha256=$archiveHash
    scanner_profile_digest=$fleet.scanner_profile_digest
    ipfs_cid=$request.ipfs_cid
    publisher_address=$request.publisher_address
    publisher_pubkey=$request.publisher_pubkey
    publisher_authorization_version=$request.publisher_authorization_version
    signed_request_sha256=(Get-FileHash -LiteralPath $signedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    checks=$checks
    real_ipfs_pin_created=$true
    offline_signature_created=$true
    chain_submission_performed=$false
    public_cutover=$false
}
$report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
Write-Output "Pinned and signed $canonical with the completed Linux CLI. The signature is bound to the R7 spec; no chain submission was performed. Evidence: $reportPath"
