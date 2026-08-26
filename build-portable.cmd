@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-portable.ps1"
if errorlevel 1 exit /b %errorlevel%
endlocal
