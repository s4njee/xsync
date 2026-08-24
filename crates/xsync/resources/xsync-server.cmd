@echo off
setlocal

rem Use the first argument as the server root, or keep data beside the binary.
set "ROOT=%~1"
if not defined ROOT set "ROOT=%~dp0data"
if not exist "%ROOT%" mkdir "%ROOT%"

"%~dp0xsync.exe" --server "%ROOT%"
exit /b %ERRORLEVEL%
