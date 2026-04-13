#requires -Version 5.1
<#
.SYNOPSIS
    Generates Ed25519 validator keys for the Chain Registry testnet on Windows.

.DESCRIPTION
    Produces .env.testnet and config/validator-set.json for the testnet validator set.
    The local Docker bootstrap flow runs one validator node on this machine.
    Requires the .NET System.Security.Cryptography assembly (built into Windows).

.EXAMPLE
    .\scripts\generate-testnet-keys.ps1 -Nodes 1 -Output .env.testnet
#>
param(
    [int]$Nodes = 1,
    [string]$Output = ".env.testnet",
    [string]$ConfigDir = "config"
)

$ErrorActionPreference = "Stop"

# Load Ed25519 support from .NET
Add-Type -AssemblyName System.Security | Out-Null

function Generate-KeyPair {
    $seed = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($seed)
    
    # .NET 5+ has Ed25519 built in
    $sk = [System.Security.Cryptography.Ed25519]::Create()
    # We need to import the seed; Create() generates a random one but we want deterministic
    # Unfortunately .NET Framework doesn't expose Ed25519 directly.
    # Fallback: just return the random seed hex and warn about pubkey derivation.
    $priv = ([System.BitConverter]::ToString($seed) -replace '-').ToLower()
    return $priv, $null
}

# Try using libsodium via P/Invoke if available
function Try-SodiumKeygen {
    try {
        $sodium = Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Sodium {
    [DllImport("libsodium", CallingConvention = CallingConvention.Cdecl)]
    public static extern int crypto_sign_seed_keypair(byte[] pk, byte[] sk, byte[] seed);
}
"@ -PassThru -ErrorAction Stop
        
        $seed = New-Object byte[] 32
        [System.Security.Cryptography.RandomNumberGenerator]::Fill($seed)
        $pk = New-Object byte[] 32
        $sk = New-Object byte[] 64
        [Sodium]::crypto_sign_seed_keypair($pk, $sk, $seed) | Out-Null
        $priv = ([System.BitConverter]::ToString($seed) -replace '-').ToLower()
        $pub  = ([System.BitConverter]::ToString($pk) -replace '-').ToLower()
        return $priv, $pub
    } catch {
        return $null, $null
    }
}

Write-Host "Generating validator definitions for $Nodes node(s)..." -ForegroundColor Cyan

New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null

$envLines = @(
    "# Chain Registry Testnet Environment"
    "# Generated for $Nodes validator node(s)"
    ""
    "# Host-facing endpoints for commands run outside Docker"
    "CREG_ETH_RPC=http://localhost:8545"
    "CREG_IPFS_URL=http://localhost:5001"
    ""
    "# Docker-internal endpoints for containers inside docker-compose.testnet.yml"
    "CREG_DOCKER_ETH_RPC=http://anvil:8545"
    "CREG_DOCKER_IPFS_URL=http://ipfs:5001"
    "CREG_PG_URL=postgres://creg:creg@postgres:5432/chain_registry"
    ""
    "# Default Anvil deployment and faucet accounts"
    "DEPLOYER_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    "CREG_BRIDGE_KEY=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    "FAUCET_PRIVATE_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    "FAUCET_ADDRESS=0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
    "FAUCET_INITIAL_BALANCE=10000000000000000000000"
    "FAUCET_DRIP_AMOUNT=1000000000000000000000"
    "FAUCET_COOLDOWN_SECS=60"
    ""
)

$validators = @()
$hasSodium = $false

for ($i = 1; $i -le $Nodes; $i++) {
    $nodeId = "node-$i"
    $priv, $pub = Try-SodiumKeygen
    if ($pub) {
        $hasSodium = $true
    } else {
        $priv, $pub = Generate-KeyPair
        if (-not $pub) {
            $pub = "PLEASE_DERIVE_FROM_$priv"
        }
    }
    
    $envLines += "NODE${i}_VALIDATOR_KEY=$priv"
    $validators += @{
        id = $nodeId
        alias = "Validator-$i"
        pubkey = $pub
        stake = 100
        reputation = 100
        status = "online"
    }
    
    $pubDisplay = if ($pub.Length -gt 24) { $pub.Substring(0,16) + "..." + $pub.Substring($pub.Length-8) } else { $pub }
    Write-Host "  $nodeId`: pubkey = $pubDisplay"
}

$validatorSetJson = ($validators | ConvertTo-Json -Depth 10 -Compress)
$envLines += ""
$envLines += "VALIDATOR_SET_JSON=$validatorSetJson"

# Publisher key
$pubPriv, $pubPub = Try-SodiumKeygen
if (-not $pubPub) { $pubPriv, $pubPub = Generate-KeyPair }
$envLines += ""
$envLines += "TESTNET_PUBLISHER_KEY=$pubPriv"
$envLines += "TESTNET_PUBLISHER_PUBKEY=$pubPub"

Set-Content -Path $Output -Value ($envLines -join "`n") -NoNewline
Add-Content -Path $Output -Value "" -NoNewLine
Write-Host "`nWrote $Output" -ForegroundColor Green

$validatorSetPath = Join-Path $ConfigDir "validator-set.json"
@{ validators = $validators } | ConvertTo-Json -Depth 10 | Set-Content -Path $validatorSetPath
Write-Host "Wrote $validatorSetPath" -ForegroundColor Green

if (-not $hasSodium) {
    Write-Host "`nWARNING: libsodium not found. Public keys were NOT derived." -ForegroundColor Yellow
    Write-Host "To derive pubkeys, install the 'creg' CLI and run:" -ForegroundColor Yellow
    Write-Host "  creg keygen validator --seed <NODE1_VALIDATOR_KEY>" -ForegroundColor Yellow
}

Write-Host "`nNext steps:" -ForegroundColor Cyan
Write-Host "  1. Review $Output"
Write-Host "  2. Deploy the bootstrap host (single validator on this machine):"
Write-Host "     docker compose -f docker-compose.testnet.yml --env-file $Output up -d --build"
Write-Host "  3. Run stress test:"
Write-Host "     python scripts/stress-test.py --nodes $Nodes --packages 1000"
