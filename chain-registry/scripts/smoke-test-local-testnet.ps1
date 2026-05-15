param(
    [string]$EnvFile = ".env.local-testnet",
    [switch]$SkipExplorer,
    [switch]$SkipPublish
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not [System.IO.Path]::IsPathRooted($EnvFile)) {
    $EnvFile = Join-Path $repoRoot $EnvFile
}

$composeFile = Join-Path $repoRoot "docker-compose.local-testnet.yml"
$composeArgs = @("--project-directory", $repoRoot, "--env-file", $EnvFile, "-f", $composeFile)

function Get-DotEnvValue {
    param(
        [string]$Path,
        [string]$Key
    )

    if (-not (Test-Path $Path)) {
        return $null
    }

    $prefix = "$Key="
    foreach ($line in [System.IO.File]::ReadAllLines($Path)) {
        if ($line.StartsWith($prefix)) {
            return $line.Substring($prefix.Length).Trim()
        }
    }

    return $null
}

function Get-EnvValueOrDefault {
    param(
        [string]$Path,
        [string]$Key,
        [string]$DefaultValue
    )

    $value = Get-DotEnvValue -Path $Path -Key $Key
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $DefaultValue
    }

    return $value
}

function Invoke-AnvilCast {
    param([string[]]$Arguments)

    $output = docker compose @composeArgs exec -T -e FOUNDRY_DISABLE_NIGHTLY_WARNING=1 anvil cast @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        $message = (($output | Select-Object -Last 20) -join [Environment]::NewLine).Trim()
        throw "cast command failed: $message"
    }

    return $output
}

function Get-AnvilUintValue {
    param(
        [string[]]$CastArguments,
        [string]$Description
    )

    $output = Invoke-AnvilCast -Arguments $CastArguments
    $line = ($output | Select-Object -Last 1).Trim()
    if ($line -match "^([0-9]+)") {
        return [System.Numerics.BigInteger]::Parse($matches[1])
    }

    throw "unexpected $Description output: $line"
}

function Ensure-PublisherStake {
    param(
        [string]$PublisherAddress,
        [string]$PublisherPrivateKey,
        [string]$TokenAddress,
        [string]$StakingAddress,
        [string]$StakeAmountWei
    )

    $stakeAmount = [System.Numerics.BigInteger]::Parse($StakeAmountWei)
    $currentStake = Get-AnvilUintValue -CastArguments @(
        "call",
        $StakingAddress,
        "stakedBalance(address)(uint256)",
        $PublisherAddress,
        "--rpc-url",
        "http://127.0.0.1:8545"
    ) -Description "publisher stake"

    if ($currentStake -ge $stakeAmount) {
        return
    }

    $balance = Get-AnvilUintValue -CastArguments @(
        "call",
        $TokenAddress,
        "balanceOf(address)(uint256)",
        $PublisherAddress,
        "--rpc-url",
        "http://127.0.0.1:8545"
    ) -Description "publisher token balance"

    if ($balance -lt $stakeAmount) {
        throw "publisher $PublisherAddress only has $balance CREG and cannot stake $stakeAmountWei"
    }

    Write-Host "Staking local publisher account for publish-based liveness proof..." -ForegroundColor Cyan

    Invoke-AnvilCast -Arguments @(
        "send",
        $TokenAddress,
        "approve(address,uint256)",
        $StakingAddress,
        $StakeAmountWei,
        "--private-key",
        $PublisherPrivateKey,
        "--rpc-url",
        "http://127.0.0.1:8545"
    ) | Out-Null

    Invoke-AnvilCast -Arguments @(
        "send",
        $StakingAddress,
        "stakeAsPublisher(uint256)",
        $StakeAmountWei,
        "--private-key",
        $PublisherPrivateKey,
        "--rpc-url",
        "http://127.0.0.1:8545"
    ) | Out-Null

    Wait-ForCondition -Description "publisher stake visible for $PublisherAddress" -TimeoutSec 60 -Test {
        $updatedStake = Get-AnvilUintValue -CastArguments @(
            "call",
            $StakingAddress,
            "stakedBalance(address)(uint256)",
            $PublisherAddress,
            "--rpc-url",
            "http://127.0.0.1:8545"
        ) -Description "publisher stake"

        return $updatedStake -ge $stakeAmount
    }
}

function Wait-ForHttpEndpoint {
    param(
        [string]$Uri,
        [string]$Description,
        [int]$TimeoutSec = 180
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    do {
        try {
            return Invoke-WebRequest -Uri $Uri -TimeoutSec 10 -UseBasicParsing
        } catch {
            Start-Sleep -Seconds 2
        }
    } while ((Get-Date) -lt $deadline)

    throw "$Description did not become reachable at $Uri within ${TimeoutSec}s"
}

function Wait-ForCondition {
    param(
        [string]$Description,
        [scriptblock]$Test,
        [int]$TimeoutSec = 180,
        [int]$IntervalSec = 3
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $lastError = $null

    do {
        try {
            if (& $Test) {
                return
            }
        } catch {
            $lastError = $_.Exception.Message
        }

        Start-Sleep -Seconds $IntervalSec
    } while ((Get-Date) -lt $deadline)

    if ($lastError) {
        throw "$Description did not succeed within ${TimeoutSec}s (last error: $lastError)"
    }

    throw "$Description did not succeed within ${TimeoutSec}s"
}

function Get-Json {
    param([string]$Uri)
    Invoke-RestMethod -Uri $Uri -TimeoutSec 10
}

function Register-ValidatorIdentity {
    param(
        [string]$NodeUrl,
        [hashtable]$Registration
    )

    $request = @{
        Method = "Post"
        Uri = "$NodeUrl/v1/validators/register"
        ContentType = "application/json"
        Body = ($Registration | ConvertTo-Json -Compress)
        TimeoutSec = 10
    }

    Invoke-RestMethod @request | Out-Null
}

function New-RandomHex32 {
    $bytes = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    return ($bytes | ForEach-Object { $_.ToString("x2") }) -join ""
}

$validatorNodes = @(
    [ordered]@{
        Name = "node-1"
        Url = "http://127.0.0.1:8080"
        EvmAddress = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_GENESIS_VALIDATOR_EVM_ADDRESS" -DefaultValue "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
    },
    [ordered]@{
        Name = "node-2"
        Url = "http://127.0.0.1:8085"
        EvmAddress = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_TESTNET_VALIDATOR2_EVM_ADDRESS" -DefaultValue "0x90F79bf6EB2c4f870365E785982E1f101E93b906"
    },
    [ordered]@{
        Name = "node-3"
        Url = "http://127.0.0.1:8086"
        EvmAddress = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_TESTNET_VALIDATOR3_EVM_ADDRESS" -DefaultValue "0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65"
    }
)

$publisherAddress = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_TESTNET_PUBLISHER_EVM_ADDRESS" -DefaultValue (Get-EnvValueOrDefault -Path $EnvFile -Key "FAUCET_ADDRESS" -DefaultValue "0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
$publisherPrivateKey = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_TESTNET_PUBLISHER_PRIVATE_KEY" -DefaultValue (Get-DotEnvValue -Path $EnvFile -Key "FAUCET_PRIVATE_KEY")
$publisherStakeWei = Get-EnvValueOrDefault -Path $EnvFile -Key "LOCAL_TESTNET_PUBLISHER_STAKE_WEI" -DefaultValue "1000000000000000000"
$tokenAddress = Get-DotEnvValue -Path $EnvFile -Key "LOCAL_TESTNET_TOKEN_ADDR"
$stakingAddress = Get-DotEnvValue -Path $EnvFile -Key "LOCAL_TESTNET_STAKING_ADDR"

if (
    [string]::IsNullOrWhiteSpace($publisherAddress) -or
    [string]::IsNullOrWhiteSpace($publisherPrivateKey) -or
    [string]::IsNullOrWhiteSpace($tokenAddress) -or
    [string]::IsNullOrWhiteSpace($stakingAddress)
) {
    throw "missing local publisher or staking configuration in $EnvFile"
}

$allNodes = @(
    $validatorNodes[0],
    $validatorNodes[1],
    $validatorNodes[2],
    [ordered]@{
        Name = "observer"
        Url = "http://127.0.0.1:8087"
        EvmAddress = $null
    }
)

Write-Host "Waiting for distributed local services..." -ForegroundColor Cyan
foreach ($node in $allNodes) {
    $null = Wait-ForHttpEndpoint -Uri "$($node.Url)/v1/health" -Description "$($node.Name) health"
}

$null = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8082/health" -Description "faucet health"
$null = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8083/health" -Description "relayer health"
$null = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:8084/health" -Description "indexer health"
if (-not $SkipExplorer) {
    $null = Wait-ForHttpEndpoint -Uri "http://127.0.0.1:3007" -Description "explorer health"
}

Write-Host "Registering validator identities across the local cluster..." -ForegroundColor Cyan
$registrations = @()
foreach ($validator in $validatorNodes) {
    $runtimeConfig = Get-Json -Uri "$($validator.Url)/v1/runtime/config"
    if ([string]::IsNullOrWhiteSpace($runtimeConfig.validator_pubkey)) {
        throw "Runtime config for $($validator.Name) did not expose validator_pubkey"
    }

    $registrations += @{
        evm_address = $validator.EvmAddress
        node_id = $runtimeConfig.node_id
        ed25519_pubkey = $runtimeConfig.validator_pubkey
        alias = $runtimeConfig.node_id
    }
}

foreach ($node in $allNodes) {
    foreach ($registration in $registrations) {
        Register-ValidatorIdentity -NodeUrl $node.Url -Registration $registration
    }
}

$expectedValidatorCount = $registrations.Count

foreach ($node in $allNodes) {
    Wait-ForCondition -Description "validator registrations visible on $($node.Name)" -Test {
        $current = Get-Json -Uri "$($node.Url)/v1/validators/registrations"
        return $current.Count -ge $expectedValidatorCount -and @($current | Where-Object { $_.registered_with_node }).Count -ge $expectedValidatorCount
    }
}

foreach ($node in $allNodes) {
    Wait-ForCondition -Description "active validator set visible on $($node.Name)" -Test {
        $stats = Get-Json -Uri "$($node.Url)/v1/chain/stats"
        return $stats.validator_count -ge $expectedValidatorCount -and $stats.active_validators -ge $expectedValidatorCount
    }
}

foreach ($node in $allNodes) {
    Wait-ForCondition -Description "non-zero P2P mesh on $($node.Name)" -Test {
        $status = Get-Json -Uri "$($node.Url)/v1/p2p/status"
        return $status.peer_count -ge 1
    }
}

if (-not $SkipPublish) {
    Ensure-PublisherStake -PublisherAddress $publisherAddress -PublisherPrivateKey $publisherPrivateKey -TokenAddress $tokenAddress -StakingAddress $stakingAddress -StakeAmountWei $publisherStakeWei

    Write-Host "Submitting a synthetic package publish to force block production..." -ForegroundColor Cyan
    $publishVersion = "1.0.$(Get-Date -Format 'yyyyMMddHHmmss')"
    $publisherSigningKey = New-RandomHex32
    & (Join-Path $PSScriptRoot "test-publish.ps1") `
        -Version $publishVersion `
        -PublisherAddress $publisherAddress `
        -PublisherSigningKey $publisherSigningKey `
        -EnvFile $EnvFile `
        -NodeApi $validatorNodes[0].Url `
        -IpfsApi "http://localhost:5001"

    foreach ($node in $allNodes) {
        Wait-ForCondition -Description "block propagation beyond genesis on $($node.Name)" -TimeoutSec 120 -Test {
            $stats = Get-Json -Uri "$($node.Url)/v1/chain/stats"
            return $stats.tip_height -ge 1 -and $stats.block_count -ge 2
        }
    }
}

$summary = foreach ($node in $allNodes) {
    $stats = Get-Json -Uri "$($node.Url)/v1/chain/stats"
    $p2p = Get-Json -Uri "$($node.Url)/v1/p2p/status"
    [ordered]@{
        name = $node.Name
        validator_count = $stats.validator_count
        active_validators = $stats.active_validators
        peer_count = $p2p.peer_count
        tip_height = $stats.tip_height
        block_count = $stats.block_count
    }
}

Write-Host "Distributed local testnet smoke test passed." -ForegroundColor Green
$summary | ConvertTo-Json -Depth 4