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
npm.cmd run tauri:dev
if errorlevel 1 (
  echo.
  echo Tauri dev exited with error %errorlevel%.
  pause
)
