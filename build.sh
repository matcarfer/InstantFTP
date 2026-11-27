#!/bin/sh
# Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
# Builds InstantFTP.exe (Windows 98SE .. 11). Needs: rustup + i686-w64-mingw32-gcc
set -e
cd "$(dirname "$0")"
RUSTC_BOOTSTRAP=1 cargo win "$@"
echo "OK -> target/i586-win9x-windows-gnu/release/InstantFTP.exe"
