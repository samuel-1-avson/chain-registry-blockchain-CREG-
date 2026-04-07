# Set up IPFS CORS configuration (PowerShell version)
# For testnet: restrict to known origins only
# For local dev: set $env:IPFS_CORS_ORIGIN = "*" to allow all origins

$CorsOrigin = if ($env:IPFS_CORS_ORIGIN) { $env:IPFS_CORS_ORIGIN } else { "http://localhost:3000,http://localhost:8080,http://creg-testnet.local" }
$Origins = $CorsOrigin -split "," | ForEach-Object { "`"$($_.Trim())`"" }
$OriginsJson = "[" + ($Origins -join ",") + "]"

Write-Host "Setting up IPFS CORS..."
Write-Host "  Allowed origins: $CorsOrigin"

ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin $OriginsJson
if ($LASTEXITCODE -ne 0) {
    Write-Error "Failed to set Access-Control-Allow-Origin"
    exit 1
}

ipfs config --json API.HTTPHeaders.Access-Control-Allow-Methods '["PUT", "POST", "GET"]'
if ($LASTEXITCODE -ne 0) {
    Write-Error "Failed to set Access-Control-Allow-Methods"
    exit 1
}

Write-Host "CORS set!"
