@echo off
REM ==============================================================================
REM Chain Registry (CREG) - CLI Staking Utility (Windows CMD Wrapper)
REM ==============================================================================
REM Runs the standalone Node.js staking script with no external Foundry dependencies.

node "%~dp0stake-validator.js" %*
