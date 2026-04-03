# Docker Build Test Script for Windows
# Tests all Docker configurations to verify they work

param(
    [switch]$Quick,  # Skip build tests
    [switch]$Help    # Show help
)

# Show help
if ($Help) {
    Write-Host @"
Docker Build Test Script for Chain Registry

Usage:
    .\test-docker.ps1        # Full test with build
    .\test-docker.ps1 -Quick # Skip build tests
    .\test-docker.ps1 -Help  # Show this help

Tests performed:
    - Docker installation check
    - Docker Compose installation
    - Docker daemon running
    - Required files exist
    - Configuration validity
    - Build context size
    - Common issues (line endings, ports)
    - Minimal Dockerfile build (unless -Quick)

Exit codes:
    0 = All tests passed
    1 = Some tests failed
"@
    exit 0
}

# Colors
$Red = "`e[31m"
$Green = "`e[32m"
$Yellow = "`e[33m"
$Blue = "`e[34m"
$NC = "`e[0m"

# Test counters
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsWarned = 0

# Logging functions
function Write-Info($Message) {
    Write-Host "$Blue[INFO]$NC $Message"
}

function Write-Success($Message) {
    Write-Host "$Green[PASS]$NC $Message"
    $script:TestsPassed++
}

function Write-Error($Message) {
    Write-Host "$Red[FAIL]$NC $Message"
    $script:TestsFailed++
}

function Write-Warn($Message) {
    Write-Host "$Yellow[WARN]$NC $Message"
    $script:TestsWarned++
}

# Test 1: Check Docker is installed
function Test-DockerInstalled {
    Write-Info "Testing Docker installation..."
    try {
        $dockerVersion = docker --version 2>$null
        if ($LASTEXITCODE -eq 0 -and $dockerVersion) {
            Write-Success "Docker installed: $dockerVersion"
        } else {
            Write-Error "Docker not installed or not in PATH"
            exit 1
        }
    } catch {
        Write-Error "Docker not installed or not in PATH"
        exit 1
    }
}

# Test 2: Check Docker Compose is installed
function Test-ComposeInstalled {
    Write-Info "Testing Docker Compose installation..."
    
    # Try docker compose (v2) first
    $composeV2 = docker compose version 2>$null
    if ($LASTEXITCODE -eq 0 -and $composeV2) {
        Write-Success "Docker Compose v2 installed: $composeV2"
        return
    }
    
    # Try docker-compose (v1)
    try {
        $composeVersion = docker-compose --version 2>$null
        if ($LASTEXITCODE -eq 0 -and $composeVersion) {
            Write-Success "Docker Compose v1 installed: $composeVersion"
        } else {
            Write-Error "Docker Compose not installed"
            exit 1
        }
    } catch {
        Write-Error "Docker Compose not installed"
        exit 1
    }
}

# Test 3: Test Docker daemon
function Test-DockerDaemon {
    Write-Info "Testing Docker daemon..."
    try {
        $dockerInfo = docker info 2>$null
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Docker daemon is running"
        } else {
            Write-Error "Docker daemon not running"
            exit 1
        }
    } catch {
        Write-Error "Docker daemon not running"
        exit 1
    }
}

# Test 4: Validate docker-compose.yml
function Test-ComposeValid {
    Write-Info "Validating docker-compose.yml..."
    try {
        $null = docker-compose config 2>$null
        if ($LASTEXITCODE -eq 0) {
            Write-Success "docker-compose.yml is valid"
        } else {
            Write-Error "docker-compose.yml has errors"
        }
    } catch {
        Write-Error "docker-compose.yml has errors"
    }
}

# Test 5: Validate docker-compose.prebuilt.yml
function Test-PrebuiltValid {
    Write-Info "Validating docker-compose.prebuilt.yml..."
    try {
        $null = docker-compose -f docker-compose.prebuilt.yml config 2>$null
        if ($LASTEXITCODE -eq 0) {
            Write-Success "docker-compose.prebuilt.yml is valid"
        } else {
            Write-Error "docker-compose.prebuilt.yml has errors"
        }
    } catch {
        Write-Error "docker-compose.prebuilt.yml has errors"
    }
}

# Test 6: Check .dockerignore exists
function Test-DockerIgnore {
    Write-Info "Checking .dockerignore..."
    if (Test-Path ".dockerignore") {
        Write-Success ".dockerignore exists"
    } else {
        Write-Warn ".dockerignore not found (optional but recommended)"
    }
}

# Test 7: Build minimal image
function Test-BuildMinimal {
    Write-Info "Testing minimal Dockerfile build..."
    Write-Info "This may take 5-10 minutes..."
    
    $logFile = "$env:TEMP\build-minimal.log"
    
    try {
        docker build -f Dockerfile.minimal -t creg-test:minimal . > $logFile 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Minimal Dockerfile builds successfully"
            
            # Check image size
            $imageInfo = docker images creg-test:minimal --format "{{.Size}}" 2>$null
            if ($imageInfo) {
                Write-Info "Minimal image size: $imageInfo"
            }
            
            # Test running
            $versionCheck = docker run --rm creg-test:minimal --version 2>&1
            if ($LASTEXITCODE -eq 0) {
                Write-Success "Minimal image runs successfully"
            } else {
                Write-Warn "Minimal image built but version check failed (may be normal)"
            }
            
            # Cleanup
            docker rmi creg-test:minimal > $null 2>&1
        } else {
            Write-Error "Minimal Dockerfile build failed"
            Write-Host "Build log (last 50 lines):"
            Get-Content $logFile -Tail 50
        }
    } catch {
        Write-Error "Minimal Dockerfile build failed: $_"
    } finally {
        if (Test-Path $logFile) {
            Remove-Item $logFile -Force -ErrorAction SilentlyContinue
        }
    }
}

# Test 8: Test pre-built compose (dry run)
function Test-PrebuiltDryRun {
    Write-Info "Testing pre-built compose (dry run)..."
    try {
        $null = docker-compose -f docker-compose.prebuilt.yml up --dry-run 2>$null
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Pre-built compose configuration is valid"
        } else {
            Write-Warn "Pre-built compose dry-run had issues (may need environment variables)"
        }
    } catch {
        Write-Warn "Pre-built compose dry-run had issues (may need environment variables)"
    }
}

# Test 9: Check required files exist
function Test-RequiredFiles {
    Write-Info "Checking required files..."
    
    $requiredFiles = @(
        "Dockerfile",
        "Dockerfile.minimal",
        "docker-compose.yml",
        "docker-compose.prebuilt.yml",
        "Cargo.toml"
    )
    
    foreach ($file in $requiredFiles) {
        if (Test-Path $file) {
            Write-Success "$file exists"
        } else {
            Write-Error "$file missing"
        }
    }
}

# Test 10: Test build context size
function Test-ContextSize {
    Write-Info "Testing Docker build context size..."
    
    try {
        # Calculate directory size
        $size = (Get-ChildItem -Recurse -ErrorAction SilentlyContinue | 
                 Measure-Object -Property Length -Sum).Sum
        $sizeMB = [math]::Round($size / 1MB, 2)
        
        if ($sizeMB -lt 500) {
            Write-Success "Build context is reasonable: ${sizeMB}MB"
        } else {
            Write-Warn "Build context is large: ${sizeMB}MB (check .dockerignore)"
        }
    } catch {
        Write-Warn "Could not calculate build context size"
    }
}

# Test 11: Check for common issues
function Test-CommonIssues {
    Write-Info "Checking for common issues..."
    
    # Check for CRLF line endings in Dockerfile
    if (Test-Path "Dockerfile") {
        $content = Get-Content -Raw "Dockerfile"
        if ($content -match "`r`n") {
            Write-Warn "Dockerfile has Windows line endings (CRLF) - should be LF for Linux containers"
        } else {
            Write-Success "Dockerfile has Unix line endings (LF)"
        }
    }
    
    # Check if port 8080 is available
    try {
        $portInUse = Get-NetTCPConnection -LocalPort 8080 -ErrorAction SilentlyContinue
        if ($portInUse) {
            Write-Warn "Port 8080 is already in use"
        } else {
            Write-Success "Port 8080 is available"
        }
    } catch {
        # If Get-NetTCPConnection fails, try alternative
        $listener = $null
        try {
            $listener = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 8080)
            $listener.Start()
            $listener.Stop()
            Write-Success "Port 8080 is available"
        } catch {
            Write-Warn "Port 8080 may be in use"
        } finally {
            if ($listener) { $listener.Stop() }
        }
    }
    
    # Check Docker Desktop WSL2 mode (recommended for Windows)
    try {
        $wslInfo = wsl -l -v 2>$null
        if ($wslInfo) {
            Write-Success "WSL2 is available (recommended for Docker Desktop)"
        }
    } catch {
        Write-Warn "WSL2 not detected - Docker Desktop may use Hyper-V backend (slower)"
    }
    
    # Check available memory
    try {
        $totalRAM = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB
        $availableRAM = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB
        
        if ($totalRAM -lt 8) {
            Write-Warn "System has only $([math]::Round($totalRAM,1))GB RAM - 8GB+ recommended for Docker builds"
        } else {
            Write-Success "System has $([math]::Round($totalRAM,1))GB RAM"
        }
        
        if ($availableRAM -lt 2048) {
            Write-Warn "Only $([math]::Round($availableRAM,0))MB RAM available - builds may fail"
        }
    } catch {
        Write-Warn "Could not check system memory"
    }
}

# Main test execution
function Main {
    Write-Host @"
$Blue╔════════════════════════════════════════════════════════════╗$NC
$Blue║       Chain Registry Docker Build Test Suite               ║$NC
$Blue╚════════════════════════════════════════════════════════════╝$NC

"@
    
    # Change to script directory
    $scriptPath = Split-Path -Parent $MyInvocation.MyCommand.Path
    if ($scriptPath) {
        Set-Location $scriptPath
    }
    
    # Run all tests
    Test-DockerInstalled
    Test-ComposeInstalled
    Test-DockerDaemon
    Test-RequiredFiles
    Test-DockerIgnore
    Test-ComposeValid
    Test-PrebuiltValid
    Test-ContextSize
    Test-CommonIssues
    Test-PrebuiltDryRun
    
    # Optional: Build test (can be skipped with -Quick)
    if (-not $Quick) {
        Write-Host ""
        Write-Info "Running build tests (use -Quick to skip)..."
        Test-BuildMinimal
    } else {
        Write-Info "Skipping build tests (-Quick mode)"
    }
    
    # Summary
    Write-Host @"

$Blue╔════════════════════════════════════════════════════════════╗$NC
$Blue║                      TEST SUMMARY                          ║$NC
$Blue╚════════════════════════════════════════════════════════════╝$NC

"@
    Write-Host "Tests Passed: $Green$script:TestsPassed$NC"
    Write-Host "Tests Failed: $Red$script:TestsFailed$NC"
    if ($script:TestsWarned -gt 0) {
        Write-Host "Warnings: $Yellow$script:TestsWarned$NC"
    }
    Write-Host ""
    
    if ($script:TestsFailed -eq 0) {
        Write-Host "$Green✓ All critical tests passed! Docker setup looks good.$NC"
        Write-Host ""
        Write-Host "Next steps:"
        Write-Host "  1. docker-compose -f docker-compose.prebuilt.yml up -d"
        Write-Host "  2. Or: docker build -f Dockerfile.minimal -t creg:minimal ."
        exit 0
    } else {
        Write-Host "$Red✗ Some tests failed. Please review the errors above.$NC"
        exit 1
    }
}

# Run main function
Main
