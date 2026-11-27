#!/usr/bin/env python3
# Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
"""Verifies that InstantFTP.exe can load on Windows 98 SE:
  * PE32, console subsystem version 4.0
  * only imports from KERNEL32/WS2_32 that exist on Windows 98
  * no SSE / SSE2 / MMX / CMOV instructions (runs on a plain Pentium)
Needs i686-w64-mingw32-objdump (binutils-mingw-w64-i686).

    python3 tests/check_win98.py target/i586-win9x-windows-gnu/release/InstantFTP.exe
"""
import re, struct, subprocess, sys

OBJDUMP = "i686-w64-mingw32-objdump"

# Functions exported by the Windows 98 SE kernel32.dll / ws2_32.dll that we allow.
WIN98 = {
    "KERNEL32.dll": set("""
        CloseHandle CreateDirectoryA CreateFileA CreateThread DeleteFileA EnterCriticalSection
        ExitProcess FileTimeToLocalFileTime FileTimeToSystemTime FindClose FindFirstFileA
        FindNextFileA FormatMessageA GetCommandLineA GetCurrentDirectoryA GetFileAttributesA
        GetFileSize GetFullPathNameA GetLastError GetLocalTime GetModuleFileNameA GetProcessHeap
        GetStdHandle GetSystemTime GetUserDefaultLangID GetConsoleScreenBufferInfo SetConsoleTextAttribute GetSystemTimeAsFileTime GetTickCount HeapAlloc HeapFree
        HeapReAlloc InitializeCriticalSection DeleteCriticalSection LeaveCriticalSection MoveFileA
        MultiByteToWideChar ReadFile RemoveDirectoryA SetConsoleTitleA SetEndOfFile
        SetFileAttributesA SetFilePointer SetLastError Sleep WideCharToMultiByte WriteFile
    """.split()),
    "WS2_32.dll": set("""
        WSAStartup WSACleanup WSAGetLastError accept bind closesocket connect gethostbyname
        gethostname getpeername getsockname listen recv select send setsockopt shutdown socket
    """.split()),
}


def pe_imports(exe):
    """Returns [(dll, [function names])] read from the PE32 import table."""
    d = open(exe, "rb").read()
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    opt = pe + 24
    optsz = struct.unpack_from("<H", d, pe + 20)[0]
    imp_rva = struct.unpack_from("<I", d, opt + 104)[0]
    secs = []
    for i in range(nsec):
        vsize, va, rsize, raw = struct.unpack_from("<IIII", d, opt + optsz + 40 * i + 8)
        secs.append((va, max(vsize, rsize), raw))

    def off(rva):
        for va, size, raw in secs:
            if va <= rva < va + size:
                return rva - va + raw
        raise ValueError(f"RVA {rva:#x} outside every section")

    def cstr(o):
        return d[o:d.index(b"\0", o)].decode("ascii")

    result, o = [], off(imp_rva)
    while True:
        ilt, _, _, name, iat = struct.unpack_from("<IIIII", d, o)
        if name == 0:
            return result
        funcs, t = [], off(ilt or iat)
        while True:
            e = struct.unpack_from("<I", d, t)[0]
            if e == 0:
                break
            funcs.append(f"#{e & 0xFFFF}" if e & 0x80000000 else cstr(off(e) + 2))
            t += 4
        result.append((cstr(off(name)), funcs))
        o += 20


def main(exe):
    p = subprocess.run([OBJDUMP, "-p", exe], capture_output=True, text=True, check=True).stdout
    errors = []
    if "PE32" not in p or "(PE32+)" in p:
        errors.append("not a 32-bit PE32 image")
    for key in ("MajorSubsystemVersion", "MajorOSystemVersion"):
        m = re.search(key + r"\s+(\d+)", p)
        if not m or int(m.group(1)) > 4:
            errors.append(f"{key} must be 4 (Windows 9x refuses newer images)")
    # Imports are read from the PE import table itself (objdump's text format
    # changes between versions and silently matched nothing before).
    count = 0
    for dll, funcs in pe_imports(exe):
        allowed = next((v for k, v in WIN98.items() if k.upper() == dll.upper()), None)
        if allowed is None:
            errors.append(f"imports from {dll}, not allowed")
            continue
        for f in funcs:
            count += 1
            if f not in allowed:
                errors.append(f"{dll}!{f} is not available on Windows 98")
    if count == 0:
        errors.append("no imports found: the import table could not be read")
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
