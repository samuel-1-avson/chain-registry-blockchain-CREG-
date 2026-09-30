param(
    [string]$EvidenceDirectory = (Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/next-real-package-w3-h17'),
    [string]$PrivateDirectory = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\w3-h17-has-flag'
)
$ErrorActionPreference = 'Stop'
$canonical = 'npm:has-flag@5.0.1'
$archiveHash = 'b1b9152724e4946f3ee7202ef5c316825ffc498384cb5870dc021d141c7d916b'
$cliPath = 'E:\creg-phase-c-build-20260927\portable-cli-r25-target\release\creg'
$cliHash = '95569797c4c0c3d767641cc7df07307874da4673fc32daa7f25ceb63776f2c69'
$runtimeImage = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$publisherKey = 'C:\Users\samue\CREG-backups\phase-c-candidate-r7\publisher\publisher-ed25519.key'
$secretName = 'creg-fresh-testnet-publisher-evm'
$specDirectory = (Resolve-Path (Join-Path $PSScriptRoot 'r7')).Path
$absencePath = Join-Path $EvidenceDirectory 'package-absence-before-publish-r13.json'
$signedPath = Join-Path $PrivateDirectory 'signed-publish-r13.json'
$evidenceSignedPath = Join-Path $EvidenceDirectory 'signed-publish-r13.json'
$reportPath = Join-Path $EvidenceDirectory 'offline-sign-r13.json'
$evidenceManifest = Join-Path $EvidenceDirectory 'creg-manifest-r13.json'
$signLogPath = Join-Path $EvidenceDirectory 'cli-offline-sign-r13.log'
$privateTarballPath = Join-Path $PrivateDirectory 'has-flag-5.0.1.tgz'
$privateManifestPath = Join-Path $PrivateDirectory 'creg-manifest.json'

foreach ($path in @($signedPath,$evidenceSignedPath,$reportPath,$evidenceManifest,$signLogPath,$privateTarballPath,$privateManifestPath)) {
    if (Test-Path -LiteralPath $path) { throw "Existing acceptance artifact found; refusing to overwrite: $path" }
}
if (Test-Path -LiteralPath $PrivateDirectory) { throw "Private staging directory already exists; refusing to reuse it: $PrivateDirectory" }
if ((Get-FileHash -LiteralPath $cliPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cliHash) { throw 'The completed Linux CLI binary hash differs from the reviewed R25 build.' }
if ((Get-FileHash -LiteralPath (Join-Path $EvidenceDirectory 'package.tgz') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $archiveHash) { throw 'The saved real npm archive hash changed.' }
$packageReview = Get-Content -LiteralPath (Join-Path $EvidenceDirectory 'review.json') -Raw | ConvertFrom-Json
if ($packageReview.canonical -ne $canonical -or $packageReview.status -ne 'prepared_for_review_not_published' -or $packageReview.package_executed_or_published -ne $false) { throw 'The real package review does not match the intended fresh package.' }
$absence = Get-Content -LiteralPath $absencePath -Raw | ConvertFrom-Json
if (-not $absence.passed -or -not $absence.all_eight_absent -or -not $absence.all_eight_same_h16 -or -not $absence.all_eight_r13_ready) { throw 'All-eight fresh-package absence gate did not pass.' }
$bundle = Join-Path (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path 'docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r13-live-bundle'
$bundleManifest = Get-Content -LiteralPath (Join-Path $bundle 'manifest.json') -Raw | ConvertFrom-Json
$snapshot = Get-Content -LiteralPath (Join-Path $bundle 'osv_snapshot.json') -Raw | ConvertFrom-Json
if ($bundleManifest.scanner_profile_digest -ne '8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e' -or $snapshot.entries.PSObject.Properties.Name -notcontains $canonical) { throw 'R13 scanner bundle does not pin the exact package version.' }
if (-not (Test-Path -LiteralPath $publisherKey) -or -not (Test-Path -LiteralPath (Join-Path $specDirectory 'chain-spec.json.sig'))) { throw 'Protected publisher key or signed R7 chain spec is unavailable.' }
$spec = Get-Content -LiteralPath (Join-Path $specDirectory 'chain-spec.json') -Raw | ConvertFrom-Json
if ($spec.chain_id -ne 'creg-phase-c-testnet-2026-09-26-r7' -or $spec.contracts.staking -eq $null) { throw 'The signed R7 publication context is unavailable.' }

$manifest = '{"allowed_network_hosts":[],"allowed_fs_writes":[],"spawns_processes":false,"description":"Pure argument flag helper; no network, filesystem writes, or child processes."}'
New-Item -ItemType Directory -Path $PrivateDirectory -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath (Join-Path $EvidenceDirectory 'package.tgz') -Destination $privateTarballPath
[IO.File]::WriteAllText($privateManifestPath,$manifest + "`n",[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($evidenceManifest,$manifest + "`n",[Text.UTF8Encoding]::new($false))

$environmentNames = @('CREG_PUBLISHER_EVM_KEY','CREG_CHAIN_ID','CREG_GENESIS_HASH','CREG_STAKING_ADDR','CREG_CHAIN_SPEC_PATH','CREG_CHAIN_SPEC_SIG_PATH','CREG_PRODUCTION','XDG_CACHE_HOME')
$savedEnvironment = @{}
foreach ($name in $environmentNames) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name,'Process') }
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
        '/tmp/creg publish /submission/has-flag-5.0.1.tgz --manifest /submission/creg-manifest.json --key-file /publisher.key --publisher-address 0xa40D79527314efD4964D49c4a0673C87b612348B --ipfs-url https://ipfs.testnet.cregnet.dev --offline /submission/signed-publish-r13.json'
    )
    $output = & docker @dockerArgs 2>&1 | Tee-Object -FilePath $signLogPath
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $signedPath)) { throw 'The real Linux CLI did not produce the offline signed request.' }
} finally {
    $evmKey = $null
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name,$savedEnvironment[$name],'Process') }
}

$request = Get-Content -LiteralPath $signedPath -Raw | ConvertFrom-Json
$signedGenesis = ([string]$spec.genesis_hash -replace '^0x','').ToLowerInvariant()
$requestGenesis = ([string]$request.publisher_genesis_hash -replace '^0x','').ToLowerInvariant()
$checks = [ordered]@{
    exact_package = $request.id.ecosystem -eq 'npm' -and $request.id.name -eq 'has-flag' -and $request.id.version -eq '5.0.1'
    actual_archive_hash = $request.content_hash -eq $archiveHash
    signed_r7_domain = $request.publisher_chain_id -eq $spec.chain_id -and $requestGenesis -eq $signedGenesis
    staking_contract = ([string]$request.publisher_staking_contract).ToLowerInvariant() -eq ([string]$spec.contracts.staking).ToLowerInvariant()
    authorization_v4 = $request.publisher_authorization_version -eq 4
    publisher_identity_present = -not [string]::IsNullOrWhiteSpace([string]$request.publisher_pubkey) -and -not [string]::IsNullOrWhiteSpace([string]$request.publisher_evm_signature)
}
if (@($checks.Values | Where-Object { $_ -ne $true }).Count -gt 0) { throw "Offline R13 authorization checks failed: $($checks | ConvertTo-Json -Compress)" }
Copy-Item -LiteralPath $signedPath -Destination $evidenceSignedPath
$report = [ordered]@{
    schema='creg-r13-real-linux-cli-offline-publish-v1'
    captured_at_utc=[DateTime]::UtcNow.ToString('o')
    canonical=$canonical
    cli_path=$cliPath
    cli_sha256=$cliHash
    runtime_image_id=$runtimeImage
    archive_sha256=$archiveHash
    scanner_profile_digest=$bundleManifest.scanner_profile_digest
    publisher_address=$request.publisher_address
    publisher_pubkey=$request.publisher_pubkey
    publisher_authorization_version=$request.publisher_authorization_version
    signed_request_sha256=(Get-FileHash -LiteralPath $signedPath -Algorithm SHA256).Hash.ToLowerInvariant()
    checks=$checks
    offline_signature_created=$true
    chain_submission_performed=$false
    public_cutover=$false
}
$report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $reportPath -Encoding utf8
Write-Output "Signed $canonical with the completed Linux CLI; real IPFS bytes are pinned and the all-eight absence gate remains current. Chain submission has not started. Evidence: $reportPath"
