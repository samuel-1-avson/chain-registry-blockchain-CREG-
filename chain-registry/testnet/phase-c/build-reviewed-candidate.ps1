param(
    [int]$PreviousBuildProcessId = 0,
    [string]$ImageTag = 'creg-node:phase-c-secure-r9-review',
    [string]$EvidenceDirectory = ''
)

$ErrorActionPreference = 'Stop'
$phaseCRepo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$phaseCEvidence = if ($EvidenceDirectory) { (Resolve-Path -LiteralPath $EvidenceDirectory).Path } else {
    Join-Path (Split-Path -Parent $phaseCRepo) 'docs/testnet-heal-evidence/2026-09-26-phase-c-scanner-readiness-r9'
}
$phaseCManifestPath = Join-Path $phaseCEvidence 'source-manifest.json'
$phaseCStatusPath = Join-Path $phaseCEvidence 'build-status.json'
$phaseCImage = $ImageTag
if ($phaseCImage -notmatch '^creg-node:phase-c-secure-r[0-9]+(?:-[a-z0-9-]+)?$') {
    throw 'Expected a versioned Phase C candidate image tag.'
}
$phaseCBuildLog = 'build-' + $phaseCImage.Split(':')[1] + '.log'
Set-Location -LiteralPath $phaseCRepo

function Write-PhaseCBuildStatus([string]$Status, [string]$Detail) {
    [ordered]@{
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        status = $Status
        image_tag = $phaseCImage
        detail = $Detail
        public_cutover = 'pending'
    } | ConvertTo-Json | Set-Content -LiteralPath $phaseCStatusPath -Encoding UTF8
}

function Assert-PhaseCReviewedSource {
    $phaseCManifest = Get-Content -LiteralPath $phaseCManifestPath -Raw | ConvertFrom-Json
    foreach ($phaseCFile in $phaseCManifest.files) {
        $phaseCPath = Join-Path $phaseCRepo $phaseCFile.path
        $phaseCActual = (Get-FileHash -LiteralPath $phaseCPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($phaseCActual -ne $phaseCFile.sha256) {
            throw "Reviewed source changed: $($phaseCFile.path). Refresh the source review before rebuilding."
        }
    }
    foreach ($phaseCDeletedFile in $phaseCManifest.deleted_files) {
        $phaseCPath = Join-Path $phaseCRepo $phaseCDeletedFile.path
        if (Test-Path -LiteralPath $phaseCPath) {
            throw "Reviewed deleted source unexpectedly exists: $($phaseCDeletedFile.path)."
        }
    }
}

try {
    Assert-PhaseCReviewedSource
    Write-PhaseCBuildStatus 'queued' 'Waiting for the preceding build to release the dependency cache.'
    if ($PreviousBuildProcessId -gt 0) {
        $phaseCPrevious = Get-CimInstance Win32_Process -Filter "ProcessId = $PreviousBuildProcessId"
        if ($phaseCPrevious -and ($phaseCPrevious.Name -ne 'docker.exe' -or
            $phaseCPrevious.CommandLine -notlike '*phase-c-secure-r5*' -or
            $phaseCPrevious.CommandLine -notmatch '\bbuild\b')) {
            throw 'The preceding process is not the expected Phase C r5 Docker build.'
        }
        $phaseCCreated = if ($phaseCPrevious) { $phaseCPrevious.CreationDate } else { $null }
        while ($phaseCPrevious -and $phaseCPrevious.CreationDate -eq $phaseCCreated) {
            Start-Sleep -Seconds 5
            $phaseCPrevious = Get-CimInstance Win32_Process -Filter "ProcessId = $PreviousBuildProcessId"
        }
    }
    Assert-PhaseCReviewedSource
    Write-PhaseCBuildStatus 'compiling' 'Building the reviewed node and CLI; real-host acceptance is outstanding.'
    Write-Output "Building $phaseCImage from the reviewed source manifest."
    # Docker's normal build progress is written to stderr. Preserve it in the
    # evidence log without treating each progress line as a PowerShell failure.
    $ErrorActionPreference = 'Continue'
    docker build -f testnet/phase-c/Dockerfile.full-repair `
        --build-arg CREG_SPEC_SIGNING_PUBKEY=3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01 `
        --build-arg CREG_NODE_FEATURES=phase-c-real-fleet `
        --progress=plain -t $phaseCImage . 2>&1 |
        Tee-Object -FilePath (Join-Path $phaseCEvidence $phaseCBuildLog)
    $phaseCExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($phaseCExit -ne 0) { throw "Reviewed Docker build exited with code $phaseCExit." }
    Assert-PhaseCReviewedSource
    $phaseCImageId = docker image inspect --format '{{.Id}}' $phaseCImage
    if ($LASTEXITCODE -ne 0) { throw 'Reviewed image could not be inspected after build.' }
    [ordered]@{
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        image_tag = $phaseCImage
        image_id = "$phaseCImageId".Trim()
        source_manifest = 'source-manifest.json'
        real_host_acceptance = 'pending'
        public_cutover = 'pending'
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $phaseCEvidence 'image-build.json') -Encoding UTF8
    Write-PhaseCBuildStatus 'built' 'Compilation succeeded; signed-spec compatibility and real-host acceptance remain pending.'
    Write-Output 'Reviewed image built. Public cutover remains pending.'
} catch {
    Write-PhaseCBuildStatus 'failed' $_.Exception.Message
    Write-Error $_
    exit 1
}
