@echo off
setlocal EnableDelayedExpansion

echo ========================================
echo Chain Registry Testnet Deployment
echo ========================================
echo.

REM Check if Anvil is running
echo [1/4] Checking Anvil connection...
docker exec creg-testnet-anvil cast block-number --rpc-url http://localhost:8545 > temp_block.txt 2>&1
if errorlevel 1 (
    echo ERROR: Anvil is not running!
    echo Start it with: docker-compose -f docker-compose.testnet.yml up -d anvil
    pause
    exit /b 1
)
set /p BLOCK=<temp_block.txt
echo OK: Anvil connected (Block: %BLOCK%)
del temp_block.txt

echo.
echo [2/4] Deploying Test CREG Token...
echo This may take 30-60 seconds...
echo.

docker run --rm -v "F:/project/chain-registry/chain-registry:/workspace" -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "forge create TestCregToken.sol:TestCregToken --rpc-url http://creg-testnet-anvil:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --constructor-args 'Test CREG Token' 'tCREG'" > deploy_token_output.txt 2>&1

type deploy_token_output.txt

echo.
echo ========================================
echo Deployment attempt complete!
echo ========================================
echo.
echo Check the output above for:
echo   - "Deployed to: 0x..." -- This is your token address
echo   - Any error messages
echo.
echo If successful, save the token address and run Step 2.
echo.
pause
