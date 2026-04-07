param(
    [string]$ManifestPath = "contracts/deployments/latest.json",
    [string]$EnvPath = ".env.testnet",
    [string]$ArtifactEnvPath = "testnet/artifacts/testnet.env",
    [string]$ArtifactJsonPath = "testnet/artifacts/testnet-contracts.json"
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

if (-not (Test-Path $ManifestPath)) {
    $fallbackManifestPath = "testnet/artifacts/testnet-contracts.json"
    if (Test-Path $fallbackManifestPath) {
        $ManifestPath = $fallbackManifestPath
    } else {
        throw "Manifest not found at $ManifestPath"
    }
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

Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_GOVERNANCE_ADDR" -Value $manifest.governance
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_STAKING_ADDR" -Value $manifest.staking
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_REPUTATION_ADDR" -Value $manifest.reputation
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_VRF_ADDR" -Value $manifest.vrf
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_REGISTRY_ADDR" -Value $manifest.registry
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_APPEAL_ADDR" -Value $manifest.appeal
Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_TOKEN_ADDR" -Value $manifest.cregToken
Set-Or-AddEnvValue -Lines $envLines -Key "FAUCET_TOKEN_CONTRACT" -Value $manifest.cregToken

if ($manifest.PSObject.Properties.Name.Contains("zkVerifier")) {
    Set-Or-AddEnvValue -Lines $envLines -Key "TESTNET_ZK_VERIFIER_ADDR" -Value $manifest.zkVerifier
}

$envFilePath = Join-Path (Get-Location) $EnvPath
$artifactEnvFilePath = Join-Path (Get-Location) $ArtifactEnvPath
$artifactJsonFilePath = Join-Path (Get-Location) $ArtifactJsonPath

[System.IO.File]::WriteAllLines($envFilePath, $envLines)

$artifactEnv = @(
    "TESTNET_GOVERNANCE_ADDR=$($manifest.governance)",
    "TESTNET_STAKING_ADDR=$($manifest.staking)",
    "TESTNET_REPUTATION_ADDR=$($manifest.reputation)",
    "TESTNET_VRF_ADDR=$($manifest.vrf)",
    "TESTNET_REGISTRY_ADDR=$($manifest.registry)",
    "TESTNET_APPEAL_ADDR=$($manifest.appeal)",
    "TESTNET_TOKEN_ADDR=$($manifest.cregToken)",
    "TESTNET_CHAIN_ID=$($manifest.chainId)",
    "TESTNET_DEPLOYED_AT=$($manifest.deployedAt)",
    "TESTNET_RPC_URL=http://localhost:8545",
    "TESTNET_NODE_URL=http://localhost:8080",
    "FAUCET_URL=http://localhost:8082"
)

if ($manifest.PSObject.Properties.Name.Contains("zkVerifier")) {
    $artifactEnv += "TESTNET_ZK_VERIFIER_ADDR=$($manifest.zkVerifier)"
}

[System.IO.Directory]::CreateDirectory((Split-Path -Parent $artifactEnvFilePath)) | Out-Null
[System.IO.File]::WriteAllLines($artifactEnvFilePath, $artifactEnv)

$artifactJson = [ordered]@{
    network = "testnet"
    chainId = [string]$manifest.chainId
    rpcUrl = "http://localhost:8545"
    nodeUrl = "http://localhost:8080"
    faucetUrl = "http://localhost:8082"
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

[System.IO.Directory]::CreateDirectory((Split-Path -Parent $artifactJsonFilePath)) | Out-Null
$artifactJson | ConvertTo-Json -Depth 6 | Set-Content -Path $artifactJsonFilePath -Encoding utf8

Write-Host "Synchronized testnet env and artifacts from $ManifestPath" -ForegroundColor Green