@echo off
rem Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
rem Compila InstantFTP.exe (Windows 98SE .. 11).
rem Requiere rustup y MinGW 32 bits (MSYS2: pacman -S mingw-w64-i686-gcc) en el PATH.
cd /d "%~dp0"
set RUSTC_BOOTSTRAP=1
cargo win %*
if errorlevel 1 exit /b 1
echo OK -^> target\i586-win9x-windows-gnu\release\InstantFTP.exe
