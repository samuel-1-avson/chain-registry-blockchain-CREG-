@echo off
REM ==============================================================================
REM Chain Registry (CREG) -- External Validator Launcher (Windows CMD / Bypass Wrapper)
REM Automatically bypasses PowerShell ExecutionPolicy restrictions
REM ==============================================================================
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-external-validator.ps1" %*
