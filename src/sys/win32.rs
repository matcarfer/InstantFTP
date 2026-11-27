// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! Raw Win32 / Winsock 2 backend.
//!
//! Compatibility rules followed here so the same .exe runs on Windows 98SE..11:
//! * only ANSI (`...A`) functions and APIs that exist in Windows 95/98;
//! * `CreateThread` always gets a thread-id pointer (Win9x fails with NULL);
//! * `CreateFileA` never uses `FILE_SHARE_DELETE` (invalid on Win9x);
//! * 64-bit file offsets via `SetFilePointer` high dword (no `SetFilePointerEx`);
//! * timeouts via `select()` (works on every Winsock 2 stack);
//! * no C runtime at all: no msvcrt version issues, no VC++ redistributables.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use super::{Error, Meta, SockAddr, WriteMode, E_TIMEOUT};
use alloc::{boxed::Box, vec, vec::Vec};
use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr::{null, null_mut};

/// Native path separator.
pub const SEP: u8 = b'\\';

type HANDLE = *mut c_void;
type SOCKET = usize;
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;
const INVALID_SOCKET: SOCKET = !0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FILETIME {
    lo: u32,
    hi: u32,
}

#[repr(C)]
struct FIND_DATA {
    attrs: u32,
    created: FILETIME,
    accessed: FILETIME,
    written: FILETIME,
    size_hi: u32,
    size_lo: u32,
    reserved0: u32,
    reserved1: u32,
    name: [u8; 260],
    alt_name: [u8; 14],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SOCKADDR_IN {
    family: u16,
    port: u16, // network byte order
    addr: [u8; 4],
    zero: [u8; 8],
}

#[repr(C)]
struct FD_SET {
    count: u32,
    sockets: [SOCKET; 64],
}

#[repr(C)]
struct TIMEVAL {
    sec: i32,
    usec: i32,
}

#[repr(C)]
struct HOSTENT {
    name: *mut u8,
    aliases: *mut *mut u8,
    addrtype: i16,
    length: i16,
    addr_list: *mut *mut u8,
}

type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn GetStdHandle(n: u32) -> HANDLE;
    fn WriteFile(h: HANDLE, b: *const u8, n: u32, w: *mut u32, o: *mut c_void) -> i32;
    fn ReadFile(h: HANDLE, b: *mut u8, n: u32, r: *mut u32, o: *mut c_void) -> i32;
    fn CreateFileA(name: *const u8, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> i32;
    fn SetFilePointer(h: HANDLE, lo: i32, hi: *mut i32, method: u32) -> u32;
    fn SetEndOfFile(h: HANDLE) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn ExitProcess(code: u32) -> !;
    fn GetCommandLineA() -> *const u8;
    fn GetProcessHeap() -> HANDLE;
    fn HeapAlloc(h: HANDLE, flags: u32, n: usize) -> *mut c_void;
    fn HeapReAlloc(h: HANDLE, flags: u32, p: *mut c_void, n: usize) -> *mut c_void;
    fn HeapFree(h: HANDLE, flags: u32, p: *mut c_void) -> i32;
    fn CreateThread(sa: *mut c_void, stack: usize, start: ThreadProc, param: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    fn InitializeCriticalSection(cs: *mut c_void);
    fn EnterCriticalSection(cs: *mut c_void);
    fn LeaveCriticalSection(cs: *mut c_void);
    fn FindFirstFileA(name: *const u8, fd: *mut FIND_DATA) -> HANDLE;
    fn FindNextFileA(h: HANDLE, fd: *mut FIND_DATA) -> i32;
    fn FindClose(h: HANDLE) -> i32;
    fn GetFileAttributesA(name: *const u8) -> u32;
    fn SetFileAttributesA(name: *const u8, attrs: u32) -> i32;
    fn CreateDirectoryA(name: *const u8, sa: *mut c_void) -> i32;
    fn RemoveDirectoryA(name: *const u8) -> i32;
    fn DeleteFileA(name: *const u8) -> i32;
    fn MoveFileA(from: *const u8, to: *const u8) -> i32;
    fn GetFullPathNameA(name: *const u8, n: u32, buf: *mut u8, file: *mut *mut u8) -> u32;
    fn GetModuleFileNameA(m: HANDLE, buf: *mut u8, n: u32) -> u32;
    fn GetSystemTimeAsFileTime(ft: *mut FILETIME);
    fn FileTimeToLocalFileTime(utc: *const FILETIME, local: *mut FILETIME) -> i32;
    fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    fn WideCharToMultiByte(cp: u32, flags: u32, w: *const u16, wn: i32, s: *mut u8, n: i32, def: *const u8, used: *mut i32) -> i32;
    fn Sleep(ms: u32);
    fn GetTickCount() -> u32;
    fn GetUserDefaultLangID() -> u16;
    fn SetConsoleTitleA(t: *const u8) -> i32;
}

#[link(name = "ws2_32")]
extern "system" {
    fn WSAStartup(ver: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, proto: i32) -> SOCKET;
    fn bind(s: SOCKET, a: *const SOCKADDR_IN, len: i32) -> i32;
    fn listen(s: SOCKET, backlog: i32) -> i32;
    fn accept(s: SOCKET, a: *mut SOCKADDR_IN, len: *mut i32) -> SOCKET;
    fn connect(s: SOCKET, a: *const SOCKADDR_IN, len: i32) -> i32;
    fn send(s: SOCKET, b: *const u8, n: i32, flags: i32) -> i32;
    fn recv(s: SOCKET, b: *mut u8, n: i32, flags: i32) -> i32;
    fn closesocket(s: SOCKET) -> i32;
    fn getsockname(s: SOCKET, a: *mut SOCKADDR_IN, len: *mut i32) -> i32;
    fn getpeername(s: SOCKET, a: *mut SOCKADDR_IN, len: *mut i32) -> i32;
    fn setsockopt(s: SOCKET, level: i32, name: i32, v: *const u8, len: i32) -> i32;
    fn select(n: i32, r: *mut FD_SET, w: *mut FD_SET, e: *mut FD_SET, t: *const TIMEVAL) -> i32;
    fn gethostname(name: *mut u8, len: i32) -> i32;
    fn gethostbyname(name: *const u8) -> *mut HOSTENT;
}

// ------------------------------------------------------------------ runtime

struct ProcessHeap;

unsafe impl GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.align() <= 8 {
            return HeapAlloc(GetProcessHeap(), 0, l.size()) as *mut u8;
        }
        // Over-aligned: keep the original pointer just before the aligned block.
        let raw = HeapAlloc(GetProcessHeap(), 0, l.size() + l.align()) as *mut u8;
        if raw.is_null() {
            return raw;
        }
        let aligned = ((raw as usize + 4 + l.align() - 1) & !(l.align() - 1)) as *mut u8;
        *(aligned as *mut *mut u8).sub(1) = raw;
        aligned
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let raw = if l.align() <= 8 { p } else { *(p as *mut *mut u8).sub(1) };
        HeapFree(GetProcessHeap(), 0, raw as *mut c_void);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if l.align() <= 8 {
            return HeapReAlloc(GetProcessHeap(), 0, p as *mut c_void, new) as *mut u8;
        }
        let n = self.alloc(Layout::from_size_align_unchecked(new, l.align()));
        if !n.is_null() {
            core::ptr::copy_nonoverlapping(p, n, l.size().min(new));
            self.dealloc(p, l);
        }
        n
    }
}

#[global_allocator]
static HEAP: ProcessHeap = ProcessHeap;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let msg = alloc::format!("\r\nError interno: {}\r\n", info);
    out(msg.as_bytes());
    unsafe { ExitProcess(101) }
}

/// Process entry point (no C runtime).
#[no_mangle]
pub extern "C" fn mainCRTStartup() -> ! {
    let code = crate::app_main();
    unsafe { ExitProcess(code as u32) }
}

/// Path bytes -> NUL-terminated buffer for the `...A` APIs.
/// Embedded NUL bytes are dropped (the path validator never lets them through).
fn cstr(p: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(p.len() + 1);
    v.extend(p.iter().copied().filter(|&c| c != 0));
    v.push(0);
    v
}

/// `GetLastError()`, never 0 (so callers can always treat it as a failure).
fn last_err() -> Error {
    match unsafe { GetLastError() } {
        0 => 1,
        e => e,
    }
}

/// Last Winsock error (10xxx codes).
fn wsa_err() -> Error {
    unsafe { WSAGetLastError() as u32 }
}

// ------------------------------------------------------------------ process

/// Writes raw bytes to the console (stdout). Bytes are shown in the console
/// code page, like `printf`.
pub fn out(s: &[u8]) {
    let mut w = 0u32;
    unsafe {
        WriteFile(GetStdHandle(-11i32 as u32), s.as_ptr(), s.len() as u32, &mut w, null_mut());
    }
}

/// Command line arguments (without the program name), split with the MSVCRT
/// rules by `args::split_cmdline`, because there is no C runtime to do it.
pub fn args() -> Vec<Vec<u8>> {
    let mut line = Vec::new();
    unsafe {
        let mut p = GetCommandLineA();
        while !p.is_null() && *p != 0 {
            line.push(*p);
            p = p.add(1);
        }
    }
    crate::args::split_cmdline(&line)
}

/// Folder that contains the running .exe: the folder shared on double click.
/// `GetModuleFileNameA` may return an 8.3 short path on Windows 9x; that is fine.
pub fn exe_dir() -> Vec<u8> {
    let mut buf = vec![0u8; 512];
    let n = unsafe { GetModuleFileNameA(null_mut(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    buf.truncate(n.min(buf.len()));
    match buf.iter().rposition(|&c| c == b'\\' || c == b'/') {
        Some(i) => {
            buf.truncate(i);
            if buf.len() == 2 && buf[1] == b':' {
                buf.push(b'\\'); // "C:" -> "C:\"
            }
            buf
        }
        None => b".".to_vec(),
    }
}

/// Absolute, normalized path (`GetFullPathNameA`), without trailing `\`
/// except for drive roots. Does not check that the path exists.
pub fn full_path(p: &[u8]) -> Option<Vec<u8>> {
    let c = cstr(p);
    let mut buf = vec![0u8; 520];
    let n = unsafe { GetFullPathNameA(c.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), null_mut()) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    buf.truncate(n);
    // Strip trailing separators except on drive roots ("C:\").
    while buf.len() > 3 && buf.last() == Some(&b'\\') {
        buf.pop();
    }
    Some(buf)
}

/// FILETIME struct -> u64 (100 ns ticks since 1601-01-01).
fn ft(f: FILETIME) -> u64 {
    ((f.hi as u64) << 32) | f.lo as u64
}

/// Current time, UTC FILETIME.
pub fn now_utc_ft() -> u64 {
    let mut f = FILETIME::default();
    unsafe { GetSystemTimeAsFileTime(&mut f) };
    ft(f)
}

/// UTC FILETIME -> local time FILETIME (current time zone rules).
pub fn ft_to_local(utc: u64) -> u64 {
    let u = FILETIME { lo: utc as u32, hi: (utc >> 32) as u32 };
    let mut l = FILETIME::default();
    if unsafe { FileTimeToLocalFileTime(&u, &mut l) } == 0 {
        return utc;
    }
    ft(l)
}

/// Current time, local FILETIME (used for log names and `ls` dates).
pub fn now_local_ft() -> u64 {
    ft_to_local(now_utc_ft())
}

/// True if Windows is installed in Spanish (any country): primary language
/// id LANG_SPANISH = 0x0A. `GetUserDefaultLangID` exists since Windows 95.
pub fn system_spanish() -> bool {
    unsafe { GetUserDefaultLangID() & 0x3ff == 0x0a }
}

/// Milliseconds since boot (wraps every 49 days; only used for durations).
pub fn ticks_ms() -> u32 {
    unsafe { GetTickCount() }
}

/// Blocks the calling thread.
pub fn sleep_ms(ms: u32) {
    unsafe { Sleep(ms) }
}

/// Sets the console window title.
pub fn set_title(t: &[u8]) {
    let c = cstr(t);
    unsafe { SetConsoleTitleA(c.as_ptr()) };
}

/// Thread entry: takes ownership of the boxed closure created by `spawn`.
unsafe extern "system" fn thread_start(p: *mut c_void) -> u32 {
    let f = Box::from_raw(p as *mut Box<dyn FnOnce() + Send>);
    f();
    0
}

/// Runs `f` on a new thread (one per FTP client). Returns false if the thread
/// could not be created. The thread handle is closed immediately (detached).
pub fn spawn(f: Box<dyn FnOnce() + Send>) -> bool {
    let p = Box::into_raw(Box::new(f)) as *mut c_void;
    let mut tid = 0u32; // must not be NULL on Windows 9x
    let h = unsafe { CreateThread(null_mut(), 0, thread_start, p, 0, &mut tid) };
    if h.is_null() {
        drop(unsafe { Box::from_raw(p as *mut Box<dyn FnOnce() + Send>) });
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}

/// CRITICAL_SECTION wrapper (24 bytes on x86, must never move once initialized).
pub struct Lock(*mut c_void);
unsafe impl Send for Lock {}
unsafe impl Sync for Lock {}

impl Lock {
    pub fn new() -> Lock {
        let cs = Box::into_raw(Box::new([0u32; 8])) as *mut c_void;
        unsafe { InitializeCriticalSection(cs) };
        Lock(cs)
    }
    pub fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        unsafe { EnterCriticalSection(self.0) };
        let r = f();
        unsafe { LeaveCriticalSection(self.0) };
        r
    }
}

// ------------------------------------------------------------------ files

const ATTR_READONLY: u32 = 0x1;
const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_DIRECTORY: u32 = 0x10;
const INVALID_ATTRS: u32 = 0xFFFF_FFFF;

/// Converts a `WIN32_FIND_DATAA` into our portable `Meta`.
fn meta_from(fd: &FIND_DATA) -> Meta {
    let is_dir = fd.attrs & ATTR_DIRECTORY != 0;
    Meta {
        is_dir,
        readonly: fd.attrs & ATTR_READONLY != 0,
        hidden: fd.attrs & (ATTR_HIDDEN | ATTR_SYSTEM) != 0,
        size: if is_dir { 0 } else { ((fd.size_hi as u64) << 32) | fd.size_lo as u64 },
        mtime: ft(fd.written),
    }
}

/// File name inside a `WIN32_FIND_DATAA` (up to the NUL terminator).
fn name_of(fd: &FIND_DATA) -> &[u8] {
    let n = fd.name.iter().position(|&c| c == 0).unwrap_or(fd.name.len());
    &fd.name[..n]
}

/// Zeroed `WIN32_FIND_DATAA` on the heap (320 bytes; keeps stack frames small).
fn new_find_data() -> Box<FIND_DATA> {
    Box::new(FIND_DATA {
        attrs: 0,
        created: FILETIME::default(),
        accessed: FILETIME::default(),
        written: FILETIME::default(),
        size_hi: 0,
        size_lo: 0,
        reserved0: 0,
        reserved1: 0,
        name: [0; 260],
        alt_name: [0; 14],
    })
}

/// Information about one file or folder (like `stat`). Uses `FindFirstFileA`,
/// which exists on every Windows version and returns size, attributes and
/// date in one call; drive roots fall back to `GetFileAttributesA`.
pub fn metadata(path: &[u8]) -> Result<Meta, Error> {
    let c = cstr(path);
    let is_root = path.last() == Some(&b'\\') || (path.len() == 2 && path[1] == b':');
    if !is_root {
        let mut fd = new_find_data();
        let h = unsafe { FindFirstFileA(c.as_ptr(), &mut *fd) };
        if h != INVALID_HANDLE {
            unsafe { FindClose(h) };
            return Ok(meta_from(&fd));
        }
    }
    // Drive roots and shares can't be "found"; ask for the attributes instead.
    let a = unsafe { GetFileAttributesA(c.as_ptr()) };
    if a == INVALID_ATTRS {
        return Err(last_err());
    }
    Ok(Meta { is_dir: a & ATTR_DIRECTORY != 0, readonly: a & ATTR_READONLY != 0, hidden: false, size: 0, mtime: 0 })
}

/// All entries of a folder except `.` and `..`, unsorted.
pub fn read_dir(dir: &[u8]) -> Result<Vec<(Vec<u8>, Meta)>, Error> {
    let mut pat = dir.to_vec();
    if pat.last() != Some(&b'\\') {
        pat.push(b'\\');
    }
    pat.push(b'*');
    let c = cstr(&pat);
    let mut fd = new_find_data();
    let h = unsafe { FindFirstFileA(c.as_ptr(), &mut *fd) };
    if h == INVALID_HANDLE {
        let e = last_err();
        // An empty drive root has no "." entry and reports "file not found".
        return if e == 2 || e == 18 { Ok(Vec::new()) } else { Err(e) };
    }
    let mut v = Vec::new();
    loop {
        let name = name_of(&fd);
        if name != b"." && name != b".." {
            v.push((name.to_vec(), meta_from(&fd)));
        }
        if unsafe { FindNextFileA(h, &mut *fd) } == 0 {
            break;
        }
    }
    unsafe { FindClose(h) };
    Ok(v)
}

/// Win32 BOOL result -> Result.
fn ok_or_err(r: i32) -> Result<(), Error> {
    if r != 0 {
        Ok(())
    } else {
        Err(last_err())
    }
}

/// Deletes a file.
pub fn delete_file(p: &[u8]) -> Result<(), Error> {
    let c = cstr(p);
    ok_or_err(unsafe { DeleteFileA(c.as_ptr()) })
}
/// Creates one folder (not recursive).
pub fn create_dir(p: &[u8]) -> Result<(), Error> {
    let c = cstr(p);
    ok_or_err(unsafe { CreateDirectoryA(c.as_ptr(), null_mut()) })
}
/// Removes an empty folder.
pub fn remove_dir(p: &[u8]) -> Result<(), Error> {
    let c = cstr(p);
    ok_or_err(unsafe { RemoveDirectoryA(c.as_ptr()) })
}
/// Renames / moves. Fails if `to` exists (`MoveFileA` semantics).
pub fn rename(from: &[u8], to: &[u8]) -> Result<(), Error> {
    let (a, b) = (cstr(from), cstr(to));
    ok_or_err(unsafe { MoveFileA(a.as_ptr(), b.as_ptr()) })
}
/// Adds the hidden attribute (used on the `logs` folder). Errors are ignored.
pub fn set_hidden(p: &[u8]) {
    let c = cstr(p);
    unsafe {
        let a = GetFileAttributesA(c.as_ptr());
        if a != INVALID_ATTRS {
            SetFileAttributesA(c.as_ptr(), a | ATTR_HIDDEN);
        }
    }
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SHARE_READ: u32 = 1;
const SHARE_WRITE: u32 = 2;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FLAG_SEQUENTIAL: u32 = 0x0800_0000;
const ATTR_NORMAL: u32 = 0x80;

/// Open file handle, closed on drop. Opened for either reading or writing.
pub struct File(HANDLE);

impl File {
    /// Opens an existing file for sequential reading (others may still read/write it).
    pub fn open_read(p: &[u8]) -> Result<File, Error> {
        let c = cstr(p);
        let h = unsafe {
            CreateFileA(
                c.as_ptr(),
                GENERIC_READ,
                SHARE_READ | SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                ATTR_NORMAL | FLAG_SEQUENTIAL,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE {
            Err(last_err())
        } else {
            Ok(File(h))
        }
    }

    /// Opens or creates a file for writing according to `mode` (see `WriteMode`).
    pub fn open_write(p: &[u8], mode: WriteMode) -> Result<File, Error> {
        let c = cstr(p);
        let disp = if let WriteMode::Truncate = mode { CREATE_ALWAYS } else { OPEN_ALWAYS };
        let h = unsafe { CreateFileA(c.as_ptr(), GENERIC_WRITE, SHARE_READ, null_mut(), disp, ATTR_NORMAL | FLAG_SEQUENTIAL, null_mut()) };
        if h == INVALID_HANDLE {
            return Err(last_err());
        }
        let mut f = File(h);
        match mode {
            WriteMode::Truncate => {}
            WriteMode::Append => f.seek_raw(0, 2)?,
            WriteMode::At(off) => {
                f.seek_raw(off, 0)?;
                ok_or_err(unsafe { SetEndOfFile(f.0) })?;
            }
        }
        Ok(f)
    }

    /// `SetFilePointer` with a 64-bit offset split in two dwords
    /// (`SetFilePointerEx` does not exist on Windows 9x). `method`: 0 = begin, 2 = end.
    fn seek_raw(&mut self, pos: u64, method: u32) -> Result<(), Error> {
        let mut hi = (pos >> 32) as i32;
        unsafe {
            SetLastError(0);
            let r = SetFilePointer(self.0, pos as u32 as i32, &mut hi, method);
            if r == 0xFFFF_FFFF && GetLastError() != 0 {
                return Err(last_err());
            }
        }
        Ok(())
    }

    /// Moves the read/write position to `pos` bytes from the start.
    pub fn seek(&mut self, pos: u64) -> Result<(), Error> {
        self.seek_raw(pos, 0)
    }

    /// Reads up to `buf.len()` bytes; `Ok(0)` means end of file.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut n = 0u32;
        if unsafe { ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) } == 0 {
            return Err(last_err());
        }
        Ok(n as usize)
    }

    /// Writes the whole buffer (retrying partial writes).
    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            let mut n = 0u32;
            if unsafe { WriteFile(self.0, buf.as_ptr(), buf.len() as u32, &mut n, null_mut()) } == 0 {
                return Err(last_err());
            }
            if n == 0 {
                return Err(super::E_DISK_FULL);
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

const CP_ACP: u32 = 0;
const CP_UTF8: u32 = 65001;

/// Re-encodes text between two code pages through UTF-16. Pure ASCII is
/// returned unchanged (fast path); on any API failure the input is returned.
fn convert(b: &[u8], from: u32, to: u32) -> Vec<u8> {
    if b.is_empty() || b.iter().all(|c| c.is_ascii()) {
        return b.to_vec();
    }
    unsafe {
        let wn = MultiByteToWideChar(from, 0, b.as_ptr(), b.len() as i32, null_mut(), 0);
        if wn <= 0 {
            return b.to_vec();
        }
        let mut w = vec![0u16; wn as usize];
        MultiByteToWideChar(from, 0, b.as_ptr(), b.len() as i32, w.as_mut_ptr(), wn);
        let n = WideCharToMultiByte(to, 0, w.as_ptr(), wn, null_mut(), 0, null(), null_mut());
        if n <= 0 {
            return b.to_vec();
        }
        let mut o = vec![0u8; n as usize];
        WideCharToMultiByte(to, 0, w.as_ptr(), wn, o.as_mut_ptr(), n, null(), null_mut());
        o
    }
}

/// UTF-8 (from the client, after OPTS UTF8 ON) to the ANSI code page.
/// Characters that don't exist in the code page become '?', which the path
/// validator rejects, so they can never match the wrong file.
pub fn utf8_to_local(b: &[u8]) -> Vec<u8> {
    convert(b, CP_UTF8, CP_ACP)
}
/// ANSI code page -> UTF-8 (names sent to clients after OPTS UTF8 ON).
pub fn local_to_utf8(b: &[u8]) -> Vec<u8> {
    convert(b, CP_ACP, CP_UTF8)
}

// ------------------------------------------------------------------ network

const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const IPPROTO_TCP: i32 = 6;
const SOL_SOCKET: i32 = 0xffff;
const SO_SNDBUF: i32 = 0x1001;
const SO_RCVBUF: i32 = 0x1002;
const TCP_NODELAY: i32 = 1;

/// Largest single `send()` call (see `Socket::send_all`).
const SEND_CHUNK: usize = 32 * 1024;
/// A send that cannot make progress for this long fails (client gone).
const SEND_TIMEOUT_MS: u32 = 120_000;

/// Starts Winsock 2 (`WSAStartup`). Must succeed before any socket call.
pub fn net_init() -> bool {
    let mut data = vec![0u8; 512];
    unsafe { WSAStartup(0x0202, data.as_mut_ptr()) == 0 }
}

/// Portable address -> `sockaddr_in` (port in network byte order).
fn to_sa(a: SockAddr) -> SOCKADDR_IN {
    SOCKADDR_IN { family: AF_INET as u16, port: a.port.to_be(), addr: a.ip, zero: [0; 8] }
}

/// `sockaddr_in` -> portable address.
fn from_sa(s: &SOCKADDR_IN) -> SockAddr {
    SockAddr { ip: s.addr, port: u16::from_be(s.port) }
}

/// TCP socket (listening or connected), closed on drop.
pub struct Socket(SOCKET);

impl Drop for Socket {
    fn drop(&mut self) {
        unsafe { closesocket(self.0) };
    }
}

/// Waits until `s` is readable (`write == false`: data, connection, or close)
/// or writable (`write == true`: room in the send buffer) using `select()`.
/// `timeout_ms == 0` means "don't wait at all, just proceed" (blocking call).
fn wait_ready(s: SOCKET, timeout_ms: u32, write: bool) -> Result<(), Error> {
    if timeout_ms == 0 {
        return Ok(());
    }
    let mut set = FD_SET { count: 1, sockets: [0; 64] };
    set.sockets[0] = s;
    let tv = TIMEVAL { sec: (timeout_ms / 1000) as i32, usec: ((timeout_ms % 1000) * 1000) as i32 };
    let (r, w) = if write { (null_mut(), &mut set as *mut FD_SET) } else { (&mut set as *mut FD_SET, null_mut()) };
    match unsafe { select(0, r, w, null_mut(), &tv) } {
        0 => Err(E_TIMEOUT),
        n if n < 0 => Err(wsa_err()),
        _ => Ok(()),
    }
}

fn wait_readable(s: SOCKET, timeout_ms: u32) -> Result<(), Error> {
    wait_ready(s, timeout_ms, false)
}

/// `setsockopt` with an int value; failures are ignored (options are optional).
fn set_int(s: SOCKET, level: i32, name: i32, v: i32) {
    unsafe { setsockopt(s, level, name, &v as *const i32 as *const u8, 4) };
}

impl Socket {
    /// Binds `ip:port` (port 0 = any free port) and starts listening.
    pub fn listen(ip: [u8; 4], port: u16, backlog: i32) -> Result<Socket, Error> {
        let s = unsafe { socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) };
        if s == INVALID_SOCKET {
            return Err(wsa_err());
        }
        let sock = Socket(s);
        let sa = to_sa(SockAddr { ip, port });
        if unsafe { bind(s, &sa, 16) } != 0 || unsafe { listen(s, backlog) } != 0 {
            return Err(wsa_err());
        }
        Ok(sock)
    }

    /// Waits for an incoming connection (`timeout_ms == 0`: forever).
    /// Returns `Err(E_TIMEOUT)` on timeout. TCP_NODELAY is enabled on the result.
    pub fn accept(&self, timeout_ms: u32) -> Result<(Socket, SockAddr), Error> {
        wait_readable(self.0, timeout_ms)?;
        let mut sa = to_sa(SockAddr { ip: [0; 4], port: 0 });
        let mut len = 16i32;
        let s = unsafe { accept(self.0, &mut sa, &mut len) };
        if s == INVALID_SOCKET {
            return Err(wsa_err());
        }
        set_int(s, IPPROTO_TCP, TCP_NODELAY, 1);
        Ok((Socket(s), from_sa(&sa)))
    }

    /// Closes connections already waiting in the accept queue (stale data
    /// connections left over from a failed command).
    pub fn drain_pending(&self) {
        for _ in 0..16 {
            let mut set = FD_SET { count: 1, sockets: [0; 64] };
            set.sockets[0] = self.0;
            let tv = TIMEVAL { sec: 0, usec: 0 };
            if unsafe { select(0, &mut set, null_mut(), null_mut(), &tv) } <= 0 {
                return;
            }
            let s = unsafe { accept(self.0, null_mut(), null_mut()) };
            if s == INVALID_SOCKET {
                return;
            }
            unsafe { closesocket(s) };
        }
    }

    /// Opens an outgoing connection (active mode data connections).
    pub fn connect(addr: SockAddr) -> Result<Socket, Error> {
        let s = unsafe { socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) };
        if s == INVALID_SOCKET {
            return Err(wsa_err());
        }
        let sock = Socket(s);
        let sa = to_sa(addr);
        if unsafe { connect(s, &sa, 16) } != 0 {
            return Err(wsa_err());
        }
        Ok(sock)
    }

    /// Our side of the connection (`getsockname`).
    pub fn local_addr(&self) -> Option<SockAddr> {
        let mut sa = to_sa(SockAddr { ip: [0; 4], port: 0 });
        let mut len = 16i32;
        if unsafe { getsockname(self.0, &mut sa, &mut len) } != 0 {
            return None;
        }
        Some(from_sa(&sa))
    }

    /// The other side of the connection (`getpeername`).
    #[allow(dead_code)]
    pub fn peer_addr(&self) -> Option<SockAddr> {
        let mut sa = to_sa(SockAddr { ip: [0; 4], port: 0 });
        let mut len = 16i32;
        if unsafe { getpeername(self.0, &mut sa, &mut len) } != 0 {
            return None;
        }
        Some(from_sa(&sa))
    }

    /// Sends the whole buffer, retrying partial sends.
    ///
    /// Written for slow links and the Windows 9x TCP stack:
    /// * data goes out in chunks of `SEND_CHUNK` bytes; one huge `send()`
    ///   (256 KB) makes Winsock 9x lock a large non-paged buffer
    ///   and can stall old NIC drivers;
    /// * before each chunk we wait until the socket is writable, at most
    ///   `SEND_TIMEOUT_MS`. If the client stops reading (cable pulled, client
    ///   frozen), the transfer fails with a timeout instead of blocking the
    ///   session forever.
    pub fn send_all(&self, mut buf: &[u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            wait_ready(self.0, SEND_TIMEOUT_MS, true)?;
            let chunk = buf.len().min(SEND_CHUNK) as i32;
            let n = unsafe { send(self.0, buf.as_ptr(), chunk, 0) };
            if n <= 0 {
                return Err(wsa_err());
            }
            buf = &buf[n as usize..];
        }
        Ok(())
    }

    /// Receives up to `buf.len()` bytes. `Ok(0)` = peer closed the connection,
    /// `Err(E_TIMEOUT)` = nothing arrived within `timeout_ms` (0 = wait forever).
    pub fn recv(&self, buf: &mut [u8], timeout_ms: u32) -> Result<usize, Error> {
        wait_readable(self.0, timeout_ms)?;
        let n = unsafe { recv(self.0, buf.as_mut_ptr(), buf.len() as i32, 0) };
        if n < 0 {
            Err(wsa_err())
        } else {
            Ok(n as usize)
        }
    }

    /// Bigger socket buffers for data connections (default 8 KB on Win9x).
    /// 64 KB is the largest TCP window Windows 9x can use (no window scaling);
    /// asking for more only wastes non-paged memory on those systems.
    pub fn tune_bulk(&self) {
        set_int(self.0, SOL_SOCKET, SO_SNDBUF, 64 * 1024);
        set_int(self.0, SOL_SOCKET, SO_RCVBUF, 64 * 1024);
    }
}

/// IPv4 addresses of this PC (gethostbyname).
pub fn host_ips() -> Vec<[u8; 4]> {
    let mut v = Vec::new();
    let mut name = vec![0u8; 256];
    unsafe {
        if gethostname(name.as_mut_ptr(), 255) != 0 {
            return v;
        }
        let he = gethostbyname(name.as_ptr());
        if he.is_null() || (*he).length != 4 || (*he).addr_list.is_null() {
            return v;
        }
        let mut p = (*he).addr_list;
        while !(*p).is_null() && v.len() < 16 {
            let a = *p;
            let ip = [*a, *a.add(1), *a.add(2), *a.add(3)];
            if !v.contains(&ip) {
                v.push(ip);
            }
            p = p.add(1);
        }
    }
    v
}
