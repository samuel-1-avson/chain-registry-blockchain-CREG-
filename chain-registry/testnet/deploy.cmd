@echo off
echo Deploying Test CREG Token...
echo.
cd /d F:\project\chain-registry\chain-registry
docker run --rm -v "%CD%:/workspace" -w /workspace/contracts/testnet --network testnet_creg-testnet ghcr.io/foundry-rs/foundry:latest sh -c "forge create TestCregToken.sol:TestCregToken --rpc-url http://creg-testnet-anvil:8545 --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --constructor-args 'Test CREG Token' 'tCREG'"
echo.
echo If successful, note the 'Deployed to:' address above!
pause
