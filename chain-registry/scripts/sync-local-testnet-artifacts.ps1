param(
    [string]$ManifestPath = "contracts/deployments/latest.json",
    [string]$EnvPath = ".env.local-testnet",
    [string]$ArtifactEnvPath = "testnet/artifacts/local-testnet.env",
    [string]$ArtifactJsonPath = "testnet/artifacts/local-testnet-contracts.json"
)

$ErrorActionPreference = "Stop"

function Set-Or-AddEnvValue {
    param(
        [System.Collections.Generic.List[string]]$Lines,
        [string]$Key,
        [string]$Value
    )

    $prefix = "$Key="
    $index = -1
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i].StartsWith($prefix)) {
            $index = $i
            break
        }
    }

    $nextLine = "$Key=$Value"
    if ($index -ge 0) {
        $Lines[$index] = $nextLine
    } else {
        $Lines.Add($nextLine) | Out-Null
    }
}

function Resolve-OutputPath {
    param([string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }

    return Join-Path (Get-Location) $Path
}

if (-not (Test-Path $ManifestPath)) {
    throw "Manifest not found at $ManifestPath"
}

$manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json

$requiredKeys = @(
    "governance",
    "staking",
    "reputation",
    "vrf",
    "registry",
    "appeal",
    "cregToken"
)

foreach ($key in $requiredKeys) {
    if (-not $manifest.PSObject.Properties.Name.Contains($key)) {
        throw "Manifest missing required key: $key"
    }
}

$envLines = [System.Collections.Generic.List[string]]::new()
if (Test-Path $EnvPath) {
    foreach ($line in [System.IO.File]::ReadAllLines((Resolve-Path $EnvPath))) {
        $envLines.Add($line) | Out-Null
    }
}

Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_GOVERNANCE_ADDR" -Value $manifest.governance
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_STAKING_ADDR" -Value $manifest.staking
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_REPUTATION_ADDR" -Value $manifest.reputation
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_VRF_ADDR" -Value $manifest.vrf
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_REGISTRY_ADDR" -Value $manifest.registry
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_APPEAL_ADDR" -Value $manifest.appeal
Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_TOKEN_ADDR" -Value $manifest.cregToken
Set-Or-AddEnvValue -Lines $envLines -Key "FAUCET_TOKEN_CONTRACT" -Value $manifest.cregToken

if ($manifest.PSObject.Properties.Name.Contains("zkVerifier")) {
    Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_ZK_VERIFIER_ADDR" -Value $manifest.zkVerifier
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewards")) {
    Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_VALIDATOR_REWARDS_ADDR" -Value $manifest.validatorRewards
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewardsTreasury")) {
    Set-Or-AddEnvValue -Lines $envLines -Key "LOCAL_TESTNET_VALIDATOR_REWARDS_TREASURY" -Value $manifest.validatorRewardsTreasury
}

$envFilePath = Resolve-OutputPath -Path $EnvPath
$artifactEnvFilePath = Resolve-OutputPath -Path $ArtifactEnvPath
$artifactJsonFilePath = Resolve-OutputPath -Path $ArtifactJsonPath

[System.IO.File]::WriteAllLines($envFilePath, $envLines)

$artifactEnv = @(
    "LOCAL_TESTNET_GOVERNANCE_ADDR=$($manifest.governance)",
    "LOCAL_TESTNET_STAKING_ADDR=$($manifest.staking)",
    "LOCAL_TESTNET_REPUTATION_ADDR=$($manifest.reputation)",
    "LOCAL_TESTNET_VRF_ADDR=$($manifest.vrf)",
    "LOCAL_TESTNET_REGISTRY_ADDR=$($manifest.registry)",
    "LOCAL_TESTNET_APPEAL_ADDR=$($manifest.appeal)",
    "LOCAL_TESTNET_TOKEN_ADDR=$($manifest.cregToken)",
    "LOCAL_TESTNET_CHAIN_ID=$($manifest.chainId)",
    "LOCAL_TESTNET_DEPLOYED_AT=$($manifest.deployedAt)",
    "LOCAL_TESTNET_RPC_URL=http://localhost:8545",
    "LOCAL_TESTNET_NODE1_URL=http://localhost:8080",
    "LOCAL_TESTNET_NODE2_URL=http://localhost:8085",
    "LOCAL_TESTNET_NODE3_URL=http://localhost:8086",
    "LOCAL_TESTNET_OBSERVER_URL=http://localhost:8087",
    "LOCAL_TESTNET_FAUCET_URL=http://localhost:8082",
    "LOCAL_TESTNET_EXPLORER_URL=http://localhost:3007"
)

if ($manifest.PSObject.Properties.Name.Contains("zkVerifier")) {
    $artifactEnv += "LOCAL_TESTNET_ZK_VERIFIER_ADDR=$($manifest.zkVerifier)"
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewards")) {
    $artifactEnv += "LOCAL_TESTNET_VALIDATOR_REWARDS_ADDR=$($manifest.validatorRewards)"
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewardsTreasury")) {
    $artifactEnv += "LOCAL_TESTNET_VALIDATOR_REWARDS_TREASURY=$($manifest.validatorRewardsTreasury)"
}

[System.IO.Directory]::CreateDirectory((Split-Path -Parent $artifactEnvFilePath)) | Out-Null
[System.IO.File]::WriteAllLines($artifactEnvFilePath, $artifactEnv)

$artifactJson = [ordered]@{
    network = "local-testnet"
    chainId = [string]$manifest.chainId
    rpcUrl = "http://localhost:8545"
    nodeUrls = [ordered]@{
        node1 = "http://localhost:8080"
        node2 = "http://localhost:8085"
        node3 = "http://localhost:8086"
        observer = "http://localhost:8087"
    }
    faucetUrl = "http://localhost:8082"
    explorerUrl = "http://localhost:3007"
    deployedAt = [string]$manifest.deployedAt
    deployer = $manifest.deployer
    contracts = [ordered]@{
        Governance = [ordered]@{ address = $manifest.governance }
        Staking = [ordered]@{ address = $manifest.staking }
        Reputation = [ordered]@{ address = $manifest.reputation }
        VRF = [ordered]@{ address = $manifest.vrf }
        Registry = [ordered]@{ address = $manifest.registry }
        Appeal = [ordered]@{ address = $manifest.appeal }
        CregToken = [ordered]@{ address = $manifest.cregToken }
    }
}

if ($manifest.PSObject.Properties.Name.Contains("zkVerifier")) {
    $artifactJson.contracts["ZKVerifier"] = [ordered]@{ address = $manifest.zkVerifier }
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewards")) {
    $artifactJson.contracts["ValidatorRewards"] = [ordered]@{ address = $manifest.validatorRewards }
}

if ($manifest.PSObject.Properties.Name.Contains("validatorRewardsTreasury")) {
    $artifactJson["validatorRewardsTreasury"] = $manifest.validatorRewardsTreasury
}

[System.IO.Directory]::CreateDirectory((Split-Path -Parent $artifactJsonFilePath)) | Out-Null
$artifactJson | ConvertTo-Json -Depth 6 | Set-Content -Path $artifactJsonFilePath -Encoding utf8

Write-Host "Synchronized local testnet env and artifacts from $ManifestPath" -ForegroundColor Green