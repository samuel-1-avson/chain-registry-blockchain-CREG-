# ==============================================================================
# Chain Registry (CREG) -- Independent External Validator Launch Script (Windows)
# ==============================================================================
# Launch an external validator for the coordinated R7 testnet. Voting requires
# on-chain admission through the validator onboarding checklist.
# ==============================================================================

param(
    [string]$NodeId = "",
    [string]$EthRpc = "",
    [int]$HttpPort = 8080,
    [int]$P2pPort = 9000,
    [string]$CustomKey = ""
)

$ErrorActionPreference = "Stop"
$chainId = "creg-phase-c-testnet-2026-09-26-r7"
$specUrl = "https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json"
if (-not $EthRpc) {
    $EthRpc = if ($env:CREG_ETH_RPC) { $env:CREG_ETH_RPC } else { "https://ethereum-sepolia-rpc.publicnode.com" }
}

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "   Chain Registry (CREG) -- External Validator Onboarding    " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# Check Docker
if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Error "Docker is not found. Please install and start Docker Desktop."
    exit 1
}

# Resolve Node ID and isolated Data Directory (allows running multiple nodes on one host)
if (-not $NodeId) {
    $NodeId = "ext-val-primary"
    $dataDir = Join-Path $env:USERPROFILE ".creg-validator"
} else {
    $dataDir = Join-Path $env:USERPROFILE ".creg-validator-$NodeId"
}

# Keep R7 state separate from the retired network's database. Pre-create the
# bind mount so Docker does not create it with ownership the node cannot use.
$containerName = "creg-validator-$NodeId"
$httpBind = "$HttpPort"
if ($env:CREG_ONBOARDING_KEY_FILE) {
    $existingContainers = docker container ls -a --format '{{.Names}}'
    if ($LASTEXITCODE -ne 0) { throw "Could not inspect Docker containers." }
    $expectedLabel = "$chainId/$env:CREG_ONBOARDING_PUBLIC_KEY"
    if ($existingContainers -ccontains $containerName) {
        $matchingContainers = docker container ls -a --filter "label=dev.creg.validator=$expectedLabel" --format '{{.Names}}'
        if ($LASTEXITCODE -ne 0 -or $matchingContainers -cnotcontains $containerName) {
            throw "Container $containerName belongs to another configuration. Review it before starting this validator."
        }
        docker start $containerName
        exit $LASTEXITCODE
    }
    $httpBind = if ($HttpPort -eq 0) { "127.0.0.1:" } else { "127.0.0.1:$HttpPort" }

    # Applicants can sync before a certified committee transition. Keep the
    # enrollment key on the host and run the reviewed R7 image without a signer.
    $profile = Join-Path $PSScriptRoot "phase-c/r7/observer.docker-env"
    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
    $scannerDir = Join-Path $repoRoot "docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-ansi-regex-bundle"
    $scannerFiles = @("rules/supply_chain.yar", "known_malicious_hashes.json", "osv_snapshot.json")
    if (-not (Test-Path -LiteralPath $profile -PathType Leaf) -or
        @($scannerFiles | Where-Object { -not (Test-Path -LiteralPath (Join-Path $scannerDir $_) -PathType Leaf) }).Count -ne 0) {
        throw "The reviewed R7 observer profile or scanner bundle is missing from the CREG checkout."
    }
    $reviewedImageId = "sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3"
    $reviewedConfigId = "sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9"
    $image = $null
    foreach ($candidate in @($reviewedImageId, $reviewedConfigId)) {
        docker image inspect $candidate *> $null
        if ($LASTEXITCODE -eq 0) { $image = $candidate; break }
    }
    if (-not $image) {
        $image = "us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@$reviewedImageId"
        docker pull $image
        if ($LASTEXITCODE -ne 0) { throw "The reviewed R7 image is unavailable. Use the maintainer-provided image." }
    }
    $observerData = Join-Path $dataDir "onboarding-observer-r7"
    New-Item -ItemType Directory -Path $observerData -Force | Out-Null
    $autoActivate = ($env:CREG_ADMISSION_AUTO_ACTIVATE -eq "true" -or $env:CREG_ADMISSION_AUTO_ACTIVATE -eq "1")
    if ($env:CREG_OBSERVER_IMAGE) {
        $image = $env:CREG_OBSERVER_IMAGE
    }
    $observerArgs = @("run", "-d", "--name", $containerName, "--restart", "unless-stopped",
        "--label", "dev.creg.validator=$expectedLabel", "--env-file", $profile,
        "--env", "CREG_NODE_ID=$NodeId", "--env", "CREG_IS_VALIDATOR=false",
        "--publish", "${httpBind}:8080",
        "--mount", "type=bind,source=$observerData,target=/data",
        "--mount", "type=bind,source=$scannerDir,target=/scanner,readonly")
    if ($autoActivate) {
        $resolvedKey = (Resolve-Path -LiteralPath $env:CREG_ONBOARDING_KEY_FILE).Path
        $observerArgs += @(
            "--env", "CREG_ADMISSION_AUTO_ACTIVATE=true",
            "--env", "CREG_VALIDATOR_KEY_FILE=/keys/validator.key",
            "--mount", "type=bind,source=$resolvedKey,target=/keys/validator.key,readonly"
        )
    }
    $observerArgs += $image
    & docker @observerArgs | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Failed to start the R7 observer container." }
    Write-Host "R7 observer started. The onboarding page will show its sync status."
    exit 0
}

$chainDataDir = Join-Path $dataDir "chain-data-r7"
New-Item -ItemType Directory -Path $chainDataDir -Force | Out-Null
$keyFile = Join-Path $dataDir "validator.key"
$envFile = Join-Path $dataDir "validator.env"

# Generate, load, or use custom Validator Key
if ($CustomKey) {
    $validatorKey = $CustomKey.Trim().ToLower()
    [System.IO.File]::WriteAllText($keyFile, $validatorKey)
    Write-Host "[OK] Using custom validator key (saved to $keyFile)" -ForegroundColor Green
} elseif (-not (Test-Path $keyFile)) {
    Write-Host "Generating new Ed25519 validator consensus key..." -ForegroundColor Yellow
    $bytes = New-Object byte[] 32
    (New-Object System.Security.Cryptography.RNGCryptoServiceProvider).GetBytes($bytes)
    $validatorKey = [System.BitConverter]::ToString($bytes).Replace("-", "").ToLower()
    [System.IO.File]::WriteAllText($keyFile, $validatorKey)
    Write-Host "[OK] Generated new validator key: $keyFile" -ForegroundColor Green
} else {
    $validatorKey = (Get-Content $keyFile -Raw).Trim()
    Write-Host "[OK] Loaded existing validator key from $keyFile" -ForegroundColor Green
}

$operatorKeyFile = Join-Path $dataDir "operator_api.key"
if (-not (Test-Path $operatorKeyFile)) {
    $opBytes = New-Object byte[] 32
    (New-Object System.Security.Cryptography.RNGCryptoServiceProvider).GetBytes($opBytes)
    $opKey = [System.BitConverter]::ToString($opBytes).Replace("-", "").ToLower()
    [System.IO.File]::WriteAllText($operatorKeyFile, $opKey)
} else {
    $opKey = (Get-Content $operatorKeyFile -Raw).Trim()
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$scannerDir = Join-Path $repoRoot "docs/testnet-heal-evidence/2026-09-27-production-pipeline-cutover/scanner-r14-ansi-regex-bundle"
$scannerFiles = @("rules/supply_chain.yar", "known_malicious_hashes.json", "osv_snapshot.json")
if (@($scannerFiles | Where-Object { -not (Test-Path -LiteralPath (Join-Path $scannerDir $_) -PathType Leaf) }).Count -ne 0) {
    throw "The reviewed Phase C R7 scanner bundle is missing at $scannerDir."
}

$explicitValidator = ($env:CREG_IS_VALIDATOR -eq "true" -or $env:CREG_IS_VALIDATOR -eq "1")
$isValidatorEnv = if ($explicitValidator) { "true" } else { "false" }
$nodeRoleEnv = if ($explicitValidator) { "validator" } else { "observer" }
$defaultSeeds = "/ip4/34.156.122.19/tcp/9000/p2p/12D3KooW9ygmUCoS7BPxkJRxz8trK7nxDwidTJU7DMe8VBqXs8W8,/ip4/34.124.165.97/tcp/9000/p2p/12D3KooWFEhFCeFgfxmJzatpgeiCGHfocDLGS336vH1ouESfM3qb"
$p2pSeedsEnv = if ($env:BOOTNODE) { $env:BOOTNODE } else { $defaultSeeds }

# creg-node verifies the spec against its pinned key and starts as an
# auto-activating observer until certified validator membership takes effect.
$envLines = @(
    "CREG_TESTNET=true",
    "CREG_NODE_ID=$NodeId",
    "CREG_DATA_DIR=/data",
    "CREG_IS_VALIDATOR=$isValidatorEnv",
    "CREG_NODE_ROLE=$nodeRoleEnv",
    "CREG_ADMISSION_AUTO_ACTIVATE=true",
    "CREG_VALIDATOR_KEY=$validatorKey",
    "CREG_OPERATOR_API_KEY=$opKey",
    "CREG_ETH_RPC=$EthRpc",
    "CREG_EXPECTED_L1_CHAIN_ID=11155111",
    "CREG_CHAIN_ID=$chainId",
    "CREG_CHAIN_SPEC_URL=$specUrl",
    "CREG_SPEC_SIGNATURE_URL=$specUrl.sig",
    "CREG_IPFS_URL=https://ipfs.testnet.cregnet.dev",
    "CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/9000",
    "CREG_P2P_SEEDS=$p2pSeedsEnv",
    "CREG_PEERS=https://api.testnet.cregnet.dev",
    "CREG_LISTEN=0.0.0.0:8080",
    "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false",
    "CREG_DEV_SANDBOX=false",
    "CREG_YARA_RULES_DIR=/scanner/rules",
    "CREG_THREAT_DB_PATH=/scanner/known_malicious_hashes.json",
    "CREG_OSV_SNAPSHOT_PATH=/scanner/osv_snapshot.json",
    "CREG_OSV_CONSENSUS=true",
    "CREG_OSV_BLOCK_CRITICAL=true",
    "CREG_OSV_LIVE_FALLBACK=false",
    "CREG_OSV_SNAPSHOT_EPOCH=osv-2026-09-28-phase-c-ansi-regex-r14-live",
    "CREG_SCANNER_PROFILE_DIGEST=7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd",
    "RUST_LOG=info,chain_registry_node=debug,consensus=info"
)
$envContent = $envLines -join [Environment]::NewLine
[System.IO.File]::WriteAllText($envFile, $envContent)

# Auto-detect available Docker image
$imageCandidates = @()
if ($env:CREG_OBSERVER_IMAGE) { $imageCandidates += $env:CREG_OBSERVER_IMAGE }
$imageCandidates += @(
    "us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:r7-auto-admission-20261127-v4",
    "us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:r7-auto-admission-20261126-v3",
    "chain-registry-node-secure:latest",
    "creg-node:fleet",
    "creg-node:latest"
)
$chosenImage = $null
foreach ($cand in $imageCandidates) {
    $imgId = docker images -q $cand 2>$null
    if ($imgId) {
        $chosenImage = $cand
        break
    }
}

if (-not $chosenImage) {
    $chainRegistryDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    $dockerfile = Join-Path $chainRegistryDir "Dockerfile"
    Write-Host "Local validator Docker image not found. Building from $dockerfile (multi-stage, no host toolchain needed; takes a while on first run)..." -ForegroundColor Yellow
    docker build -f "$dockerfile" -t chain-registry-node-secure:latest "$chainRegistryDir"
    if ($LASTEXITCODE -ne 0) { throw "Failed to build the validator Docker image." }
    $chosenImage = "chain-registry-node-secure:latest"
}

Write-Host ""
Write-Host "Starting independent validator container using $chosenImage..." -ForegroundColor Yellow

docker rm -f $containerName 2>$null | Out-Null

$dockerArgs = @(
    "run",
    "-d",
    "--name", $containerName,
    "--restart", "unless-stopped",
    "--user", "10002:10002",
    "--privileged",
    "--env-file", "$envFile",
    "-p", "${httpBind}:8080",
    "-p", "$($P2pPort):9000",
    "-v", "${chainDataDir}:/data",
    "-v", "${scannerDir}:/scanner:ro",
    $chosenImage
)
& docker @dockerArgs | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Failed to start the R7 validator container." }

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host "   R7 Validator Container Started                          " -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host "Node ID:         $NodeId" -ForegroundColor Cyan
Write-Host "Chain Data:      $chainDataDir" -ForegroundColor Cyan
Write-Host "Key Location:    $keyFile" -ForegroundColor Cyan
Write-Host "Docker Image:    $chosenImage" -ForegroundColor Cyan
Write-Host "P2P Port:        $P2pPort (Firewall: OPEN TCP/UDP $P2pPort)" -ForegroundColor Cyan
if ($HttpPort -eq 0) {
    Write-Host "HTTP RPC:        Docker-assigned loopback port" -ForegroundColor Cyan
} else {
    Write-Host "HTTP RPC Port:   $HttpPort (Keep private or proxy with TLS)" -ForegroundColor Cyan
}
$seedDescription = if ($env:BOOTNODE) { $env:BOOTNODE } else { "from the signed R7 spec" }
Write-Host "P2P Seeds:       $seedDescription" -ForegroundColor Cyan
Write-Host ""
Write-Host "--- KEY BACKUP NOTICE ---" -ForegroundColor Yellow
Write-Host "Validator key is stored at $keyFile; back it up securely without copying it into logs or chat." -ForegroundColor Yellow
Write-Host ""
Write-Host "--- REMOTE VERIFICATION ---" -ForegroundColor Yellow
if ($HttpPort -ne 0) {
    Write-Host "Local query:     curl.exe http://localhost:$($HttpPort)/v1/health" -ForegroundColor Gray
    Write-Host "Remote query:    curl.exe http://<YOUR_SERVER_IP>:$($HttpPort)/v1/health" -ForegroundColor Gray
}
Write-Host "Global Explorer: https://explorer.testnet.cregnet.dev (Validators Tab)" -ForegroundColor Gray
Write-Host ""
Write-Host "--- R7 VALIDATOR ADMISSION ---" -ForegroundColor Yellow
Write-Host "Follow docs/VALIDATOR_ONBOARDING_CHECKLIST.md for staking, registration, and approval."
Write-Host "Monitor live logs: docker logs -f creg-validator-$NodeId" -ForegroundColor Cyan
Write-Host ""
