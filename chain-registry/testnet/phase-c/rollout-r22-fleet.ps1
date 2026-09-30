param(
    [string]$EvidenceDirectory = (Join-Path $PSScriptRoot '../../../docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/r22-rollout/fleet'),
    [ValidateSet('o1', 'o2', 'o3', 'o4', 'v2', 'v3', 'v4')]
    [string[]]$Slots = @('o1', 'o2', 'o3', 'o4', 'v2', 'v3', 'v4')
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 or newer is required.' }

$root = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')
$packagePath = Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/image-package.json'
$publicationPath = Join-Path $root 'docs/testnet-heal-evidence/2026-09-27-phase-c-implementation-start/registry-publish.json'
$readinessScript = Join-Path $PSScriptRoot 'capture-r7-fleet-readiness.ps1'
$nodeScript = Join-Path $PSScriptRoot 'rollout-r22-linux-node.sh'
$package = Get-Content -LiteralPath $packagePath -Raw | ConvertFrom-Json
$publication = Get-Content -LiteralPath $publicationPath -Raw | ConvertFrom-Json
$expectedImageId = 'sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'
$acceptedRuntimeImageIds = @($expectedImageId, 'sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9')
$expectedProfile = 'd09a9d2450e6e691bc71bee591b1101aa14d8b0feb6f6088ab6819f48e1a6be4'
$expectedChain = 'creg-phase-c-testnet-2026-09-26-r7'
$expectedGenesis = '0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4'
$expectedTip = 'a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d'
$image = [string]$package.image_tag
$registryImageRef = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3'

if ($package.image_id -ne $expectedImageId -or $image -ne 'creg-node:phase-c-secure-r22-commit-capture') { throw 'R22 image package identity differs from the reviewed pin.' }
if ($publication.image_id -ne $expectedImageId -or $publication.digest_ref -ne $registryImageRef -or $publication.public_cutover) { throw 'R22 registry publication evidence differs from the reviewed private digest.' }
$scriptHash = (Get-FileHash -LiteralPath $nodeScript -Algorithm SHA256).Hash.ToLowerInvariant()
$localImageId = (& docker image inspect $image --format '{{.Id}}' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $localImageId -ne $expectedImageId) { throw 'Reviewed R22 image is not present locally at the pinned image ID.' }

$fleet = @(
    @{ slot = 'v1'; role = 'validator'; name = 'operator laptop'; project = ''; zone = ''; port = 18080 },
    @{ slot = 'v2'; role = 'validator'; name = 'creg-validator-1'; project = 'gen-lang-client-0022105784'; zone = 'us-east1-b'; port = 18080 },
    @{ slot = 'v3'; role = 'validator'; name = 'creg-validator-3'; project = 'fiffi-500411'; zone = 'europe-west1-b'; port = 8080 },
    @{ slot = 'v4'; role = 'validator'; name = 'creg-validator-4'; project = 'fiffi-500411'; zone = 'asia-southeast1-b'; port = 8080 },
    @{ slot = 'o1'; role = 'observer'; name = 'creg-testnet-vm'; project = 'gen-lang-client-0022105784'; zone = 'us-central1-b'; port = 18080 },
    @{ slot = 'o2'; role = 'observer'; name = 'creg-observer-vm'; project = 'gen-lang-client-0022105784'; zone = 'us-central1-a'; port = 8080 },
    @{ slot = 'o3'; role = 'observer'; name = 'creg-observer-3'; project = 'fiffi-500411'; zone = 'us-west1-b'; port = 8080 },
    @{ slot = 'o4'; role = 'observer'; name = 'creg-observer-4'; project = 'fiffi-500411'; zone = 'africa-south1-a'; port = 8080 }
)
$orderedSlots = @('o1', 'o2', 'o3', 'o4', 'v2', 'v3', 'v4')
if (@($Slots | Select-Object -Unique).Count -ne $Slots.Count) { throw 'A slot may appear only once.' }
if (@($Slots | Where-Object { $_ -notin $orderedSlots }).Count -gt 0) { throw 'R22 rollout accepts only the seven remote Phase C nodes.' }
$Slots = @($orderedSlots | Where-Object { $_ -in $Slots })
if ($Slots.Count -eq 0) { throw 'Choose at least one remote node.' }

New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
$progressPath = Join-Path $EvidenceDirectory "fleet-rollout-$runId.json"
$progress = [ordered]@{
    schema = 'creg-real-r22-fleet-rollout-v1'
    run_id = $runId
    started_at_utc = [DateTime]::UtcNow.ToString('o')
    image_id = $expectedImageId
    image_tag = $image
    registry_image_ref = $registryImageRef
    scanner_profile_digest = $expectedProfile
    public_cutover = $false
    requested_slots = $Slots
    nodes = @()
    passed = $false
}

function Get-FreshFleetGate {
    param([string]$Label)
    $path = Join-Path $EvidenceDirectory "preflight-$Label-$runId.json"
    $captureOutput = & $readinessScript -EvidencePath $path -ScannerProfileDigest $expectedProfile 2>&1 | Out-String
    if (-not (Test-Path -LiteralPath $path)) { throw "Fleet readiness capture did not produce evidence for $Label. $captureOutput" }
    $record = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if ($record.nodes.Count -ne 8 -or -not $record.all_eight_ready -or -not $record.all_eight_recovery_clear -or -not $record.all_eight_pbft_clear) {
        throw "Eight-node readiness/recovery/PBFT gate failed for $Label. Evidence: $path"
    }
    foreach ($node in $record.nodes) {
        if ($node.stats.chain_id -ne $expectedChain -or $node.stats.spec_signature_verified -ne $true -or
            (($node.stats.genesis_hash -replace '^0x', '') -ne $expectedGenesis) -or
            $node.stats.tip_height -ne 12 -or $node.stats.finalized_height -ne 12 -or $node.stats.tip_hash -ne $expectedTip) {
            throw "Accepted R7 identity or h12 changed during $Label. Evidence: $path"
        }
    }
    $record
}

function Get-RemoteNodeEvidence {
    param($HostInfo, [string[]]$GcpArgs, [string]$RemotePath, [string]$LocalPath)
    $json = (& gcloud compute ssh $HostInfo.name @GcpArgs "--command=sudo cat '$RemotePath'" 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $json) { throw "Could not read root-owned node evidence from $($HostInfo.slot) over SSH." }
    $record = $json | ConvertFrom-Json
    $record | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $LocalPath -Encoding utf8
    $record
}

function Install-RegistryImageOnRemote {
    param($HostInfo, [string[]]$GcpArgs, [string]$Slot)
    $remoteAuthDirectory = "/tmp/creg-ar-auth-$runId-$Slot"
    $remoteTokenPath = "$remoteAuthDirectory/access-token"
    $localSecretDirectory = Join-Path ([IO.Path]::GetTempPath()) "creg-ar-token-$runId-$Slot-$([guid]::NewGuid().ToString('N'))"
    $localSecretPath = Join-Path $localSecretDirectory 'access-token'
    $token = $null
    $remoteDirectoryCreated = $false
    New-Item -ItemType Directory -Path $localSecretDirectory | Out-Null
    try {
        $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        & icacls.exe $localSecretDirectory /inheritance:r /grant:r "*$($currentSid):(OI)(CI)F" '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Could not restrict the temporary registry-token directory ACL.' }
        $token = (& gcloud auth print-access-token 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $token) { throw 'Could not obtain a short-lived Artifact Registry access token.' }
        [IO.File]::WriteAllText($localSecretPath, $token + "`n", [Text.Encoding]::ASCII)
        $token = $null

        $prepare = "mkdir -m 700 '$remoteAuthDirectory'"
        & gcloud compute ssh $HostInfo.name @GcpArgs "--command=$prepare" 2>$null | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Could not create a private temporary auth directory on $Slot." }
        $remoteDirectoryCreated = $true
        & gcloud compute scp $localSecretPath "$($HostInfo.name):$remoteTokenPath" @GcpArgs | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Could not stage the short-lived registry token on $Slot." }

        $pullCommand = @'
set -euo pipefail
auth_config=$(sudo mktemp -d /run/creg-registry-auth.XXXXXX)
remote_token=REMOTE_TOKEN_PATH
remote_dir=REMOTE_AUTH_DIRECTORY
cleanup() {
  sudo rm -rf -- "$auth_config" || true
  rm -f -- "$remote_token" || true
  rmdir -- "$remote_dir" 2>/dev/null || true
}
trap cleanup EXIT
if ! login_output=$(sudo docker --config "$auth_config" login --username oauth2accesstoken --password-stdin https://us-central1-docker.pkg.dev < "$remote_token" 2>&1); then
  printf '%s\n' "$login_output" >&2
  exit 1
fi
sudo docker --config "$auth_config" pull 'REGISTRY_IMAGE_REF' >/dev/null
sudo docker --config "$auth_config" tag 'REGISTRY_IMAGE_REF' 'IMAGE_TAG'
actual=$(sudo docker image inspect 'IMAGE_TAG' --format '{{.Id}}')
case "$actual" in sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3|sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9) ;; *) echo 'Unreviewed local Docker image identity' >&2; exit 1 ;; esac
repo_digests=$(sudo docker image inspect 'IMAGE_TAG' --format '{{join .RepoDigests "\n"}}')
printf '%s\n' "$repo_digests" | grep -Fxq 'REGISTRY_IMAGE_REF'
sudo rm -rf -- "$auth_config"
rm -f -- "$remote_token"
rmdir -- "$remote_dir"
trap - EXIT
test ! -e "$auth_config" && test ! -e "$remote_token" && test ! -e "$remote_dir"
printf '{"runtime_image_id":"%s","registry_digest":"%s"}\n' "$actual" 'REGISTRY_IMAGE_REF'
'@
        $pullCommand = $pullCommand.Replace('REMOTE_TOKEN_PATH', "'$remoteTokenPath'").Replace('REMOTE_AUTH_DIRECTORY', "'$remoteAuthDirectory'").Replace('REGISTRY_IMAGE_REF', $registryImageRef).Replace('IMAGE_TAG', $image)
        $pullOutput = (& gcloud compute ssh $HostInfo.name @GcpArgs "--command=$pullCommand" 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Pinned private-registry pull failed on $Slot. $pullOutput" }
        try { $pin = $pullOutput | ConvertFrom-Json } catch { throw "Pinned registry pull did not return a valid image record on $Slot." }
        if ($pin.registry_digest -ne $registryImageRef -or $pin.runtime_image_id -notin $acceptedRuntimeImageIds) { throw "Pinned private-registry image identity failed on $Slot." }
        $pin
    } finally {
        $token = $null
        if ($remoteDirectoryCreated) {
            $remoteCleanup = "rm -f -- '$remoteTokenPath'; rmdir -- '$remoteAuthDirectory' 2>/dev/null || true"
            try { & gcloud compute ssh $HostInfo.name @GcpArgs "--command=$remoteCleanup" 2>$null | Out-Null } catch { }
        }
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        $tempTarget = [IO.Path]::GetFullPath($localSecretDirectory)
        if (-not $tempTarget.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
            -not [IO.Path]::GetFileName($tempTarget).StartsWith("creg-ar-token-$runId-$Slot-", [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Refusing to remove a temporary token path outside the task-scoped temp directory.'
        }
        if (Test-Path -LiteralPath $localSecretPath) { Remove-Item -LiteralPath $localSecretPath -Force }
        if (Test-Path -LiteralPath $localSecretDirectory) { Remove-Item -LiteralPath $localSecretDirectory -Force }
    }
}

function Record-NodeAndRequirePostflight {
    param($HostInfo, $NodeEvidence, [string]$PreflightName, [string]$LocalNodeEvidence, [bool]$AlreadyRunningR22, [string]$DistributionMode)
    if (-not $NodeEvidence.passed -or $NodeEvidence.image_id -notin $acceptedRuntimeImageIds -or $NodeEvidence.slot -ne $HostInfo.slot -or
        ($NodeEvidence.registry_digest -and $NodeEvidence.registry_digest -ne $registryImageRef)) {
        throw "R22 node-side evidence failed or mismatched on $($HostInfo.slot)."
    }
    $progress.nodes += [ordered]@{
        slot = $HostInfo.slot; role = $HostInfo.role; host = $HostInfo.name; project = $HostInfo.project; zone = $HostInfo.zone
        prior_image_id = $NodeEvidence.prior_image_id; image_id = $NodeEvidence.image_id
        startup_marker_count = $NodeEvidence.startup_marker_count; readiness_passed = $NodeEvidence.passed
        already_running_r22 = $AlreadyRunningR22
        distribution_mode = $DistributionMode
        node_evidence = [IO.Path]::GetFileName($LocalNodeEvidence)
        preflight_evidence = [IO.Path]::GetFileName($PreflightName)
    }
    $progress | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $progressPath -Encoding utf8
    $afterName = Join-Path $EvidenceDirectory "preflight-after-$($HostInfo.slot)-$runId.json"
    $null = Get-FreshFleetGate "after-$($HostInfo.slot)"
    $progress.nodes[-1].postflight_evidence = [IO.Path]::GetFileName($afterName)
    $progress.nodes[-1].all_eight_postflight_ready = $true
    $progress | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $progressPath -Encoding utf8
    Write-Output "R22 readiness passed on $($HostInfo.slot); all eight nodes passed the immediate postflight at h12."
}

try {
    foreach ($slot in $Slots) {
        $hostInfo = @($fleet | Where-Object slot -eq $slot)[0]
        $before = Get-FreshFleetGate "before-$slot"
        $targetState = @($before.nodes | Where-Object slot -eq $slot)[0]
        if (-not $targetState.ready -or -not $targetState.recovery_clear -or -not $targetState.pbft_clear) { throw "$slot did not pass its immediate pre-rollout readiness gate." }
        if ($hostInfo.role -eq 'validator') {
            $onlineValidators = @($before.nodes | Where-Object { $_.role -eq 'validator' -and $_.ready -and $_.recovery_clear -and $_.pbft_clear })
            if ($onlineValidators.Count -ne 4) { throw 'All four validators must be ready immediately before a one-validator rollout.' }
        }
        $deadLetters = [int]$targetState.recovery_queue.durable_finalized_work_dead_letter_depth
        if ($null -eq $targetState.recovery_queue.durable_finalized_work_dead_letter_depth) { throw "Could not read $slot terminal dead-letter depth." }

        $gcp = @("--project=$($hostInfo.project)", "--zone=$($hostInfo.zone)", '--tunnel-through-iap', '--quiet')
        $remoteScript = '/tmp/rollout-r22-linux-node.sh'
        $remoteEvidence = "/var/lib/creg/phase-c-r7/evidence/r22-$slot-rollout.json"
        $localNodeEvidence = Join-Path $EvidenceDirectory "$slot-rollout-$runId.json"
        $preflightName = Join-Path $EvidenceDirectory "preflight-before-$slot-$runId.json"
        $inspectCommand = "sudo docker inspect 'creg-phase-c-r7-$slot' --format '{{.Image}}'"
        $currentImage = (& gcloud compute ssh $hostInfo.name @gcp "--command=$inspectCommand" 2>$null | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $currentImage) { throw "Could not inspect the current container image for $slot." }
        if ($currentImage -in $acceptedRuntimeImageIds) {
            $tagPinCommand = "sudo docker image inspect '$image' --format '{{.Id}}|{{json .RepoDigests}}'"
            $tagPinText = (& gcloud compute ssh $hostInfo.name @gcp "--command=$tagPinCommand" 2>$null | Out-String).Trim()
            if ($LASTEXITCODE -ne 0) { throw "Could not verify the existing R22 registry digest on $slot." }
            $tagPinParts = $tagPinText -split '\|', 2
        $existingRepoDigests = if ($tagPinParts.Count -eq 2) { @($tagPinParts[1] | ConvertFrom-Json) } else { @() }
        if ($tagPinParts.Count -ne 2 -or $tagPinParts[0] -ne $currentImage -or $existingRepoDigests -notcontains $registryImageRef) {
                throw "$slot has an R22-like local image without the reviewed private registry digest; no container was changed."
            }
            Write-Output "$slot is already on the reviewed R22 registry digest; validating its retained rollout record and running a new all-eight postflight."
            $existingEvidence = Get-RemoteNodeEvidence -HostInfo $hostInfo -GcpArgs $gcp -RemotePath $remoteEvidence -LocalPath $localNodeEvidence
            Record-NodeAndRequirePostflight -HostInfo $hostInfo -NodeEvidence $existingEvidence -PreflightName $preflightName -LocalNodeEvidence $localNodeEvidence -AlreadyRunningR22 $true -DistributionMode 'already-deployed'
            continue
        }
        Write-Output "Staging the reviewed private Artifact Registry digest on $slot ($($hostInfo.name)); current h12 readiness passed."
        & gcloud compute scp $nodeScript "$($hostInfo.name):$remoteScript" @gcp
        if ($LASTEXITCODE -ne 0) { throw "R22 helper transfer failed for $slot." }
        $registryPin = Install-RegistryImageOnRemote -HostInfo $hostInfo -GcpArgs $gcp -Slot $slot
        $runtimeImageId = [string]$registryPin.runtime_image_id
        if ($registryPin.registry_digest -ne $registryImageRef -or $runtimeImageId -notin $acceptedRuntimeImageIds) { throw "Registry runtime identity mismatch on $slot." }

        $apiPort = [int]$hostInfo.port
        $p2pPort = if ($apiPort -eq 18080) { 9001 } else { 9000 }
        $nodeId = "$($hostInfo.role)-$($slot.Substring(1))"
        $command = "printf '%s  %s\n' '$scriptHash' '$remoteScript' | sha256sum -c - >/dev/null && sed -i 's/\r`$//' '$remoteScript' && sudo bash '$remoteScript' '$slot' '$($hostInfo.role)' '$apiPort' '$p2pPort' '$nodeId' '$deadLetters' '$runtimeImageId' '$registryImageRef'"
        $rolloutOutput = (& gcloud compute ssh $hostInfo.name @gcp "--command=$command" 2>(Join-Path $EvidenceDirectory "$slot-rollout-$runId-stderr.txt") | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) {
            $rolloutOutput | Set-Content -LiteralPath (Join-Path $EvidenceDirectory "$slot-failure-$runId.txt") -Encoding utf8
            try { $null = Get-RemoteNodeEvidence -HostInfo $hostInfo -GcpArgs $gcp -RemotePath $remoteEvidence -LocalPath $localNodeEvidence } catch { }
            throw "Guarded R22 rollout failed on $slot. Any started R22 instance remains quarantined with its signer state preserved; inspect the captured host and failure evidence before recovery."
        }
        $rolloutRecord = $rolloutOutput | ConvertFrom-Json
        if (-not $rolloutRecord.passed -or $rolloutRecord.image_id -ne $runtimeImageId -or $rolloutRecord.registry_digest -ne $registryImageRef -or $rolloutRecord.slot -ne $slot) { throw "R22 rollout command reported a failed or mismatched result on $slot." }
        $nodeEvidence = Get-RemoteNodeEvidence -HostInfo $hostInfo -GcpArgs $gcp -RemotePath $remoteEvidence -LocalPath $localNodeEvidence
        Record-NodeAndRequirePostflight -HostInfo $hostInfo -NodeEvidence $nodeEvidence -PreflightName $preflightName -LocalNodeEvidence $localNodeEvidence -AlreadyRunningR22 $false -DistributionMode 'artifact-registry-digest'
    }
    $progress.passed = $true
    $progress.completed_at_utc = [DateTime]::UtcNow.ToString('o')
    $progress | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $progressPath -Encoding utf8
    Write-Output "Remote R22 rollout sequence passed for $($Slots -join ', '). Evidence: $progressPath"
} catch {
    $progress.failure = $_.Exception.Message
    $progress.completed_at_utc = [DateTime]::UtcNow.ToString('o')
    $progress | ConvertTo-Json -Depth 32 | Set-Content -LiteralPath $progressPath -Encoding utf8
    throw
}
