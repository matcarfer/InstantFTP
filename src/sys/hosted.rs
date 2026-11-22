// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! # Test backend built on Rust `std`
//!
//! Used only to run and test the server on a development machine (Linux,
//! macOS, or Windows via `cargo test` / `--features hosted`). The shipped
//! Windows .exe uses `win32.rs`.
//!
//! Every function mirrors the one with the same name in `win32.rs`; see the
//! contract table in `sys/mod.rs`. Differences, on purpose:
//! * "hidden" means the name starts with a dot;
//! * local time is treated as UTC;
//! * `set_hidden` and `set_title` do nothing.

use super::{Error, Meta, SockAddr, WriteMode, E_ACCESS, E_EXISTS, E_NOT_FOUND, E_TIMEOUT};
use std::boxed::Box;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::vec::Vec;

pub const SEP: u8 = if cfg!(windows) { b'\\' } else { b'/' };

/// Byte path -> `Path` (Unix paths are arbitrary bytes).
fn p(b: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::Path::new(std::ffi::OsStr::from_bytes(b)).to_path_buf()
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(b).into_owned())
    }
}

/// `OsStr` -> bytes (lossy outside Unix; fine for tests).
fn b(s: &std::ffi::OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned().into_bytes()
    }
}

/// Maps `std::io::Error` to the Windows codes used by `error_text`.
fn io_err(e: std::io::Error) -> Error {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => E_NOT_FOUND,
        PermissionDenied => E_ACCESS,
        AlreadyExists => E_EXISTS,
        TimedOut | WouldBlock => E_TIMEOUT,
        DirectoryNotEmpty => super::E_DIR_NOT_EMPTY,
        StorageFull => super::E_DISK_FULL,
        _ => e.raw_os_error().map(|c| 20000 + c as u32).unwrap_or(1),
    }
}

pub fn out(s: &[u8]) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s);
    let _ = o.flush();
}

pub fn args() -> Vec<Vec<u8>> {
    std::env::args_os().skip(1).map(|a| b(&a)).collect()
}

pub fn exe_dir() -> Vec<u8> {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| b(d.as_os_str()))).unwrap_or_else(|| b".".to_vec())
}

pub fn full_path(path: &[u8]) -> Option<Vec<u8>> {
    std::fs::canonicalize(p(path)).ok().map(|c| b(c.as_os_str()))
}

/// `SystemTime` -> FILETIME (100 ns since 1601).
fn st_to_ft(t: SystemTime) -> u64 {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() + crate::timefmt::EPOCH_DIFF_SECS) * 10_000_000 + (d.subsec_nanos() / 100) as u64
}

pub fn now_utc_ft() -> u64 {
    st_to_ft(SystemTime::now())
}
/// The test backend treats local time as UTC.
pub fn now_local_ft() -> u64 {
    now_utc_ft()
}
pub fn ft_to_local(ft: u64) -> u64 {
    ft
}

/// True if the environment locale is Spanish (`LANG=es_AR.UTF-8`, ...).
pub fn system_spanish() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .is_some_and(|v| v.starts_with("es"))
}

pub fn ticks_ms() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

pub fn sleep_ms(ms: u32) {
    std::thread::sleep(Duration::from_millis(ms as u64));
}

pub fn spawn(f: Box<dyn FnOnce() + Send>) -> bool {
    std::thread::Builder::new().spawn(f).is_ok()
}

pub fn set_title(_t: &[u8]) {}

/// Mutual exclusion; `with` runs a closure while holding the lock.
pub struct Lock(std::sync::Mutex<()>);
impl Lock {
    pub fn new() -> Lock {
        Lock(std::sync::Mutex::new(()))
    }
    pub fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        let _g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }
}

/// `std` metadata -> portable `Meta`.
fn meta_of(name: &[u8], m: &std::fs::Metadata) -> Meta {
    Meta {
        is_dir: m.is_dir(),
        readonly: m.permissions().readonly(),
        hidden: name.first() == Some(&b'.'),
        size: if m.is_dir() { 0 } else { m.len() },
        mtime: m.modified().map(st_to_ft).unwrap_or(0),
    }
}

pub fn metadata(path: &[u8]) -> Result<Meta, Error> {
    let m = std::fs::metadata(p(path)).map_err(io_err)?;
    let name = path.rsplit(|&c| c == SEP).next().unwrap_or(b"");
    Ok(meta_of(name, &m))
}

pub fn read_dir(dir: &[u8]) -> Result<Vec<(Vec<u8>, Meta)>, Error> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(p(dir)).map_err(io_err)? {
        let e = e.map_err(io_err)?;
        let name = b(&e.file_name());
        if let Ok(m) = std::fs::metadata(e.path()) {
            let meta = meta_of(&name, &m);
            v.push((name, meta));
        }
    }
    Ok(v)
}

pub fn delete_file(path: &[u8]) -> Result<(), Error> {
    std::fs::remove_file(p(path)).map_err(io_err)
}
pub fn create_dir(path: &[u8]) -> Result<(), Error> {
    std::fs::create_dir(p(path)).map_err(io_err)
}
pub fn remove_dir(path: &[u8]) -> Result<(), Error> {
    std::fs::remove_dir(p(path)).map_err(io_err)
}
pub fn rename(from: &[u8], to: &[u8]) -> Result<(), Error> {
    if std::fs::symlink_metadata(p(to)).is_ok() {
        return Err(E_EXISTS); // same semantics as MoveFileA
    }
    std::fs::rename(p(from), p(to)).map_err(io_err)
}
pub fn set_hidden(_path: &[u8]) {}

/// Open file, closed on drop.
pub struct File(std::fs::File);
impl File {
    pub fn open_read(path: &[u8]) -> Result<File, Error> {
        if std::fs::metadata(p(path)).map_err(io_err)?.is_dir() {
            return Err(E_ACCESS);
        }
        std::fs::File::open(p(path)).map(File).map_err(io_err)
    }
    pub fn open_write(path: &[u8], mode: WriteMode) -> Result<File, Error> {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true);
        match mode {
            WriteMode::Truncate => {
                o.truncate(true);
            }
            WriteMode::Append => {
                o.append(true);
            }
            WriteMode::At(_) => {}
        }
        let mut f = o.open(p(path)).map_err(io_err)?;
        if let WriteMode::At(off) = mode {
            f.set_len(off).map_err(io_err)?;
            f.seek(SeekFrom::Start(off)).map_err(io_err)?;
        }
        Ok(File(f))
    }
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        self.0.read(buf).map_err(io_err)
    }
    pub fn write_all(&mut self, buf: &[u8]) -> Result<(), Error> {
        self.0.write_all(buf).map_err(io_err)
    }
    pub fn seek(&mut self, pos: u64) -> Result<(), Error> {
        self.0.seek(SeekFrom::Start(pos)).map(|_| ()).map_err(io_err)
    }
}

pub fn utf8_to_local(b: &[u8]) -> Vec<u8> {
    b.to_vec()
}
pub fn local_to_utf8(b: &[u8]) -> Vec<u8> {
    b.to_vec()
}

// ---------------------------------------------------------------- network

pub fn net_init() -> bool {
    true
}

enum Inner {
    L(TcpListener),
    S(TcpStream),
}
/// TCP socket (listener or stream), closed on drop.
pub struct Socket(Inner);

/// `std` address -> portable IPv4 address (IPv6 is ignored).
fn sa(a: std::net::SocketAddr) -> Option<SockAddr> {
    match a {
        std::net::SocketAddr::V4(v) => Some(SockAddr { ip: v.ip().octets(), port: v.port() }),
        _ => None,
    }
}

impl Socket {
    pub fn listen(ip: [u8; 4], port: u16, _backlog: i32) -> Result<Socket, Error> {
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::from(ip), port)).map(|l| Socket(Inner::L(l))).map_err(io_err)
    }
    pub fn accept(&self, timeout_ms: u32) -> Result<(Socket, SockAddr), Error> {
        let Inner::L(l) = &self.0 else { return Err(E_ACCESS) };
        l.set_nonblocking(true).map_err(io_err)?;
        let start = Instant::now();
        loop {
            match l.accept() {
                Ok((s, a)) => {
                    s.set_nonblocking(false).map_err(io_err)?;
                    let _ = s.set_nodelay(true);
                    return Ok((Socket(Inner::S(s)), sa(a).unwrap_or(SockAddr { ip: [0; 4], port: 0 })));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if timeout_ms != 0 && start.elapsed() >= Duration::from_millis(timeout_ms as u64) {
                        return Err(E_TIMEOUT);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(io_err(e)),
            }
        }
    }
    pub fn drain_pending(&self) {
        if let Inner::L(l) = &self.0 {
            let _ = l.set_nonblocking(true);
            while l.accept().is_ok() {}
        }
    }
    pub fn connect(addr: SockAddr) -> Result<Socket, Error> {
        TcpStream::connect_timeout(&SocketAddrV4::new(Ipv4Addr::from(addr.ip), addr.port).into(), Duration::from_secs(30))
            .map(|s| Socket(Inner::S(s)))
            .map_err(io_err)
    }
    pub fn local_addr(&self) -> Option<SockAddr> {
        match &self.0 {
            Inner::L(l) => l.local_addr().ok().and_then(sa),
            Inner::S(s) => s.local_addr().ok().and_then(sa),
        }
    }
    #[allow(dead_code)]
    pub fn peer_addr(&self) -> Option<SockAddr> {
        match &self.0 {
            Inner::S(s) => s.peer_addr().ok().and_then(sa),
            _ => None,
        }
    }
    pub fn send_all(&self, buf: &[u8]) -> Result<(), Error> {
        let Inner::S(s) = &self.0 else { return Err(E_ACCESS) };
        // Same 120 s "client stopped reading" limit as the Win32 backend.
        let _ = s.set_write_timeout(Some(Duration::from_secs(120)));
        (&*s).write_all(buf).map_err(io_err)
    }
    pub fn recv(&self, buf: &mut [u8], timeout_ms: u32) -> Result<usize, Error> {
        let Inner::S(s) = &self.0 else { return Err(E_ACCESS) };
        let _ = s.set_read_timeout(if timeout_ms == 0 { None } else { Some(Duration::from_millis(timeout_ms as u64)) });
        (&*s).read(buf).map_err(io_err)
    }
    pub fn tune_bulk(&self) {}
}

/// Primary local IPv4 address.
pub fn host_ips() -> Vec<[u8; 4]> {
    // Route-based trick: no packet is sent.
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|u| {
            u.connect("192.0.2.1:9")?;
            u.local_addr()
        })
        .ok()
        .and_then(sa)
        .map(|a| std::vec![a.ip])
        .unwrap_or_default()
}
