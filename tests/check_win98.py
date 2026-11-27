#!/usr/bin/env python3
# Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
"""Verifies that InstantFTP.exe can load on Windows 98 SE:
  * PE32, console subsystem version 4.0
  * only imports from KERNEL32/WS2_32 that exist on Windows 98
  * no SSE / SSE2 / MMX / CMOV instructions (runs on a plain Pentium)
Needs i686-w64-mingw32-objdump (binutils-mingw-w64-i686).

    python3 tests/check_win98.py target/i586-win9x-windows-gnu/release/InstantFTP.exe
"""
import re, subprocess, sys

OBJDUMP = "i686-w64-mingw32-objdump"

# Functions exported by the Windows 98 SE kernel32.dll / ws2_32.dll that we allow.
WIN98 = {
    "KERNEL32.dll": set("""
        CloseHandle CreateDirectoryA CreateFileA CreateThread DeleteFileA EnterCriticalSection
        ExitProcess FileTimeToLocalFileTime FileTimeToSystemTime FindClose FindFirstFileA
        FindNextFileA FormatMessageA GetCommandLineA GetCurrentDirectoryA GetFileAttributesA
        GetFileSize GetFullPathNameA GetLastError GetLocalTime GetModuleFileNameA GetProcessHeap
        GetStdHandle GetSystemTime GetUserDefaultLangID GetSystemTimeAsFileTime GetTickCount HeapAlloc HeapFree
        HeapReAlloc InitializeCriticalSection DeleteCriticalSection LeaveCriticalSection MoveFileA
        MultiByteToWideChar ReadFile RemoveDirectoryA SetConsoleTitleA SetEndOfFile
        SetFileAttributesA SetFilePointer SetLastError Sleep WideCharToMultiByte WriteFile
    """.split()),
    "WS2_32.dll": set("""
        WSAStartup WSACleanup WSAGetLastError accept bind closesocket connect gethostbyname
        gethostname getpeername getsockname listen recv select send setsockopt shutdown socket
    """.split()),
}


def main(exe):
    p = subprocess.run([OBJDUMP, "-p", exe], capture_output=True, text=True, check=True).stdout
    errors = []
    if "PE32" not in p or "(PE32+)" in p:
        errors.append("not a 32-bit PE32 image")
    for key in ("MajorSubsystemVersion", "MajorOSystemVersion"):
        m = re.search(key + r"\s+(\d+)", p)
        if not m or int(m.group(1)) > 4:
            errors.append(f"{key} must be 4 (Windows 9x refuses newer images)")
    dll, count = None, 0
    for line in p.splitlines():
        m = re.match(r"\s*DLL Name: (\S+)", line)
        if m:
            dll = m.group(1)
            if dll.upper() not in {d.upper() for d in WIN98}:
                errors.append(f"imports from {dll}, not allowed")
            continue
        m = re.match(r"^\t[0-9a-f]+\t +\d+ +(\S+)$", line)
        if m and dll:
            count += 1
            allowed = next((v for k, v in WIN98.items() if k.upper() == dll.upper()), set())
            if m.group(1) not in allowed:
                errors.append(f"{dll}!{m.group(1)} is not available on Windows 98")
    d = subprocess.run([OBJDUMP, "-d", exe], capture_output=True, text=True, check=True).stdout
    bad = [l for l in d.splitlines() if re.search(r"%xmm|%mm[0-7]|\bcmov", l)]
    if bad:
        errors.append(f"{len(bad)} SSE/MMX/CMOV instructions, e.g. {bad[0].strip()}")
    if errors:
        print("FAIL:\n  " + "\n  ".join(errors))
        sys.exit(1)
    print(f"OK: PE32 subsystem 4.0, {count} imports all present in Windows 98, no SSE/MMX/CMOV")


if __name__ == "__main__":
    main(sys.argv[1])
