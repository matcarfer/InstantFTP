# InstantFTP (private source)

Private source code of **InstantFTP**, a ~56 KB portable FTP server for
Windows 98 SE through 11, written in Rust (`#![no_std]`). Closed source, not
for distribution. Public download page (exe only):
https://github.com/matcarfer/InstantFTP

- Architecture: [ARCHITECTURE.md](ARCHITECTURE.md) · [ARCHITECTURE.es.md](ARCHITECTURE.es.md)
- Rules for AI agents, build, tests and release steps: [AGENTS.md](AGENTS.md)
- License of the .exe (freeware EULA): [LICENSE](LICENSE)

## Build

```sh
sudo apt install gcc-mingw-w64-i686   # Linux / WSL
sh build.sh                           # -> target/i586-win9x-windows-gnu/release/InstantFTP.exe
```

On Windows: MSYS2 with `mingw-w64-i686-gcc` in PATH, then `build.bat`.
CI (GitHub Actions) runs the tests and uploads the .exe as an artifact.

## Tests

```sh
cargo test
cargo build && python3 tests/e2e.py target/debug/InstantFTP
```

Copyright (c) 2026 matcarfer. All rights reserved.
