#!/bin/sh
# Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
# Builds InstantFTP.exe (Windows 98SE .. 11). Needs: rustup + i686-w64-mingw32-gcc
set -e
cd "$(dirname "$0")"
RUSTC_BOOTSTRAP=1 cargo win "$@"
echo "OK -> target/i586-win9x-windows-gnu/release/InstantFTP.exe"
