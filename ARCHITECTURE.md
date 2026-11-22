# InstantFTP architecture

**English** | [Español](ARCHITECTURE.es.md)

This document explains how the program is built, so that any person or AI can find
their way around in 5 minutes. Code comments are in English; documentation is in
English (main) and Spanish.

## File map

```
InstantFTP/
├── src/
│   ├── main.rs        Start-up: options, shared folder, log, port, accept() loop
│   ├── session.rs     One FTP connection: command reading, dispatch, transfers
│   ├── path.rs        Virtual paths and validation (the security barrier)
│   ├── args.rs        Command line options + MSVCRT-style command line splitting
│   ├── i18n.rs        Console language: English by default, Spanish
│   ├── listing.rs     LIST line format (`ls -l` style)
│   ├── timefmt.rs     Dates without floats or OS calls (FILETIME -> calendar)
│   ├── log.rs         Console + daily log logs\YYYY-MM-DD.log
│   ├── util.rs        Byte-string helpers (numbers, IPs, comparison)
│   └── sys/
│       ├── mod.rs     OS layer contract (function table)
│       ├── win32.rs   Real backend: Win32 + Winsock 2, no_std, Win98-compatible
│       └── hosted.rs  Test backend: Rust std (Linux / macOS / Windows)
├── tests/
│   ├── e2e.py         67 checks with a real FTP client (ftplib)
│   └── check_win98.py Checks the .exe's imports, PE subsystem and instructions
├── i586-win9x-windows-gnu.json   Rust target: i586 without SSE, soft-float
├── .cargo/config.toml            `cargo win` alias and linker flags
├── build.rs                      Chooses the `sys` backend
```

## Life of a connection

```
main.rs                         session.rs
───────                         ──────────
Socket::listen(21) ──accept()──▶ run()
  (one thread per client)          ├─ "220 InstantFTP ready"
                                   └─ loop: read_line() ─▶ dispatch()
                                                              ├─ USER/PASS/SYST/FEAT...
                                                              ├─ CWD/PWD/MKD/RMD/DELE/RNFR
                                                              │     └─ resolve() ─▶ path.rs
                                                              ├─ PASV/EPSV/PORT/EPRT
                                                              └─ LIST/RETR/STOR/APPE
                                                                    └─ open_data() ─▶ data connection
```

## The three key ideas

### 1. Virtual paths (security)

Each session stores its current directory as a **virtual** path (`/photos/2004`) and
never uses the process current directory. Every path sent by a client goes through
`Session::resolve` → `path::resolve`, which:

- normalizes `.` and `..` without ever going above `/`;
- rejects `:` (drive letters, alternate data streams), invalid characters, wildcards,
  device names (`CON`, `NUL`, `COM1`...) and dot/space-only components (`...`, `.. `);
- blocks the protected `logs` folder;
- only then maps it to a native path below the shared folder.

**Golden rule**: no new command may touch the disk with a client path without going
through `resolve`.

### 2. Swappable OS layer

`session.rs`, `main.rs` and `log.rs` only call `crate::sys::*`. There are two
implementations with the same API (see the table in `src/sys/mod.rs`):

- `win32.rs`: direct calls into `KERNEL32.dll` and `WS2_32.dll`, no C runtime. This is
  what ships in the .exe.
- `hosted.rs`: Rust `std`. Runs the complete server on Linux/macOS/Windows so it can be
  tested with real clients.

`build.rs` sets `cfg(win32)` when building for Windows; the code checks
`cfg(all(win32, not(test)))`, so `cargo test` always uses the `std` backend.

### 3. Windows 98 SE → 11 compatibility

| Problem | Solution |
|---|---|
| Modern Rust `std` requires Windows 7/10 | `#![no_std]` + `alloc`, hand-written Win32 |
| Pentium/PII/PIII have no SSE2 | custom `i586` target, soft-float ABI, no MMX/SSE |
| Win9x refuses images with subsystem > 4.0 | linker flags `--subsystem console:4.0` |
| Win9x has no Unicode (`...W`) APIs | ANSI `...A` functions only |
| `CreateThread` with a NULL thread id fails on Win9x | a pointer is always passed |
| `FILE_SHARE_DELETE` doesn't exist on Win9x | not used |
| `SetFilePointerEx` doesn't exist on Win9x | `SetFilePointer` with the high dword |
| No C runtime (`msvcrt` differs between versions) | allocator on `HeapAlloc`, own `mainCRTStartup` entry |
| Win9x Winsock dislikes huge `send()` calls | 32 KB chunks, `select()` timeouts, 64 KB buffers |

`tests/check_win98.py` enforces this in CI: if someone uses a function that doesn't
exist on Windows 98, the build fails.

## Language

Console and log texts go through `i18n::t("English", "Castellano")` or the
`tr!("en fmt", "es fmt", args)` macro. The language is chosen at start-up: `-lang`,
otherwise the Windows language (`GetUserDefaultLangID`), otherwise English. FTP replies
are never translated. Console texts are ASCII (the Win9x console uses an OEM code page).

## Executable size (~53 KB)

Achieved with `opt-level = "z"`, LTO, `panic = immediate-abort`, and above all
`function-sections` in the target, which lets the linker drop unused
`compiler_builtins` code. If a change makes the .exe grow a lot, it probably pulled in
`core::fmt` with new formats or floats: prefer concatenating bytes with `util::cat`.
