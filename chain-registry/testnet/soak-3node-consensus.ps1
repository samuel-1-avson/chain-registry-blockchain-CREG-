# 3-node Sepolia consensus soak: health, sync, publish, tip parity.
# Fleet targets live Ethereum Sepolia via docker-compose.3node.yml (not Anvil).
#
# Prerequisites: .\testnet\start-3node-test.ps1  (or compose up -d with sepolia-3node.env)
#
# Usage:
#   .\testnet\soak-3node-consensus.ps1
#   .\testnet\soak-3node-consensus.ps1 -SkipPublish
#   .\testnet\soak-3node-consensus.ps1 -HealthTimeoutSec 300

param(
    [switch]$SkipPublish,
    [int]$HealthTimeoutSec = 180,
    [int]$ConsensusTimeoutSec = 180
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir
Set-Location $repoRoot
. (Join-Path $scriptDir "ipfs-api.ps1")

$envFile = Join-Path $scriptDir "sepolia-3node.env"
if (Test-Path $envFile) {
    Get-Content $envFile | ForEach-Object {
        $line = $_.Trim()
        if ($line -match '^\s*(CREG_ETH_RPC|CREG_3NODE_IPFS_HOST_PORT)\s*=\s*(.+)$') {
            Set-Item -Path "Env:$($matches[1])" -Value $matches[2].Trim().Trim('"')
        }
    }
}

$node1Port = if ($env:CREG_3NODE_NODE1_API_PORT) { [int]$env:CREG_3NODE_NODE1_API_PORT } else { 28180 }
$node2Port = if ($env:CREG_3NODE_NODE2_API_PORT) { [int]$env:CREG_3NODE_NODE2_API_PORT } else { 28181 }
$node3Port = if ($env:CREG_3NODE_NODE3_API_PORT) { [int]$env:CREG_3NODE_NODE3_API_PORT } else { 28182 }
$ipfsPort = if ($env:CREG_3NODE_IPFS_HOST_PORT) { [int]$env:CREG_3NODE_IPFS_HOST_PORT } else { 15001 }

$nodes = @(
    @{ name = "node1"; port = $node1Port; validator = $true },
    @{ name = "node2"; port = $node2Port; validator = $true },
    @{ name = "node3"; port = $node3Port; validator = $false }
)

function Log($msg) {
    Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $msg"
}

function Wait-NodeHealth {
    param([int]$Port, [int]$MaxSec)
    $deadline = (Get-Date).AddSeconds($MaxSec)
    $url = "http://localhost:$Port/v1/health"
    while ((Get-Date) -lt $deadline) {
        try {
            $h = Invoke-RestMethod -Uri $url -TimeoutSec 10
            $sync = $h.validator_set_sync
            if ($h.status -eq "ok" -and $sync.state -eq "synced") {
                return $h
            }
            Log "$url status=$($h.status) validator_set_sync=$($sync.state)"
        } catch {
            Log "waiting for $url ..."
        }
        Start-Sleep -Seconds 5
    }
    throw "Health not synced on port $Port within ${MaxSec}s"
}

function Get-ChainStats {
    param([int]$Port)
    $url = "http://localhost:$Port/v1/chain/stats"
    return Invoke-RestMethod -Uri $url -TimeoutSec 15
}

function Invoke-Creg {
    param(
        [Parameter(ValueFromRemainingArguments = $true)][string[]]$CregArgs,
        [string]$LogPath = ""
    )
    $cregRelease = Join-Path $repoRoot "target\release\creg.exe"
    $cregDebug = Join-Path $repoRoot "target\debug\creg.exe"
    $bin = if (Test-Path $cregRelease) { $cregRelease } elseif (Test-Path $cregDebug) { $cregDebug } else { $null }
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    if ($bin) {
        $out = & $bin @CregArgs 2>&1
    } else {
        $out = cargo run --bin creg -p chain-registry-cli -- @CregArgs 2>&1
    }
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    if ($LogPath) {
        $out | Out-File -FilePath $LogPath -Encoding utf8
    } else {
        $out | Write-Host
    }
    return $exitCode
}

function Publish-SoakPackage {
    param([int]$ApiPort, [int]$IpfsHostPort)
    $publishEnv = Join-Path $scriptDir ".env.publish.local"
    if (Test-Path $publishEnv) {
        Get-Content $publishEnv | ForEach-Object {
            $line = $_.Trim()
            if ($line -match '^\s*(CREG_PUBLISHER_ADDRESS)\s*=\s*(.+)$') {
                Set-Item -Path "Env:$($matches[1])" -Value $matches[2].Trim().Trim('"')
            }
        }
    }

    $ipfsUrl = "http://127.0.0.1:$IpfsHostPort"
    $env:CREG_IPFS_URL = $ipfsUrl
    $zkKeysDir = Join-Path $repoRoot "circuits"
    $env:CREG_ZK_KEYS_DIR = $zkKeysDir

    if (-not (Test-CregIpfsApi -BaseUrl $ipfsUrl)) {
        throw "IPFS API not reachable at $ipfsUrl (is creg-3node-ipfs up?)"
    }

    $pubKey = Join-Path $repoRoot "publisher.key"
    if (-not (Test-Path $pubKey)) {
        throw "Missing publisher.key at $pubKey (run: cargo run --bin creg -p chain-registry-cli -- keygen publisher)"
    }
    if (-not $env:CREG_PUBLISHER_ADDRESS) {
        throw "Set CREG_PUBLISHER_ADDRESS in testnet/.env.publish.local"
    }

    $stakeCheck = Join-Path $scriptDir "check-publisher-stake.ps1"
    if (Test-Path $stakeCheck) {
        $stakeOk = $false
        for ($stakeAttempt = 1; $stakeAttempt -le 3; $stakeAttempt++) {
            & $stakeCheck -PublisherAddress $env:CREG_PUBLISHER_ADDRESS -RpcUrl $env:CREG_ETH_RPC 2>&1 | ForEach-Object { Log "  $_" }
            if ($LASTEXITCODE -eq 0) {
                $stakeOk = $true
                break
            }
            if ($stakeAttempt -lt 3) {
                Log "WARN: publisher stake check failed (attempt $stakeAttempt); retrying in 15s"
                Start-Sleep -Seconds 15
            }
        }
        if (-not $stakeOk) {
            throw "Publisher stake check failed after 3 attempts"
        }
    }

    $ts = Get-Date -Format "yyyyMMdd-HHmmss"
    $smokeDir = Join-Path $repoRoot "tmp\soak-3node-smoke"
    New-Item -ItemType Directory -Force -Path $smokeDir | Out-Null
    $ver = "1.0.$ts"
    $tar = Join-Path $smokeDir "pkg.tgz"
    $pkgDir = Join-Path $smokeDir "package"
    New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null
    $pkgJson = @{
        name        = "@creg/soak-3node-smoke"
        version     = $ver
        description = "3-node soak benign publish"
        main        = "index.js"
    } | ConvertTo-Json -Compress
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    [System.IO.File]::WriteAllText((Join-Path $pkgDir "package.json"), $pkgJson, $utf8NoBom)
    [System.IO.File]::WriteAllText((Join-Path $pkgDir "index.js"), "module.exports = () => 'soak-safe';", $utf8NoBom)
    tar -czf $tar -C $smokeDir package 2>$null
    if (-not (Test-Path $tar)) {
        throw "tar failed - install tar (Git for Windows) or use WSL"
    }

    $api = "http://localhost:$ApiPort"
    $logDir = Join-Path $repoRoot "testnet\soak-3node-logs"
    New-Item -ItemType Directory -Force -Path $logDir | Out-Null
    Log "Publishing $tar to $api (IPFS $ipfsUrl)"
    $maxAttempts = 5
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        $publishExit = Invoke-Creg --node-url $api publish $tar --key-file $pubKey `
            --publisher-address $env:CREG_PUBLISHER_ADDRESS `
            -LogPath (Join-Path $logDir "publish-$ts-attempt$attempt.txt")
        if ($publishExit -eq 0) {
            Log "Publish submitted OK (attempt $attempt)"
            return
        }
        if ($attempt -lt $maxAttempts) {
            Log "WARN: publish attempt $attempt failed (Infura stake lookup may be rate-limited); retrying in 20s"
            Start-Sleep -Seconds 20
            $ver = "1.0.$ts-r$attempt"
            $pkgJson = (@{
                name        = "creg-soak-3node-smoke"
                version     = $ver
                description = "3-node soak benign publish"
                main        = "index.js"
            } | ConvertTo-Json -Compress)
            [System.IO.File]::WriteAllText((Join-Path $pkgDir "package.json"), $pkgJson, $utf8NoBom)
            tar -czf $tar -C $smokeDir package 2>$null
        }
    }
    throw "creg publish failed after $maxAttempts attempts (see testnet/soak-3node-logs)"
}

Log "Phase 1 soak - waiting for validator_set_sync on all nodes"
foreach ($n in $nodes) {
    $h = Wait-NodeHealth -Port $n.port -MaxSec $HealthTimeoutSec
    Log "$($n.name): ok peers=$($h.peer_count) validators=$($h.validator_count)"
}

$tips = @{}
$packages = @{}
foreach ($n in $nodes) {
    $stats = Get-ChainStats -Port $n.port
    $tips[$n.name] = [int64]$stats.tip_height
    $packages[$n.name] = [int64]$stats.package_count
    Log "$($n.name) tip_height=$($stats.tip_height) packages=$($stats.package_count) peers=$($stats.peer_count)"
}

$uniqueTips = ($tips.Values | Sort-Object -Unique)
if ($uniqueTips.Count -gt 1) {
    $detail = ($tips.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ", "
    throw "Tip height mismatch before publish: $detail"
}
Log "Tip parity OK (height=$($uniqueTips[0]))"

$baselineTip = $uniqueTips[0]
$published = $false
if (-not $SkipPublish) {
    Publish-SoakPackage -ApiPort $node1Port -IpfsHostPort $ipfsPort
    $published = $true
    Log "Phase 2 soak - waiting for consensus tip advance on all nodes"
} else {
    Log "Phase 2 skipped (-SkipPublish)"
}

$deadline = (Get-Date).AddSeconds($ConsensusTimeoutSec)
do {
    Start-Sleep -Seconds 5
    $tips = @{}
    foreach ($n in $nodes) {
        $stats = Get-ChainStats -Port $n.port
        $tips[$n.name] = [int64]$stats.tip_height
    }
    $uniqueTips = ($tips.Values | Sort-Object -Unique)
    $maxTip = ($tips.Values | Measure-Object -Maximum).Maximum
    if ($published) {
        Log "tips: $(($tips.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', ')"
    }
} while (
    $uniqueTips.Count -gt 1 -or
    ($published -and $maxTip -le $baselineTip) -and
    (Get-Date) -lt $deadline
)

if ($uniqueTips.Count -gt 1) {
    $detail = ($tips.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ", "
    throw "Tip height mismatch after soak: $detail"
}
if ($published -and $maxTip -le $baselineTip) {
    throw "Tip did not advance after publish within ${ConsensusTimeoutSec}s (still at $baselineTip). Check P2P peers and validator quorum."
}

Log "Soak complete - all nodes at tip_height=$($uniqueTips[0])"
