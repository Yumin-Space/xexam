@echo off
rem CCARP quiz launcher WITHOUT Docker. Double-click this file. Keep this window open while you study.
rem (With Docker, use:  docker compose up -d --build  )
cd /d "%~dp0"
chcp 65001 >nul
title CCARP quiz

where python >nul 2>nul
if errorlevel 1 (
  echo.
  echo [!] Python was not found on this computer.
  echo     Install Python 3.8 or newer from https://www.python.org/downloads/
  echo     and tick "Add python.exe to PATH" during setup, then double-click this file again.
  echo.
  pause
  exit /b 1
)

python server.py --allow-shutdown %*
if errorlevel 1 (
  echo.
  echo [!] The program stopped with an error. Please read the message above.
  pause
)
