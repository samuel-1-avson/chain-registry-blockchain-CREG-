# test_cleanup.ps1
# Chain Registry Deep Purge Utility
# Resets the local system to a clean state for production use.

Write-Host "⚠️  WARNING: Starting Deep Purge of Chain Registry Mock Data..." -ForegroundColor Yellow
$CONFIRM = Read-Host "Are you sure you want to delete all local history? (y/n)"
if ($CONFIRM -ne "y") { Write-Host "Cleanup aborted."; exit }

# 1. Stop any running processes (optional but recommended)
Write-Host "Stopping Node processes..."
Get-Process "creg-node" -ErrorAction SilentlyContinue | Stop-Process -Force

# 2. Cleanup sled database
Write-Host "Purging Chain Database (sled)..."
if (Test-Path "./chain-data") {
    Remove-Item -Path "./chain-data" -Recurse -Force
    Write-Host "[OK] Database purged." -ForegroundColor Green
} else {
    Write-Host "[INFO] No chain-data folder found."
}

# 3. Cleanup IPFS Cache
Write-Host "Purging IPFS Metadata Cache..."
if (Test-Path "./ipfs-data") {
    Remove-Item -Path "./ipfs-data" -Recurse -Force
    Write-Host "[OK] IPFS cache purged." -ForegroundColor Green
}

# 4. Cleanup temporary package artifacts
Write-Host "Cleaning temporary workspace data..."
Get-ChildItem -Path "." -Filter "publish_log.txt" | Remove-Item -Force
Get-ChildItem -Path "." -Filter "node_err.log" | Remove-Item -Force

Write-Host "------------------------------------------------"
Write-Host "✅ DEEP PURGE COMPLETE." -ForegroundColor Green
Write-Host "Next Step: Run 'creg-node' to initialize a fresh Genesis block."
Write-Host "------------------------------------------------"
