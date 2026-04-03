# Set up IPFS CORS configuration (PowerShell version)

Write-Host "Setting up IPFS CORS..."

ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin '["*"]'
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
