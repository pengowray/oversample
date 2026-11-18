@echo off
rem Runs desktop-run.ps1 without execution-policy trouble. Same options:
rem   desktop-run.cmd [-Release ^| -Dev ^| -NoBuild ^| -Help] [app args...]
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0desktop-run.ps1" %*
exit /b %ERRORLEVEL%
