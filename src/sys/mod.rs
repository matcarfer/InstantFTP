// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! # Operating system layer (the only place that talks to the OS)
//!
//! The FTP logic (`session.rs`, `main.rs`, `log.rs`) never calls Windows or
//! `std` directly; it calls the functions re-exported here. Two backends
//! implement exactly the same API:
//!
//! | backend        | used for                         | built with                    |
//! |----------------|----------------------------------|-------------------------------|
//! | `win32.rs`     | the shipped `InstantFTP.exe`     | `no_std`, raw Win32/Winsock 2 |
//! | `hosted.rs`    | `cargo test`, `tests/e2e.py`     | Rust `std` (Linux/macOS)      |
//!
//! `build.rs` + `cfg(all(win32, not(test)))` select the backend. This lets the complete server run, and
//! be tested with real FTP clients, on any development machine, while the
//! Windows build stays tiny and compatible with Windows 98.
//!
//! ## API contract (both backends must provide all of this)
//!
//! | item | meaning |
//! |------|---------|
//! | `SEP` | native path separator (`\` or `/`) |
//! | `out(bytes)` | write to the console |
//! | `args() -> Vec<Vec<u8>>` | command line arguments, without the program name |
//! | `exe_dir()` | folder of the running executable |
//! | `full_path(p)` | absolute normalized path, no trailing separator except drive roots |
//! | `now_utc_ft()`, `now_local_ft()`, `ft_to_local(ft)` | times as FILETIME (100 ns since 1601) |
//! | `ticks_ms()` | monotonic-ish milliseconds, only for durations |
//! | `sleep_ms(ms)`, `spawn(closure) -> bool`, `set_title(bytes)` | threads / console |
//! | `system_spanish()` | Windows / locale language is Spanish (UI language default) |
//! | `Lock::new()`, `lock.with(\|\| ..)` | mutual exclusion (CRITICAL_SECTION / Mutex) |
//! | `metadata(p)`, `read_dir(p)` | file info, folder listing (no `.`/`..`) |
//! | `delete_file`, `create_dir`, `remove_dir`, `rename`, `set_hidden` | file system changes |
//! | `File::open_read`, `File::open_write(p, WriteMode)`, `read`, `write_all`, `seek` | file I/O |
//! | `utf8_to_local`, `local_to_utf8` | file name encoding for `OPTS UTF8` clients |
//! | `net_init()`, `host_ips()` | network start-up, local IPv4 addresses |
//! | `Socket::listen/accept/connect/recv/send_all/local_addr/peer_addr/tune_bulk/drain_pending` | TCP |
//!
//! Errors are Windows error codes (`Error = u32`) on both backends, so the
//! FTP replies are identical everywhere. When adding a function, add it to
//! **both** backends and to this table.

#[cfg(all(win32, not(test)))]
mod win32;
#[cfg(all(win32, not(test)))]
pub use win32::*;

#[cfg(not(all(win32, not(test))))]
mod hosted;
#[cfg(not(all(win32, not(test))))]
pub use hosted::*;

/// Windows style error code (`GetLastError` / `WSAGetLastError`).
/// The test backend maps `std::io::Error` onto these same numbers.
pub type Error = u32;
/// ERROR_FILE_NOT_FOUND
pub const E_NOT_FOUND: Error = 2;
/// ERROR_PATH_NOT_FOUND
pub const E_PATH_NOT_FOUND: Error = 3;
/// ERROR_ACCESS_DENIED (also returned when opening a folder as a file)
pub const E_ACCESS: Error = 5;
/// ERROR_SHARING_VIOLATION: another program has the file open
pub const E_SHARING: Error = 32;
/// ERROR_FILE_EXISTS
pub const E_FILE_EXISTS: Error = 80;
/// ERROR_DISK_FULL (Windows 9x may report 39, ERROR_HANDLE_DISK_FULL)
pub const E_DISK_FULL: Error = 112;
/// ERROR_INVALID_NAME
pub const E_BAD_NAME: Error = 123;
/// ERROR_DIR_NOT_EMPTY
pub const E_DIR_NOT_EMPTY: Error = 145;
/// ERROR_ALREADY_EXISTS
pub const E_EXISTS: Error = 183;
/// WSAETIMEDOUT, also used for our own `select()` timeouts
pub const E_TIMEOUT: Error = 10060;

/// Information about a file or folder (the subset of `stat` we need).
#[derive(Debug, Clone, Copy, Default)]
pub struct Meta {
    pub is_dir: bool,
    pub readonly: bool,
    /// Hidden or system attribute (on the test backend: name starts with '.').
    pub hidden: bool,
    /// Size in bytes (0 for folders). 64-bit: files > 4 GB are fine.
    pub size: u64,
    /// Last write time, UTC FILETIME.
    pub mtime: u64,
}

/// IPv4 address + port (Windows 98 has no IPv6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockAddr {
    pub ip: [u8; 4],
    pub port: u16,
}

/// How `File::open_write` opens a file.
#[derive(Debug, Clone, Copy)]
pub enum WriteMode {
    /// STOR: create or overwrite.
    Truncate,
    /// APPE: create or append at the end.
    Append,
    /// STOR after `REST n`: keep the first n bytes, cut the rest, write from there.
    At(u64),
}

/// Human readable text for an error code, used in "550 <text>" replies.
/// Not `sys_errlist[errno]`, which is often stale because
/// Win32 calls don't set `errno`.
pub fn error_text(e: Error) -> &'static str {
    match e {
        E_NOT_FOUND | E_PATH_NOT_FOUND | 15 | 53 | 67 => "No such file or directory",
        E_ACCESS | 19 => "Permission denied",
        E_SHARING | 33 => "File is in use by another program",
        E_FILE_EXISTS | E_EXISTS => "File exists",
        E_DISK_FULL | 39 => "No space left on device",
        E_BAD_NAME | 161 => "Invalid file name",
        E_DIR_NOT_EMPTY => "Directory not empty",
        206 => "File name too long",
        21 => "Device not ready",
        E_TIMEOUT => "Connection timed out",
        10061 => "Connection refused",
        10054 | 10053 => "Connection reset",
        _ => "Input/output error",
    }
}
