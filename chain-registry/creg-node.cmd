@echo off
REM ==============================================================================
REM Chain Registry (CREG) -- Node Daemon Helper (creg-node)
REM ==============================================================================

if "%1"=="--status" (
    powershell.exe -NoProfile -Command "Invoke-RestMethod http://localhost:8080/v1/chain/stats | Format-List"
    exit /b %ERRORLEVEL%
)
if "%1"=="--logs" (
    docker logs -f creg-validator-ext-val-primary
    exit /b %ERRORLEVEL%
)
if "%1"=="--start" (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0testnet\start-external-validator.ps1" %*
    exit /b %ERRORLEVEL%
)

echo ============================================================
echo    Chain Registry Node Daemon (creg-node)
echo ============================================================
echo  Status      : Container running (creg-validator-ext-val-primary)
echo  Local API   : http://localhost:8080
echo  P2P Port    : 9000
echo.
echo Quick commands:
echo   .\creg-node.cmd --status   View live node sync and validator stats
echo   .\creg-node.cmd --logs     Follow live container logs
echo   .\creg-node.cmd --start    Start / restart the node container
echo   creg console               Open the interactive terminal console
echo   creg --help                View user and operator CLI commands
echo ============================================================
