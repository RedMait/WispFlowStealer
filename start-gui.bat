@echo off
rem Double-click to start flowvoice GUI. Runs start-gui.ps1 next to this file.
powershell -ExecutionPolicy Bypass -NoProfile -File "%~dp0start-gui.ps1"
if errorlevel 1 pause
