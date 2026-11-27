@echo off
rem Copyright (C) 2026 Matias Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
rem Compila InstantFTP.exe (Windows 98SE .. 11).
rem Requiere rustup y MinGW 32 bits (MSYS2: pacman -S mingw-w64-i686-gcc) en el PATH.
cd /d "%~dp0"
set RUSTC_BOOTSTRAP=1
cargo win %*
if errorlevel 1 exit /b 1
echo OK -^> target\i586-win9x-windows-gnu\release\InstantFTP.exe
