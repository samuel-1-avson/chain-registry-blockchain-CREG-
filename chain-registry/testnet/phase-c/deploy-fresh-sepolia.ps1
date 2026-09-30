param(
    [switch]$Broadcast,
    [string]$RpcUrl = 'https://ethereum-sepolia-rpc.publicnode.com'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$project = 'gen-lang-client-0022105784'
$manifest = 'contracts/deployments/sepolia-phase-c-2026-09-25.json'
$expectedDeployer = '0x3A3206D683441AA91cCA7CD5757B1F0ca463213A'
$signers = @(
    '0x6323F9780Edf77763CF77401e559711372D76b8C',
    '0x5dBD86B868F576e0C5b6dee9f9700A650FCFF639',
    '0x3Febc440Bb85AD51B83576b3Dcc53BD253D75cCc'
)

if (-not $Broadcast) {
    Write-Output "Fresh Sepolia deployment is prepared, not broadcast. Deployer: $expectedDeployer"
    Write-Output "Governance: 2-of-3. Manifest: $manifest"
    exit 0
}

$forge = Join-Path $repoRoot 'testnet/.tools/foundry/forge.exe'
if (-not (Test-Path -LiteralPath $forge)) { throw 'Foundry forge.exe is missing.' }
if (Test-Path -LiteralPath (Join-Path $repoRoot $manifest)) {
    throw 'Phase C deployment manifest already exists; inspect the previous broadcast before retrying.'
}

$key = (& gcloud secrets versions access latest --secret=creg-phase-c-treasury-v2-evm --project=$project)
if ($LASTEXITCODE -ne 0) {
    throw 'Could not read the new Phase C treasury/deployer key from Secret Manager.'
}
$key = ([string]$key).Trim()
if ($key -notmatch '^0x[0-9a-fA-F]{64}$') { throw 'New Phase C treasury/deployer key has invalid format.' }
$env:DEPLOYER_KEY = $key
$key = $null
try {
    Push-Location $repoRoot
    $actualDeployer = (& node -e 'const {ethers}=require("./scripts/evm-tool/node_modules/ethers");try{console.log(new ethers.Wallet((process.env.DEPLOYER_KEY||"").trim()).address)}catch{console.error("Invalid deployer key");process.exitCode=1}').Trim()
    if ($LASTEXITCODE -ne 0 -or $actualDeployer.ToLowerInvariant() -ne $expectedDeployer.ToLowerInvariant()) {
        throw 'Secret Manager deployer key does not match the approved Phase C address.'
    }
    $env:SEPOLIA_RPC_URL = $RpcUrl
    $env:GOVERNANCE_THRESHOLD = '2'
    $env:GENESIS_SIGNERS = ConvertTo-Json -InputObject $signers -Compress
    $env:DEPLOYMENT_MANIFEST_PATH = $manifest
    [Environment]::SetEnvironmentVariable('CREG_BRIDGE_KEY', $null, 'Process')
    [Environment]::SetEnvironmentVariable('FAUCET_ADDRESS', $null, 'Process')
    [Environment]::SetEnvironmentVariable('FINALIZE_RELAY_ADDRESS', $null, 'Process')

    $chainId = (& node -e 'const {ethers}=require("./scripts/evm-tool/node_modules/ethers");const p=new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC_URL);p.getNetwork().then(n=>console.log(String(n.chainId))).catch(e=>{console.error(e.message);process.exitCode=1})').Trim()
    if ($LASTEXITCODE -ne 0 -or $chainId -ne '11155111') { throw 'RPC is not live Ethereum Sepolia.' }
    $balance = (& node -e 'const {ethers}=require("./scripts/evm-tool/node_modules/ethers");try{const p=new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC_URL);p.getBalance(new ethers.Wallet((process.env.DEPLOYER_KEY||"").trim()).address).then(x=>console.log(String(x))).catch(()=>{console.error("Balance query failed");process.exitCode=1})}catch{console.error("Invalid deployer key");process.exitCode=1}').Trim()
    if ($LASTEXITCODE -ne 0 -or [System.Numerics.BigInteger]::Parse($balance) -le 0) {
        throw "Fresh deployer $expectedDeployer needs real Sepolia ETH before broadcast."
    }

    Write-Output "Broadcasting fresh real Sepolia contracts from $actualDeployer with 2-of-3 governance."
    & $forge script contracts/script/DeploySepolia.s.sol:DeploySepolia `
        --rpc-url $RpcUrl --broadcast --skip-simulation --slow --chain-id 11155111 -vv
    if ($LASTEXITCODE -ne 0) { throw 'Phase C Sepolia deployment failed; inspect Foundry broadcast records before retrying.' }
    if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $manifest))) {
        throw 'Broadcast completed but the Phase C deployment manifest is missing.'
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot $manifest) -Destination (Join-Path $PSScriptRoot 'deployment-manifest.json') -Force
    Write-Output "Phase C Sepolia manifest: $manifest"
} finally {
    [Environment]::SetEnvironmentVariable('DEPLOYER_KEY', $null, 'Process')
    Pop-Location
}
