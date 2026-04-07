<#
.SYNOPSIS
    Chain Registry — Docker Deployment Script v0.3.0
    Deploys the CREG blockchain network to Docker for testnet or mainnet.

.DESCRIPTION
    This script automates the full lifecycle of deploying Chain Registry:
      1. Pre-flight checks (Docker, disk, memory, ports)
      2. Environment setup and validation
      3. Docker image build (with Dockerfile selection)
      4. Infrastructure startup (Anvil, IPFS, PostgreSQL)
      5. Smart contract deployment
      6. Validator node startup (1 or 10 nodes)
      7. Faucet and explorer startup (testnet)
      8. Health verification and smoke tests
      9. Summary dashboard

.PARAMETER Mode
    Deployment mode: "testnet" (10 validators, faucet, explorer) or
    "mainnet" (single-validator production, no faucet).
    Default: testnet

.PARAMETER Nodes
    Number of validator nodes to start (1-10). Default: 10 for testnet, 1 for mainnet.

.PARAMETER Dockerfile
    Which Dockerfile to use: "default", "minimal", "optimized".
    Default: "minimal" (faster builds for testnet).

.PARAMETER SkipBuild
    Skip the Docker image build step (use existing images).

.PARAMETER SkipContracts
    Skip smart contract deployment (use existing addresses in .env.testnet).

.PARAMETER Down
    Tear down the running deployment instead of starting one.

.PARAMETER Logs
    Follow logs after deployment.

.PARAMETER Status
    Show status of a running deployment and exit.

.PARAMETER Reset
    Tear down and delete all volumes (DESTRUCTIVE — removes all chain data).

.EXAMPLE
    # Deploy 10-validator testnet (recommended first run)
    .\deploy.ps1

.EXAMPLE
    # Deploy testnet with fast build
    .\deploy.ps1 -Mode testnet -Dockerfile minimal

.EXAMPLE
    # Deploy with 3 validators only
    .\deploy.ps1 -Mode testnet -Nodes 3

.EXAMPLE
    # Skip build, just restart nodes
    .\deploy.ps1 -SkipBuild

.EXAMPLE
    # Tear down testnet
    .\deploy.ps1 -Down

.EXAMPLE
    # Full reset (deletes chain data)
    .\deploy.ps1 -Reset

.EXAMPLE
    # Check status
    .\deploy.ps1 -Status
#>

param(
    [ValidateSet("testnet", "mainnet")]
    [string]$Mode = "testnet",

    [ValidateRange(1, 10)]
    [int]$Nodes = 0,

    [ValidateSet("default", "minimal", "optimized")]
    [string]$Dockerfile = "minimal",

    [switch]$SkipBuild,
    [switch]$SkipContracts,
    [switch]$Down,
    [switch]$Logs,
    [switch]$Status,
    [switch]$Reset,
    [switch]$Help
)

# ─── Configuration ────────────────────────────────────────────────────────────
$ErrorActionPreference = "Stop"
$ScriptVersion = "v0.3.0"
$ProjectRoot = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path "$ProjectRoot\Cargo.toml")) {
    $ProjectRoot = $PSScriptRoot
}
if (-not (Test-Path "$ProjectRoot\Cargo.toml")) {
    $ProjectRoot = Get-Location
}

Set-Location $ProjectRoot

# Default node count based on mode
if ($Nodes -eq 0) {
    $Nodes = if ($Mode -eq "testnet") { 10 } else { 1 }
}

# Compose files
$TestnetCompose = "docker-compose.testnet.yml"
$DevCompose = "docker-compose.yml"
$EnvFile = ".env.testnet"

$ComposeFile = if ($Mode -eq "testnet") { $TestnetCompose } else { $DevCompose }

# Dockerfile mapping
$DockerfileMap = @{
    "default"   = "Dockerfile"
    "minimal"   = "Dockerfile.minimal"
    "optimized" = "Dockerfile.optimized"
}
$SelectedDockerfile = $DockerfileMap[$Dockerfile]
$env:CREG_DOCKERFILE = $SelectedDockerfile

# ─── Terminal Colors ──────────────────────────────────────────────────────────
function Write-Banner {
    Write-Host ""
    Write-Host "  ╔═══════════════════════════════════════════════════════╗" -ForegroundColor Cyan
    Write-Host "  ║                                                       ║" -ForegroundColor Cyan
    Write-Host "  ║         Chain Registry  —  Docker Deployer            ║" -ForegroundColor Cyan
    Write-Host "  ║         Version $ScriptVersion  |  Mode: $($Mode.ToUpper().PadRight(8))          ║" -ForegroundColor Cyan
    Write-Host "  ║                                                       ║" -ForegroundColor Cyan
    Write-Host "  ╚═══════════════════════════════════════════════════════╝" -ForegroundColor Cyan
    Write-Host ""
}

function Write-Step([string]$Step, [string]$Message) {
    Write-Host "  [$Step] " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-OK([string]$Message) {
    Write-Host "    ✓ " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Warn([string]$Message) {
    Write-Host "    ⚠ " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Fail([string]$Message) {
    Write-Host "    ✗ " -ForegroundColor Red -NoNewline
    Write-Host $Message
}

function Write-Info([string]$Message) {
    Write-Host "    → " -ForegroundColor DarkGray -NoNewline
    Write-Host $Message
}

function Write-Section([string]$Title) {
    Write-Host ""
    Write-Host "  ─── $Title ───" -ForegroundColor Magenta
    Write-Host ""
}

# ─── Help ─────────────────────────────────────────────────────────────────────
if ($Help) {
    Get-Help $MyInvocation.MyCommand.Path -Detailed
    exit 0
}

# ─── Utility Functions ────────────────────────────────────────────────────────

function Invoke-Docker {
    param([string[]]$Arguments)
    $cmd = "docker"
    $output = & $cmd @Arguments 2>&1
    return @{ ExitCode = $LASTEXITCODE; Output = $output }
}

function Invoke-Compose {
    param([string[]]$Arguments)
    $baseArgs = @("compose")
    if ($Mode -eq "testnet" -and (Test-Path $EnvFile)) {
        $baseArgs += @("--env-file", $EnvFile)
    }
    $baseArgs += @("-f", $ComposeFile)
    $allArgs = $baseArgs + $Arguments
    & docker @allArgs
    return $LASTEXITCODE
}

function Test-Port([int]$Port) {
    try {
        $connection = New-Object System.Net.Sockets.TcpClient
        $connection.Connect("127.0.0.1", $Port)
        $connection.Close()
        return $true
    } catch {
        return $false
    }
}

function Wait-ForEndpoint {
    param(
        [string]$Name,
        [string]$Url,
        [int]$TimeoutSecs = 60,
        [string]$Method = "GET",
        [string]$Body = $null
    )
    Write-Info "Waiting for $Name..."
    for ($i = 1; $i -le $TimeoutSecs; $i++) {
        try {
            $params = @{
                Uri         = $Url
                Method      = $Method
                TimeoutSec  = 3
                ErrorAction = "SilentlyContinue"
            }
            if ($Body) {
                $params["Body"] = $Body
                $params["ContentType"] = "application/json"
            }
            $response = Invoke-RestMethod @params
            if ($response -or $LASTEXITCODE -eq 0) {
                Write-OK "$Name is ready (${i}s)"
                return $true
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    Write-Warn "$Name did not become ready within ${TimeoutSecs}s"
    return $false
}

# ─── Status Command ──────────────────────────────────────────────────────────
if ($Status) {
    Write-Banner
    Write-Section "Deployment Status"

    # Check running containers
    $containers = docker compose -f $ComposeFile ps --format json 2>$null | ConvertFrom-Json -ErrorAction SilentlyContinue
    if (-not $containers) {
        Write-Fail "No containers running for $ComposeFile"
        exit 1
    }

    $running = 0
    $total = 0
    foreach ($c in $containers) {
        $total++
        $state = if ($c.State -eq "running") { "Green" } else { "Red" }
        $icon = if ($c.State -eq "running") { "●" } else { "○" }
        Write-Host "    $icon " -ForegroundColor $state -NoNewline
        Write-Host "$($c.Name.PadRight(35)) $($c.State.PadRight(10)) $($c.Status)" 
        if ($c.State -eq "running") { $running++ }
    }

    Write-Host ""
    Write-Host "    $running/$total containers running" -ForegroundColor $(if ($running -eq $total) { "Green" } else { "Yellow" })

    # Check service endpoints
    Write-Host ""
    Write-Step "ENDPOINTS" "Service accessibility"
    $endpoints = @(
        @{ Name = "Node API";     Url = "http://localhost:8080/v1/health" },
        @{ Name = "Faucet";       Url = "http://localhost:8082/health" },
        @{ Name = "Explorer";     Url = "http://localhost:3000" },
        @{ Name = "Anvil RPC";    Url = "http://localhost:8545"; Method = "POST"; Body = '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' },
        @{ Name = "IPFS API";     Url = "http://localhost:5001/api/v0/version"; Method = "POST" }
    )
    foreach ($ep in $endpoints) {
        try {
            $params = @{ Uri = $ep.Url; TimeoutSec = 3; ErrorAction = "Stop"; Method = $(if ($ep.Method) { $ep.Method } else { "GET" }) }
            if ($ep.Body) { $params["Body"] = $ep.Body; $params["ContentType"] = "application/json" }
            $null = Invoke-RestMethod @params
            Write-OK "$($ep.Name.PadRight(15)) → $($ep.Url)"
        } catch {
            Write-Fail "$($ep.Name.PadRight(15)) → $($ep.Url)"
        }
    }

    # Chain stats
    Write-Host ""
    try {
        $stats = Invoke-RestMethod -Uri "http://localhost:8080/v1/chain/stats" -TimeoutSec 3
        Write-Step "CHAIN" "Blockchain statistics"
        Write-OK "Height:   $($stats.tip_height)"
        Write-OK "Packages: $($stats.package_count)"
        Write-OK "Tip Hash: $($stats.tip_hash)"
    } catch {
        Write-Warn "Could not fetch chain stats"
    }

    exit 0
}

# ─── Tear Down ───────────────────────────────────────────────────────────────
if ($Down -or $Reset) {
    Write-Banner
    Write-Section "Tearing Down"

    if ($Reset) {
        Write-Warn "This will DELETE ALL chain data, volumes, and images."
        $confirm = Read-Host "    Type 'yes' to confirm"
        if ($confirm -ne "yes") {
            Write-Info "Cancelled."
            exit 0
        }
    }

    Write-Step "1/2" "Stopping containers..."
    Invoke-Compose @("down") | Out-Null
    Write-OK "Containers stopped"

    if ($Reset) {
        Write-Step "2/2" "Removing volumes and data..."
        Invoke-Compose @("down", "-v", "--remove-orphans") | Out-Null
        # Clean local data directories
        @("data/node-1", "data/node-2", "data/node-3", "data/node1", "data/node2", "data/node3") | ForEach-Object {
            if (Test-Path $_) { Remove-Item -Recurse -Force $_ }
        }
        Write-OK "Volumes and data removed"
    }

    Write-OK "Deployment torn down"
    exit 0
}

# ═══════════════════════════════════════════════════════════════════════════════
# MAIN DEPLOYMENT FLOW
# ═══════════════════════════════════════════════════════════════════════════════

Write-Banner

$startTime = Get-Date
$errors = @()

# ─── Step 1: Pre-flight Checks ──────────────────────────────────────────────
Write-Section "Step 1: Pre-flight Checks"

Write-Step "1.1" "Docker daemon"
$dockerResult = Invoke-Docker @("info", "--format", "{{.ServerVersion}}")
if ($dockerResult.ExitCode -ne 0) {
    Write-Fail "Docker daemon is not running. Start Docker Desktop and try again."
    exit 1
}
Write-OK "Docker $($dockerResult.Output) running"

Write-Step "1.2" "Docker Compose v2"
$composeResult = Invoke-Docker @("compose", "version", "--short")
if ($composeResult.ExitCode -ne 0) {
    Write-Fail "Docker Compose v2 not found. Update Docker Desktop."
    exit 1
}
Write-OK "Docker Compose $($composeResult.Output)"

Write-Step "1.3" "Available disk space"
$drive = (Get-Item $ProjectRoot).PSDrive
$freeGB = [math]::Round($drive.Free / 1GB, 1)
if ($freeGB -lt 10) {
    Write-Fail "Only ${freeGB}GB free on $($drive.Name):. Need at least 10GB."
    exit 1
}
Write-OK "${freeGB}GB free on $($drive.Name): drive"

Write-Step "1.4" "Available memory"
$memGB = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
$minRAM = if ($Nodes -gt 3) { 16 } else { 8 }
if ($memGB -lt $minRAM) {
    Write-Warn "${memGB}GB RAM detected. ${minRAM}GB+ recommended for $Nodes nodes."
    $errors += "Low RAM"
} else {
    Write-OK "${memGB}GB RAM"
}

Write-Step "1.5" "Port availability"
$requiredPorts = @(
    @{ Port = 8080; Name = "Node API" },
    @{ Port = 8545; Name = "Anvil RPC" },
    @{ Port = 5432; Name = "PostgreSQL" },
    @{ Port = 3000; Name = "Explorer" }
)
if ($Mode -eq "testnet") {
    $requiredPorts += @{ Port = 8082; Name = "Faucet" }
}
$portConflicts = @()
foreach ($p in $requiredPorts) {
    if (Test-Port $p.Port) {
        Write-Warn "Port $($p.Port) ($($p.Name)) is already in use"
        $portConflicts += $p.Port
    }
}
if ($portConflicts.Count -eq 0) {
    Write-OK "All required ports available"
} else {
    Write-Warn "$($portConflicts.Count) port conflict(s). Deployment may fail if these aren't from a previous CREG run."
}

Write-Step "1.6" "Required files"
$requiredFiles = @(
    "Cargo.toml", "Cargo.lock", $SelectedDockerfile, $ComposeFile
)
if ($Mode -eq "testnet") { $requiredFiles += $EnvFile }
$missingFiles = @()
foreach ($f in $requiredFiles) {
    if (-not (Test-Path $f)) {
        Write-Fail "Missing: $f"
        $missingFiles += $f
    }
}
if ($missingFiles.Count -gt 0) {
    if ($missingFiles -contains $EnvFile) {
        Write-Fail "Cannot deploy testnet without $EnvFile. Copy .env.example to $EnvFile and fill in values."
    }
    exit 1
}
Write-OK "All required files present"

Write-Step "1.7" "Runtime assets"
$assetDirs = @("models", "rules", "config/sandbox")
foreach ($d in $assetDirs) {
    if (-not (Test-Path $d)) {
        Write-Warn "Missing directory: $d (non-fatal, but ML/sandbox features may be limited)"
    }
}
if ((Test-Path "models") -and (Test-Path "rules") -and (Test-Path "config/sandbox")) {
    Write-OK "Runtime assets (models, rules, config) present"
}

# ─── Step 2: Environment Validation ─────────────────────────────────────────
Write-Section "Step 2: Environment Validation"

if ($Mode -eq "testnet") {
    Write-Step "2.1" "Loading $EnvFile"
    $envContent = Get-Content $EnvFile -ErrorAction Stop
    $envVars = @{}
    foreach ($line in $envContent) {
        if ($line -match '^\s*([A-Z_][A-Z0-9_]*)=(.*)$') {
            $envVars[$Matches[1]] = $Matches[2]
        }
    }
    Write-OK "Loaded $($envVars.Count) environment variables"

    Write-Step "2.2" "Validator keys"
    $keyCount = 0
    for ($i = 1; $i -le $Nodes; $i++) {
        $keyName = "NODE${i}_VALIDATOR_KEY"
        if ($envVars.ContainsKey($keyName) -and $envVars[$keyName].Length -ge 60) {
            $keyCount++
        } else {
            Write-Warn "Missing or short key: $keyName"
        }
    }
    if ($keyCount -ge $Nodes) {
        Write-OK "All $Nodes validator keys present"
    } else {
        Write-Warn "Only $keyCount/$Nodes validator keys found. Missing nodes won't join consensus."
    }

    Write-Step "2.3" "Contract addresses"
    $contractVars = @("TESTNET_TOKEN_ADDR", "TESTNET_STAKING_ADDR", "TESTNET_REGISTRY_ADDR")
    $hasContracts = $true
    foreach ($cv in $contractVars) {
        if (-not $envVars.ContainsKey($cv) -or $envVars[$cv].Length -lt 10) {
            $hasContracts = $false
        }
    }
    if ($hasContracts) {
        Write-OK "Contract addresses configured"
    } else {
        Write-Warn "Some contract addresses missing — will be set after deployment"
    }

    Write-Step "2.4" "Faucet configuration"
    if ($envVars.ContainsKey("FAUCET_PRIVATE_KEY") -and $envVars["FAUCET_PRIVATE_KEY"].Length -gt 10) {
        Write-OK "Faucet key configured"
    } else {
        Write-Warn "FAUCET_PRIVATE_KEY not set — faucet will not work"
    }

} else {
    Write-Step "2.1" "Mainnet mode — using docker-compose.yml defaults"
    Write-OK "Development compose selected"
}

# ─── Step 3: Build Docker Images ────────────────────────────────────────────
Write-Section "Step 3: Docker Image Build"

if ($SkipBuild) {
    Write-Step "3.0" "Skipping build (--SkipBuild)"
    # Verify image exists
    $imageCheck = docker images chain-registry-app --format "{{.Tag}}" 2>$null
    if (-not $imageCheck) {
        Write-Fail "No chain-registry-app image found. Remove -SkipBuild or build manually."
        exit 1
    }
    Write-OK "Using existing image: chain-registry-app:$imageCheck"
} else {
    Write-Step "3.1" "Building application image"
    Write-Info "Dockerfile: $SelectedDockerfile"
    Write-Info "This may take 5-60 minutes depending on the Dockerfile..."

    if ($Mode -eq "testnet") {
        # Testnet: build via compose profile
        $buildExitCode = Invoke-Compose @("--profile", "build", "build", "--build-arg", "BUILDKIT_INLINE_CACHE=1")
        if ($buildExitCode -ne 0) {
            Write-Fail "Docker build failed. Check the output above for errors."
            Write-Info "Common fixes:"
            Write-Info "  - Use -Dockerfile minimal for faster builds"
            Write-Info "  - Ensure Cargo.lock is up to date: cargo generate-lockfile"
            Write-Info "  - Check Docker has enough memory allocated (Settings → Resources)"
            exit 1
        }
        Write-OK "Application image built"
    } else {
        # Dev/Mainnet: build directly
        docker build -f $SelectedDockerfile -t chain-registry-app:latest .
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "Docker build failed."
            exit 1
        }
        Write-OK "Application image built"
    }

    Write-Step "3.2" "Build web explorer image"
    if ($Mode -eq "testnet" -and (Test-Path "explorer/Dockerfile")) {
        # Already built via compose profile above
        Write-OK "Web explorer image built (included in profile build)"
    } elseif (Test-Path "explorer/Dockerfile") {
        docker build -f explorer/Dockerfile -t chain-registry-web-explorer:latest explorer/
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "Explorer build failed (non-fatal)"
        } else {
            Write-OK "Web explorer image built"
        }
    }
}

# ─── Step 4: Start Infrastructure ───────────────────────────────────────────
Write-Section "Step 4: Infrastructure Services"

Write-Step "4.1" "Starting Anvil (Ethereum local chain)"
Invoke-Compose @("up", "-d", "anvil") | Out-Null
$anvilReady = Wait-ForEndpoint -Name "Anvil" `
    -Url "http://localhost:8545" `
    -Method "POST" `
    -Body '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' `
    -TimeoutSecs 30
if (-not $anvilReady) {
    Write-Fail "Anvil failed to start. Check: docker logs creg-testnet-anvil"
    exit 1
}

Write-Step "4.2" "Starting IPFS"
Invoke-Compose @("up", "-d", "ipfs") | Out-Null
Start-Sleep -Seconds 5
# Verify IPFS from inside container (host API may return 403)
$ipfsCheck = docker exec $(if ($Mode -eq "testnet") { "creg-testnet-ipfs" } else { "chain-registry-ipfs" }) ipfs version 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-OK "IPFS running: $($ipfsCheck.Trim())"
} else {
    Write-Warn "IPFS may still be initializing"
}

Write-Step "4.3" "Starting PostgreSQL"
Invoke-Compose @("up", "-d", "postgres") | Out-Null
Start-Sleep -Seconds 3
$pgContainer = if ($Mode -eq "testnet") { "creg-testnet-postgres" } else { "chain-registry-postgres" }
$pgReady = $false
for ($i = 1; $i -le 20; $i++) {
    $pgCheck = docker exec $pgContainer pg_isready -U creg 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-OK "PostgreSQL is ready"
        $pgReady = $true
        break
    }
    Start-Sleep -Seconds 1
}
if (-not $pgReady) {
    Write-Warn "PostgreSQL may still be starting"
}

# ─── Step 5: Deploy Smart Contracts ─────────────────────────────────────────
Write-Section "Step 5: Smart Contract Deployment"

if ($SkipContracts) {
    Write-Step "5.0" "Skipping contract deployment (--SkipContracts)"
    Write-OK "Using existing contract addresses from $EnvFile"
} else {
    Write-Step "5.1" "Deploying contracts via Foundry"

    if ($Mode -eq "testnet") {
        Invoke-Compose @("up", "deploy-contracts") | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Warn "Contract deployment returned non-zero exit code."
            Write-Info "This may be OK if contracts were already deployed."
            Write-Info "Check logs: docker logs creg-testnet-deploy"
        } else {
            Write-OK "Contracts deployed successfully"
        }
    } else {
        Write-Info "Dev mode: contracts deployed as part of compose startup"
        Write-OK "Skipped (dev mode)"
    }

    Write-Step "5.2" "Contract verification"
    try {
        $chainId = Invoke-RestMethod -Uri "http://localhost:8545" -Method POST `
            -ContentType "application/json" `
            -Body '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' `
            -TimeoutSec 5
        Write-OK "Chain ID: $([Convert]::ToInt64($chainId.result, 16))"
    } catch {
        Write-Warn "Could not verify chain ID"
    }
}

# ─── Step 6: Start Validator Nodes ──────────────────────────────────────────
Write-Section "Step 6: Validator Nodes ($Nodes node(s))"

if ($Mode -eq "testnet") {
    # Start nodes in batches to avoid overwhelming the system
    Write-Step "6.1" "Starting node-1 (bootstrap seed)"
    Invoke-Compose @("up", "-d", "node-1") | Out-Null
    $node1Ready = Wait-ForEndpoint -Name "Node 1" -Url "http://localhost:8080/v1/health" -TimeoutSecs 120
    if (-not $node1Ready) {
        Write-Fail "Node 1 failed to start. This is the bootstrap node — other nodes depend on it."
        Write-Info "Check logs: docker logs creg-testnet-node-1"
        Write-Info "Common issues:"
        Write-Info "  - Missing CREG_VALIDATOR_KEY in .env.testnet"
        Write-Info "  - Anvil not reachable"
        Write-Info "  - Build failure (binary not in image)"
        exit 1
    }

    if ($Nodes -gt 1) {
        Write-Step "6.2" "Starting nodes 2-$Nodes"
        $nodeNames = @()
        for ($i = 2; $i -le $Nodes; $i++) {
            $nodeNames += "node-$i"
        }
        Invoke-Compose @("up", "-d") + $nodeNames | Out-Null
        Write-Info "Waiting for nodes to sync..."
        Start-Sleep -Seconds 10

        # Check each node
        $healthyNodes = 1  # node-1 already verified
        for ($i = 2; $i -le $Nodes; $i++) {
            $port = 8080 + ($i - 1) * 100 + 2  # 8182, 8183, ...
            # The testnet compose maps node-2 → 8182, node-3 → 8183, etc.
            $nodePort = switch ($i) {
                2 { 8182 } 3 { 8183 } 4 { 8184 } 5 { 8185 }
                6 { 8186 } 7 { 8187 } 8 { 8188 } 9 { 8189 } 10 { 8190 }
            }
            try {
                $null = Invoke-RestMethod -Uri "http://localhost:${nodePort}/v1/health" -TimeoutSec 5 -ErrorAction Stop
                $healthyNodes++
            } catch {
                Write-Warn "Node $i (port $nodePort) not yet healthy"
            }
        }
        Write-OK "$healthyNodes/$Nodes nodes healthy"

        # Calculate quorum
        $quorum = [math]::Floor(2 * $Nodes / 3) + 1
        if ($healthyNodes -ge $quorum) {
            Write-OK "Quorum reached: $healthyNodes ≥ $quorum (⌊2×$Nodes/3⌋+1)"
        } else {
            Write-Warn "Quorum NOT reached: $healthyNodes < $quorum. Consensus will stall."
            Write-Info "Wait a few minutes and re-check with: .\deploy.ps1 -Status"
        }
    }
} else {
    Write-Step "6.1" "Starting single validator node"
    Invoke-Compose @("up", "-d", "node-1") | Out-Null
    $node1Ready = Wait-ForEndpoint -Name "Node 1" -Url "http://localhost:8080/v1/health" -TimeoutSecs 120
    if (-not $node1Ready) {
        Write-Fail "Node failed to start."
        exit 1
    }
}

# ─── Step 7: Start Ancillary Services ───────────────────────────────────────
Write-Section "Step 7: Ancillary Services"

if ($Mode -eq "testnet") {
    Write-Step "7.1" "Starting Faucet"
    Invoke-Compose @("up", "-d", "faucet") | Out-Null
    $faucetReady = Wait-ForEndpoint -Name "Faucet" -Url "http://localhost:8082/health" -TimeoutSecs 30
    if (-not $faucetReady) {
        Write-Warn "Faucet not ready — token distribution won't work"
        Write-Info "Check logs: docker logs creg-testnet-faucet"
    }

    Write-Step "7.2" "Starting Web Explorer"
    Invoke-Compose @("up", "-d", "web-explorer") | Out-Null
    $explorerReady = Wait-ForEndpoint -Name "Explorer" -Url "http://localhost:3000" -TimeoutSecs 30
    if (-not $explorerReady) {
        Write-Warn "Explorer not ready — web UI won't be accessible"
    }
} else {
    Write-Step "7.1" "Starting Faucet (dev mode)"
    Invoke-Compose @("up", "-d", "faucet") | Out-Null

    Write-Step "7.2" "Starting Explorer"
    Invoke-Compose @("up", "-d", "web-explorer") | Out-Null
}

# ─── Step 8: Health Verification & Smoke Tests ──────────────────────────────
Write-Section "Step 8: Verification"

Write-Step "8.1" "Chain health"
Start-Sleep -Seconds 5
try {
    $stats = Invoke-RestMethod -Uri "http://localhost:8080/v1/chain/stats" -TimeoutSec 10 -ErrorAction Stop
    Write-OK "Chain tip height: $($stats.tip_height)"
    Write-OK "Package count:    $($stats.package_count)"
    Write-OK "Tip hash:         $($stats.tip_hash.Substring(0, 16))..."
} catch {
    Write-Warn "Could not fetch chain stats. Node may still be syncing."
}

Write-Step "8.2" "Validator set"
try {
    $nodes = Invoke-RestMethod -Uri "http://localhost:8080/v1/nodes" -TimeoutSec 10 -ErrorAction Stop
    $validatorCount = if ($nodes.validators) { $nodes.validators.Count } else { 0 }
    Write-OK "Active validators: $validatorCount"
} catch {
    Write-Warn "Could not fetch validator set"
}

Write-Step "8.3" "P2P network"
try {
    $p2p = Invoke-RestMethod -Uri "http://localhost:8080/v1/p2p/status" -TimeoutSec 10 -ErrorAction Stop
    $peerCount = if ($p2p.peers) { $p2p.peers } else { 0 }
    Write-OK "Connected peers: $peerCount"
} catch {
    Write-Warn "Could not fetch P2P status"
}

if ($Mode -eq "testnet") {
    Write-Step "8.4" "Faucet status"
    try {
        $faucetHealth = Invoke-RestMethod -Uri "http://localhost:8082/health" -TimeoutSec 5 -ErrorAction Stop
        Write-OK "Faucet operational"
    } catch {
        Write-Warn "Faucet not responding"
    }
}

# ─── Step 9: Summary ────────────────────────────────────────────────────────
$elapsed = (Get-Date) - $startTime
$elapsedStr = "{0:mm\:ss}" -f $elapsed

Write-Host ""
Write-Host "  ╔═══════════════════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "  ║                                                       ║" -ForegroundColor Green
Write-Host "  ║            Deployment Complete!                       ║" -ForegroundColor Green
Write-Host "  ║                                                       ║" -ForegroundColor Green
Write-Host "  ╚═══════════════════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""

Write-Host "  Mode:       $($Mode.ToUpper())" -ForegroundColor White
Write-Host "  Nodes:      $Nodes validator(s)" -ForegroundColor White
Write-Host "  Dockerfile: $SelectedDockerfile" -ForegroundColor White
Write-Host "  Duration:   $elapsedStr" -ForegroundColor White
Write-Host ""

Write-Host "  ─── Service URLs ───" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Node API:      " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:8080" -ForegroundColor White
Write-Host "  Health Check:  " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:8080/v1/health" -ForegroundColor White
Write-Host "  Chain Stats:   " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:8080/v1/chain/stats" -ForegroundColor White
if ($Mode -eq "testnet") {
    Write-Host "  Faucet:        " -NoNewline -ForegroundColor DarkGray
    Write-Host "http://localhost:8082" -ForegroundColor White
}
Write-Host "  Web Explorer:  " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:3000" -ForegroundColor White
Write-Host "  Anvil RPC:     " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:8545" -ForegroundColor White
Write-Host "  Prometheus:    " -NoNewline -ForegroundColor DarkGray
Write-Host "http://localhost:8080/metrics" -ForegroundColor White
Write-Host ""

Write-Host "  ─── Next Steps ───" -ForegroundColor Cyan
Write-Host ""
Write-Host "  # Check deployment status" -ForegroundColor DarkGray
Write-Host "  .\deploy.ps1 -Status" -ForegroundColor White
Write-Host ""
Write-Host "  # Follow logs" -ForegroundColor DarkGray
Write-Host "  docker compose -f $ComposeFile logs -f" -ForegroundColor White
Write-Host ""
if ($Mode -eq "testnet") {
    Write-Host "  # Get test tokens" -ForegroundColor DarkGray
    Write-Host '  Invoke-RestMethod -Uri "http://localhost:8082/drip" -Method POST -ContentType "application/json" -Body ''{"address":"0x..."}''' -ForegroundColor White
    Write-Host ""
    Write-Host "  # Run stress test" -ForegroundColor DarkGray
    Write-Host "  docker compose --env-file .env.testnet -f $ComposeFile --profile stress-test up stress-test" -ForegroundColor White
    Write-Host ""
}
Write-Host "  # Tear down" -ForegroundColor DarkGray
Write-Host "  .\deploy.ps1 -Down" -ForegroundColor White
Write-Host ""

if ($errors.Count -gt 0) {
    Write-Host "  ⚠ Warnings during deployment: $($errors -join ', ')" -ForegroundColor Yellow
    Write-Host ""
}

# ─── Optional: Follow Logs ──────────────────────────────────────────────────
if ($Logs) {
    Write-Host "  Following logs (Ctrl+C to stop)..." -ForegroundColor DarkGray
    Write-Host ""
    Invoke-Compose @("logs", "-f", "--tail", "50")
}
