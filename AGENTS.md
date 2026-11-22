# Instructions for AI coding agents

This file is for AI assistants (Claude, Copilot, Codex, Cursor, ...) working on
this repository.

## What this project is

InstantFTP: a tiny (~53 KB) portable FTP server for Windows 98 SE through 11.
Double-clicking `InstantFTP.exe` shares the folder it lives in. Author: matcarfer.
**Closed source**: this repository is private. The .exe is freeware (see LICENSE).
Every source file starts with a copyright line; keep it. Never mention other
programs as inspiration or origin, in code, docs or the .exe.

## Read first

1. `ARCHITECTURE.md` — file map, connection flow, the three key ideas.
2. `src/sys/mod.rs` — the OS abstraction contract.
3. `src/session.rs` header — rules for FTP commands.

## Hard constraints (never break these)

- The Windows build is `#![no_std]` + `alloc` on a custom i586 soft-float target.
  Do not add crates, do not use `std` outside `src/sys/hosted.rs`, do not use
  floats, SIMD or Unicode-only (`...W`) Win32 functions.
- Only Win32/Winsock functions that exist on Windows 98 may be imported.
  `tests/check_win98.py` lists the allowed ones; extend that list only after
  verifying the function exists in Windows 98's `kernel32.dll` / `ws2_32.dll`.
- Every client-supplied path must go through `Session::resolve` (or
  `path::resolve*` + `is_protected`). Never join client input onto a native path.
- Every write operation checks `Session::read_only` first.
- Each FTP command sends exactly one final reply.
- File names are bytes (`&[u8]`, ANSI on Windows), not `str`.
- New OS functionality: implement in BOTH `win32.rs` and `hosted.rs`, add it to
  the table in `sys/mod.rs`.

## Workflow

```sh
cargo fmt
cargo clippy -- -D warnings
cargo test
cargo build && python3 tests/e2e.py target/debug/InstantFTP
sh build.sh && python3 tests/check_win98.py target/i586-win9x-windows-gnu/release/InstantFTP.exe
```

All five must pass. Add a unit test or an e2e case for every fix or feature.
Report the .exe size before/after if it changes by more than 1 KB.

## Versions and releases

Any change to the program must bump `version` in Cargo.toml (new feature or
visible change: minor, 1.2.0 -> 1.3.0; fix only: patch, 1.2.0 -> 1.2.1).
Docs/tests-only changes don't.

CI builds `InstantFTP.exe` as an artifact; it does NOT publish releases here.
Releases go to the public repo **matcarfer/InstantFTP** (exe only, no source):
download the CI artifact, rename it `InstantFTP-v<major>.<minor>.exe` (hyphen, never underscore) (keep the
patch number if not 0), then `gh release create v<version> <exe> --repo
matcarfer/InstantFTP` with bilingual notes (EN then ES) including the SHA-256,
update the public README's download line and the row in matcarfer/library.

## Style

- Code comments in English.
- Console/log text is bilingual: always `i18n::t("English", "Castellano")` or
  `tr!("en fmt", "es fmt", args)`, never a single-language literal. ASCII only in
  console strings (the Win9x console uses an OEM code page). FTP replies stay English.
- ARCHITECTURE exists in English and Spanish (`.es.md`); update both together.
- `///` doc comment on every function and type, explaining what and why.
- Existing FTP reply texts stay identical (client compatibility).

## Useful facts

- `cfg(all(win32, not(test)))` = real Windows build. `build.rs` sets `win32`.
- Local builds need `RUSTC_BOOTSTRAP=1` (done by `build.sh` / `build.bat`)
  because `-Zbuild-std` compiles `core` for the custom target.
- `cargo test` and the e2e tests run on the `std` backend on any OS.
