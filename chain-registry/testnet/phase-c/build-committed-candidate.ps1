[CmdletBinding()]
param(
    [ValidatePattern('^[0-9a-f]{40}$')]
    [string]$Revision = 'a19fce7dcbd917fa1650212485807ab1b7bae1c1',
    [ValidatePattern('^[a-z0-9][a-z0-9._/-]*:[a-zA-Z0-9_][a-zA-Z0-9_.-]{0,127}$')]
    [string]$ImageTag = 'creg-node:phase-c-a19fce7-rust193',
    [ValidateSet('node-runtime', 'node-cli', 'recovery-inspector', 'faucet-runtime', 'creg-indexer', 'relayer-runtime')]
    [string]$Target = 'node-runtime',
    [string]$StagingDirectory = '',
    [string]$EvidenceDirectory = '',
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$phaseCRepoRoot = (& git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve the Git repository.' }
$phaseCResolvedRevision = (& git -C $phaseCRepoRoot rev-parse --verify ($Revision + '^{commit}')).Trim()
if ($LASTEXITCODE -ne 0 -or $phaseCResolvedRevision -ne $Revision) {
    throw 'Revision must name the full existing commit hash.'
}
$phaseCTree = (& git -C $phaseCRepoRoot rev-parse ($Revision + '^{tree}')).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve the committed source tree.' }
$phaseCSourceEpoch = (& git -C $phaseCRepoRoot show -s --format=%ct $Revision).Trim()
if ($LASTEXITCODE -ne 0 -or $phaseCSourceEpoch -notmatch '^[0-9]+$') {
    throw 'Cannot resolve the commit timestamp.'
}

if (-not $StagingDirectory) {
    $StagingDirectory = Join-Path ([IO.Path]::GetTempPath()) (
        'creg-phase-c-' + $Revision.Substring(0, 12) + '-' + [Guid]::NewGuid().ToString('N'))
}
$phaseCStage = [IO.Path]::GetFullPath($StagingDirectory)
if (Test-Path -LiteralPath $phaseCStage) {
    throw 'StagingDirectory must be a new directory, so existing files cannot enter the source context.'
}
# Never create a staging tree inside the checkout: it should remain independent
# of dirty files, OneDrive sync and the repository's Docker ignore rules.
$phaseCRepoPrefix = [IO.Path]::GetFullPath($phaseCRepoRoot).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
if ($phaseCStage.StartsWith($phaseCRepoPrefix, [StringComparison]::OrdinalIgnoreCase) -or
    $phaseCStage -eq $phaseCRepoPrefix.TrimEnd('\', '/')) {
    throw 'StagingDirectory must be outside the Git checkout.'
}
New-Item -ItemType Directory -Path $phaseCStage | Out-Null
$phaseCExportRoot = Join-Path $phaseCStage 'context'
New-Item -ItemType Directory -Path $phaseCExportRoot | Out-Null
$phaseCArchive = Join-Path $phaseCStage 'committed-source.tar'

# Only tracked compiler inputs are archived. The two sample specs support
# include_str! references; fleet credentials/state and scripts are excluded.
$phaseCArchivePaths = @(
    'chain-registry/Cargo.toml',
    'chain-registry/Cargo.lock',
    'chain-registry/Dockerfile',
    'chain-registry/crates',
    'chain-registry/migrations',
    'chain-registry/config',
    'chain-registry/rules',
    'chain-registry/validators',
    'chain-registry/docker-compose.yml',
    'chain-registry/docker-compose.light.yml',
    'chain-registry/testnet/phase-c/r7/chain-spec.json',
    'chain-registry/testnet/phase-c/r7/chain-spec.json.sig',
    'chain-registry/testnet/chain-spec.example.json',
    'chain-registry/testnet/chain-spec.sepolia.json'
)
& git -C $phaseCRepoRoot archive --format=tar ('--output=' + $phaseCArchive) $Revision -- @phaseCArchivePaths
if ($LASTEXITCODE -ne 0) { throw 'git archive failed.' }
& tar -xf $phaseCArchive -C $phaseCExportRoot
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract the committed source archive.' }
$phaseCContext = Join-Path $phaseCExportRoot 'chain-registry'
$phaseCUtf8 = New-Object Text.UTF8Encoding($false)
function Write-PhaseCUtf8([string]$Path, [string]$Content) {
    [IO.File]::WriteAllText($Path, $Content, $phaseCUtf8)
}
function Get-PhaseCSha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$phaseCFiles = @(
    Get-ChildItem -LiteralPath $phaseCContext -Recurse -File | Sort-Object FullName | ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($phaseCContext.Length + 1).Replace('\', '/')
            sha256 = Get-PhaseCSha256 $_.FullName
        }
    }
)
$phaseCRecipeName = 'Dockerfile'
$phaseCRecipe = Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) $phaseCRecipeName
Copy-Item -LiteralPath $phaseCRecipe -Destination (Join-Path $phaseCContext $phaseCRecipeName)
$phaseCCloudBuildName = switch ($Target) {
    'node-runtime'   { 'cloudbuild-node-candidate.yaml' }
    'faucet-runtime' { 'cloudbuild-faucet-candidate.yaml' }
    'creg-indexer'   { 'cloudbuild-indexer-candidate.yaml' }
    'relayer-runtime' { 'cloudbuild-relayer-candidate.yaml' }
    default          { 'cloudbuild-committed-candidate.yaml' }
}
$phaseCCloudBuild = Join-Path $PSScriptRoot $phaseCCloudBuildName
Copy-Item -LiteralPath $phaseCCloudBuild -Destination (Join-Path $phaseCContext $phaseCCloudBuildName)
$phaseCManifest = [ordered]@{
    schema = 'creg-phase-c-committed-image-source-v1'
    source_revision = $Revision
    source_tree = $phaseCTree
    source_date_epoch = [long]$phaseCSourceEpoch
    source_selection = 'git archive; compiler inputs only; working tree excluded'
    archive_sha256 = Get-PhaseCSha256 $phaseCArchive
    cargo_lock_sha256 = Get-PhaseCSha256 (Join-Path $phaseCContext 'Cargo.lock')
    rust_toolchain = '1.98.1'
    rust_toolchain_image = 'rust:1.98.1-slim-bookworm'
    native_builder_image = 'ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60'
    runtime_image = 'ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60'
    platform = 'linux/amd64'
    node_features = @('phase-c-real-fleet')
    cli_features = @('common/phase-c-canonical-transactions')
    spec_signing_public_key = '3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01'
    build_recipe = $phaseCRecipeName
    build_recipe_sha256 = Get-PhaseCSha256 $phaseCRecipe
    cloud_build_config = $phaseCCloudBuildName
    cloud_build_config_sha256 = Get-PhaseCSha256 $phaseCCloudBuild
    build_helper_sha256 = Get-PhaseCSha256 $PSCommandPath
    file_count = $phaseCFiles.Count
    files = $phaseCFiles
}
$phaseCManifestPath = Join-Path $phaseCContext 'BUILD-SOURCE-MANIFEST.json'
Write-PhaseCUtf8 $phaseCManifestPath (($phaseCManifest | ConvertTo-Json -Depth 8) + "`n")
$phaseCManifestSha = Get-PhaseCSha256 $phaseCManifestPath
$phaseCChecksums = ($phaseCFiles | ForEach-Object { $_.sha256 + '  ' + $_.path }) -join "`n"
Write-PhaseCUtf8 (Join-Path $phaseCContext 'BUILD-SOURCE-FILES.sha256') ($phaseCChecksums + "`n")
# This fresh context has no generated outputs or fleet credentials. Keep every
# archived source file, including modules whose names end in _tests.rs.
Write-PhaseCUtf8 (Join-Path $phaseCContext '.dockerignore') "target/`n.git/`n"

if (-not $EvidenceDirectory) { $EvidenceDirectory = Join-Path $phaseCStage 'evidence' }
$phaseCEvidence = [IO.Path]::GetFullPath($EvidenceDirectory)
if (-not (Test-Path -LiteralPath $phaseCEvidence)) {
    New-Item -ItemType Directory -Path $phaseCEvidence | Out-Null
}
$phaseCBuildEvidence = Join-Path $phaseCEvidence ($Target + '-build.json')
if (Test-Path -LiteralPath $phaseCBuildEvidence) {
    throw 'EvidenceDirectory already contains this target build record; use a fresh evidence directory.'
}
$phaseCEvidenceManifest = Join-Path $phaseCEvidence 'source-manifest.json'
if ((Test-Path -LiteralPath $phaseCEvidenceManifest) -and
    (Get-PhaseCSha256 $phaseCEvidenceManifest) -ne $phaseCManifestSha) {
    throw 'EvidenceDirectory already describes different source inputs; use a fresh evidence directory.'
}
Copy-Item -LiteralPath $phaseCManifestPath -Destination (Join-Path $phaseCEvidence 'source-manifest.json')
Copy-Item -LiteralPath (Join-Path $phaseCContext 'BUILD-SOURCE-FILES.sha256') -Destination (Join-Path $phaseCEvidence 'source-files.sha256')
$phaseCDockerArgs = @(
    'build', '--platform', 'linux/amd64',
    '--file', (Join-Path $phaseCContext $phaseCRecipeName),
    '--target', $Target,
    '--build-arg', ('SOURCE_REVISION=' + $Revision),
    '--build-arg', ('SOURCE_MANIFEST_SHA256=' + $phaseCManifestSha),
    '--build-arg', ('SOURCE_DATE_EPOCH=' + $phaseCSourceEpoch),
    '--build-arg', ('CARGO_TARGET_CACHE_ID=creg-phase-c-rust198-' + $phaseCManifestSha),
    '--progress=plain', '--tag', $ImageTag, $phaseCContext
)
$phaseCRecord = [ordered]@{
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    status = 'prepared'
    source_revision = $Revision
    source_manifest_sha256 = $phaseCManifestSha
    source_file_count = $phaseCFiles.Count
    source_context = $phaseCContext
    source_archive = $phaseCArchive
    target = $Target
    image_tag = $ImageTag
    docker_arguments = $phaseCDockerArgs
    deployment_performed = $false
}
function Write-PhaseCRecord {
    Write-PhaseCUtf8 $phaseCBuildEvidence (($phaseCRecord | ConvertTo-Json -Depth 6) + "`n")
}
Write-PhaseCRecord
Write-Output ('Prepared committed source: ' + $Revision)
Write-Output ('Source context: ' + $phaseCContext)
Write-Output ('Source manifest SHA256: ' + $phaseCManifestSha)
Write-Output ('Build record: ' + $phaseCBuildEvidence)
if (-not $Build) {
    Write-Output 'Preparation complete. Add -Build to compile and load the local candidate image.'
    return
}

try {
    $phaseCRecord.status = 'building'
    Write-PhaseCRecord
    # Docker progress uses stderr, so preserve it without turning normal
    # progress lines into terminating PowerShell errors.
    $ErrorActionPreference = 'Continue'
    & docker @phaseCDockerArgs 2>&1 | Tee-Object -FilePath (Join-Path $phaseCEvidence ($Target + '-build.log'))
    $phaseCBuildExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($phaseCBuildExit -ne 0) { throw ('Docker build exited with code ' + $phaseCBuildExit) }
    $phaseCImageJson = & docker image inspect $ImageTag
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the completed local image.' }
    $phaseCImage = @($phaseCImageJson | ConvertFrom-Json)[0]
    if ($phaseCImage.Config.Labels.'org.opencontainers.image.revision' -ne $Revision -or
        $phaseCImage.Config.Labels.'dev.creg.source-manifest.sha256' -ne $phaseCManifestSha) {
        throw 'Completed image does not carry the expected committed source labels.'
    }
    $phaseCRecord.status = 'built'
    $phaseCRecord.image_id = $phaseCImage.Id
    $phaseCRecord.completed_utc = [DateTime]::UtcNow.ToString('o')
    Write-PhaseCRecord
    Write-Output ('Built local image: ' + $ImageTag + ' ' + $phaseCImage.Id)
} catch {
    $ErrorActionPreference = 'Stop'
    $phaseCRecord.status = 'failed'
    $phaseCRecord.detail = $_.Exception.Message
    Write-PhaseCRecord
    throw
}
