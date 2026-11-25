@echo off
setlocal
if exist "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\Common7\Tools\VsDevCmd.bat" (
  call "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\Common7\Tools\VsDevCmd.bat" -arch=amd64 -host_arch=amd64
) else if exist "C:\Program Files\Microsoft Visual Studio\18\Community\Common7\Tools\VsDevCmd.bat" (
  call "C:\Program Files\Microsoft Visual Studio\18\Community\Common7\Tools\VsDevCmd.bat" -arch=amd64 -host_arch=amd64
)
set CARGO_TARGET_DIR=%USERPROFILE%\.cargo-targets\creg-desktop
set CARGO_INCREMENTAL=0
set CARGO_PROFILE_DEV_DEBUG=0
set CARGO_PROFILE_TEST_DEBUG=0
set CARGO_BUILD_JOBS=2
cd /d "%~dp0"
call npm.cmd run lint
if errorlevel 1 exit /b %errorlevel%
call npm.cmd run build
if errorlevel 1 exit /b %errorlevel%
call npm.cmd run tauri:build
if errorlevel 1 exit /b %errorlevel%
copy /Y "%CARGO_TARGET_DIR%\release\creg-validator-desktop.exe" "%~dp0CREG-Validator-Console.exe" >nul
echo Built standalone executable: %~dp0CREG-Validator-Console.exe
