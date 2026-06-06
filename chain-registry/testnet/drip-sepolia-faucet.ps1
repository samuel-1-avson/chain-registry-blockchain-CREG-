# Request CREG from the local Sepolia faucet HTTP API (PoW disabled when FAUCET_POW_DISABLED=true).
#
# Usage:
#   .\testnet\start-sepolia-faucet.ps1   # other window
#   .\testnet\drip-sepolia-faucet.ps1 -Address 0x8E468575568756E210caA39D04A24a8bF2266B84

param(
    [Parameter(Mandatory = $true)]
    [string]$Address,
    [string]$FaucetUrl = "http://127.0.0.1:8082"
)

$ErrorActionPreference = "Stop"

if ($Address -notmatch '^0x[a-fA-F0-9]{40}$') { throw "Invalid address" }

$base = $FaucetUrl.TrimEnd('/')
$health = curl.exe -s -m 5 "$base/health"
if ($LASTEXITCODE -ne 0 -or $health -notmatch 'healthy') {
    throw "Faucet not reachable at $base - run .\testnet\start-sepolia-faucet.ps1 (keep that window open)"
}

Write-Host "Dripping CREG to $Address via $base/api/drip ..." -ForegroundColor Cyan
try {
    $result = Invoke-RestMethod -Uri "$base/api/drip" -Method Post -ContentType "application/json; charset=utf-8" `
        -Body (@{ address = $Address } | ConvertTo-Json -Compress) -TimeoutSec 120
} catch {
    if ($_.ErrorDetails.Message) { Write-Host $_.ErrorDetails.Message }
    throw "Drip HTTP failed: $($_.Exception.Message). Is .\testnet\start-sepolia-faucet.ps1 running?"
}

$result | ConvertTo-Json -Compress | Write-Host
if (-not $result.success) {
    throw "Drip failed: $($result.message). Check faucet CREG/ETH balance and cooldown."
}
Write-Host "OK" -ForegroundColor Green
if ($result.tx_hash) { Write-Host "  tx: $($result.tx_hash)" -ForegroundColor DarkGray }
if ($result.token_tx_hash) { Write-Host "  token tx: $($result.token_tx_hash)" -ForegroundColor DarkGray }
