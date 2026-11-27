// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! # One FTP client connection
//!
//! Every accepted control connection runs [`run`] on its own thread. A session
//! reads one command line at a time from the control socket, dispatches it in
//! [`Session::dispatch`] and writes one reply. File contents and listings travel
//! over a separate *data connection*, opened per transfer in either:
//!
//! * **passive mode** (`PASV` / `EPSV`): we listen, the client connects to us
//!   (works behind the client's NAT, the common case today);
//! * **active mode** (`PORT` / `EPRT`): the client listens, we connect to it.
//!
//! ## Compatibility
//!
//! The command set and reply texts follow classic Windows FTP servers wherever they are correct
//! ("331 pretend login accepted", "226 Transfer Complete", ...), so existing
//! clients and scripts keep working.
//!
//! ## Rules for contributors
//!
//! * Never pass a client-supplied path to the file system directly: always go
//!   through [`Session::resolve`] (or `path::resolve*` + `is_protected`).
//! * Every command must send exactly one final reply (multi-line replies count
//!   as one), otherwise the client and server get out of sync.
//! * Write operations must check [`Session::read_only`] first.
//! * Everything is bytes (`[u8]`): file names are ANSI on Windows, not UTF-8.

use crate::args::Config;
use crate::listing::long_line;
use crate::path::{self, PathError};
use crate::sys::{self, error_text, File, SockAddr, Socket, WriteMode, E_TIMEOUT};
use crate::timefmt::{from_filetime, mdtm};
use crate::util::{cat, cmp_ic, eq_ic, find, fmt_ipv4, parse_ipv4, parse_u16, parse_u64, parse_u8, trim};
use crate::{log, VERSION};
use alloc::{format, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};

/// State shared by all sessions.
///
/// Created once in `main.rs`, leaked to get a `&'static` reference that every
/// session thread can hold. Only the atomic counters change after start-up, so
/// no lock is needed.
pub struct Shared {
    /// Parsed command line options.
    pub cfg: Config,
    /// Native absolute path of the shared folder (e.g. `D:\Fotos`).
    pub root: Vec<u8>,
    /// Virtual path that clients must never see or touch: the `logs` folder
    /// when it lives inside the shared folder (the double-click case).
    pub protected: Option<Vec<u8>>,
    /// Number of sessions currently running (for the `-m` limit).
    pub clients: AtomicU32,
    /// Next session number, only used to tag log lines (`[#3 192.168.0.7]`).
    pub next_id: AtomicU32,
    /// Round-robin cursor over the `-tp` passive port range.
    pub pasv_next: AtomicU32,
}

/// Transfer buffer size. 256 KB; big buffers are what make
/// transfers fast on old machines (fewer system calls).
const XFER_BUF: usize = 256 * 1024;
/// How long we wait for the client to connect to our passive port.
const DATA_ACCEPT_MS: u32 = 60_000;
/// Longest command line accepted; longer lines are discarded with a 500 reply.
const MAX_LINE: usize = 8192;

/// How the next data connection will be opened.
#[derive(Clone, Copy)]
enum DataMode {
    /// Neither PORT nor PASV received yet: transfers are refused with 425.
    None,
    /// Connect to this client address (PORT / EPRT).
    Active(SockAddr),
    /// Accept on `Session::pasv` (PASV / EPSV).
    Passive,
}

/// Per-connection state. Lives on the session thread only.
struct Session {
    sh: &'static Shared,
    /// Control connection (commands and replies).
    ctrl: Socket,
    /// Client address; data connections must come from / go to this IP.
    peer: SockAddr,
    /// Log prefix, e.g. `"[#3 192.168.0.7] "`.
    tag: Vec<u8>,
    /// Current *virtual* directory, always absolute ("/", "/fotos/2004").
    /// Never the process current directory.
    cwd: Vec<u8>,
    /// User name given with USER (only used when `-u` is set).
    user: Option<Vec<u8>>,
    logged_in: bool,
    failed_logins: u32,
    mode: DataMode,
    /// Passive listening socket, created on the first PASV/EPSV and reused
    /// for the rest of the session.
    pasv: Option<Socket>,
    /// Restart offset set by REST, consumed by the next RETR/STOR.
    rest: u64,
    /// TYPE A was requested. Only reported back; transfers are always binary.
    ascii: bool,
    /// Client enabled `OPTS UTF8 ON`: names are converted UTF-8 <-> ANSI.
    utf8: bool,
    /// Bytes received on the control connection not yet consumed as a line.
    inbuf: Vec<u8>,
    /// Transfer buffer, allocated on the first transfer only.
    buf: Vec<u8>,
}

/// What the main loop should do after a command.
enum Flow {
    Continue,
    Close,
}

/// Runs a whole client session until QUIT, disconnect, timeout or error.
/// Called on a dedicated thread by the accept loop in `main.rs`.
pub fn run(sh: &'static Shared, ctrl: Socket, peer: SockAddr, id: u32) {
    let tag = format!("[#{} {}] ", id, String::from_utf8_lossy(&fmt_ipv4(peer.ip))).into_bytes();
    let mut s = Session {
        sh,
        ctrl,
        peer,
        tag,
        cwd: b"/".to_vec(),
        user: None,
        // Without -u anybody is accepted.
        logged_in: sh.cfg.login.is_none(),
        failed_logins: 0,
        mode: DataMode::None,
        pasv: None,
        rest: 0,
        ascii: false,
        utf8: false,
        inbuf: Vec::new(),
        buf: Vec::new(),
    };
    s.log(crate::i18n::t("New connection", "Conexion nueva").as_bytes());
    s.reply(format!("220 InstantFTP {} ready", VERSION).as_bytes());
    while let Some(line) = s.read_line() {
        if let Flow::Close = s.dispatch(&line) {
            break;
        }
    }
    s.log(crate::i18n::t("Connection closed", "Conexion cerrada").as_bytes());
    // Sockets are closed by Drop when `s` goes out of scope.
}

impl Session {
    // ================================================================ I/O helpers

    /// Writes a log line prefixed with this session's tag.
    fn log(&self, msg: &[u8]) {
        log::line(&cat(&[&self.tag, msg]));
    }

    /// Sends a single-line reply (`"250 OK"`); CRLF is appended here.
    /// Replies are logged indented by 4 spaces.
    fn reply(&self, r: &[u8]) {
        self.log(&cat(&[b"    ", r]));
        let _ = self.ctrl.send_all(&cat(&[r, b"\r\n"]));
    }

    /// Sends a multi-line reply (RFC 959 §4.2):
    /// `first` ("211-Features:"), then `lines` (each already ending in CRLF,
    /// usually starting with a space), then `end` ("211 End").
    fn reply_multi(&self, first: &[u8], lines: &[u8], end: &[u8]) {
        self.log(&cat(&[b"    ", end]));
        let _ = self.ctrl.send_all(&cat(&[first, b"\r\n", lines, end, b"\r\n"]));
    }

    /// `550 <reason>` for a file system error code.
    fn reply_err(&self, e: sys::Error) {
        self.reply(format!("550 {}", error_text(e)).as_bytes());
    }

    /// Idle timeout in milliseconds (0 = wait forever).
    fn idle_ms(&self) -> u32 {
        self.sh.cfg.idle_timeout.saturating_mul(1000)
    }

    /// Reads one command line from the control connection, without its line
    /// ending. Returns `None` when the session must end (client closed the
    /// connection, network error, or idle timeout — the 421 reply is sent here).
    ///
    /// Several commands may arrive in one TCP packet ("pipelining"); the extra
    /// ones stay in `inbuf` for the next call. Dropping them would lose commands. Bare LF is accepted too.
    fn read_line(&mut self) -> Option<Vec<u8>> {
        // Stays below 4 KB so no stack probe is needed on the Win32 build.
        let mut tmp = [0u8; 2048];
        loop {
            if let Some(i) = find(&self.inbuf, b'\n') {
                let mut line: Vec<u8> = self.inbuf.drain(..=i).collect();
                line.pop(); // '\n'
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Some(line);
            }
            if self.inbuf.len() > MAX_LINE {
                self.inbuf.clear();
                self.reply(b"500 Command line too long");
            }
            match self.ctrl.recv(&mut tmp, self.idle_ms()) {
                Ok(0) => return None, // client closed the connection
                Ok(n) => self.inbuf.extend_from_slice(&tmp[..n]),
                Err(E_TIMEOUT) => {
                    self.reply(format!("421 Timeout ({} seconds): closing control connection", self.sh.cfg.idle_timeout).as_bytes());
                    return None;
                }
                Err(_) => return None,
            }
        }
    }

    // ================================================================ names and paths

    /// Client bytes -> local file name bytes (UTF-8 -> ANSI when enabled).
    fn local_arg(&self, a: &[u8]) -> Vec<u8> {
        if self.utf8 {
            sys::utf8_to_local(a)
        } else {
            a.to_vec()
        }
    }

    /// Local file name bytes -> bytes for the client (ANSI -> UTF-8 when enabled).
    fn client_name(&self, n: &[u8]) -> Vec<u8> {
        if self.utf8 {
            sys::local_to_utf8(n)
        } else {
            n.to_vec()
        }
    }

    /// True if `vpath` is the protected logs folder or something inside it.
    fn is_protected(&self, vpath: &[u8]) -> bool {
        self.sh.protected.as_deref().is_some_and(|p| path::is_within(vpath, p))
    }

    /// THE gatekeeper for client paths. Returns `(virtual, native)` paths, or
    /// sends "550 Path permission error" and returns `None` if the path is
    /// invalid, escapes the shared folder or points into the logs folder.
    fn resolve(&self, arg: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
        let local = self.local_arg(arg);
        match path::resolve(&self.cwd, &local) {
            Ok(v) if !self.is_protected(&v) => match path::to_native(&self.sh.root, &v, sys::SEP) {
                Some(n) => Some((v, n)),
                None => {
                    self.reply(b"550 Path too long");
                    None
                }
            },
            Ok(_) | Err(PathError::Invalid) | Err(PathError::Escapes) => {
                self.reply(b"550 Path permission error");
                None
            }
        }
    }

    /// If the server runs read-only (`-g` / `-r`), sends
    /// "`code` Permission denied" and returns true; the caller must stop.
    /// DELE and RNFR need this check too.
    fn read_only(&self, code: &str) -> bool {
        if self.sh.cfg.read_only {
            self.reply(format!("{} Permission denied", code).as_bytes());
        }
        self.sh.cfg.read_only
    }

    // ================================================================ dispatcher

    /// Parses one command line and runs the matching command.
    ///
    /// Line format: `VERB[ argument]`. The verb is case-insensitive; the
    /// argument is everything after the first space, up to the first control
    /// character (so file names may contain spaces).
    fn dispatch(&mut self, raw: &[u8]) -> Flow {
        // Skip Telnet IAC sequences (bytes >= 0xF0) some clients send before ABOR.
        let start = raw.iter().position(|&c| (0x20..0xF0).contains(&c)).unwrap_or(raw.len());
        let line = &raw[start..];
        let (verb_raw, arg) = match find(line, b' ') {
            Some(i) => (&line[..i], &line[i + 1..]),
            None => (line, &b""[..]),
        };
        let arg_end = arg.iter().position(|&c| c < 0x20).unwrap_or(arg.len());
        let arg = arg[..arg_end].to_vec();
        let verb: Vec<u8> = verb_raw.iter().take(8).map(|c| c.to_ascii_uppercase()).collect();

        // Log the command; never log passwords.
        if verb.as_slice() == b"PASS" {
            self.log(b"PASS ****");
        } else {
            let sep: &[u8] = if arg.is_empty() { b"" } else { b" " };
            self.log(&cat(&[&verb, sep, &arg]));
        }

        let v = verb.as_slice();
        // A REST offset only applies to the command right after it.
        let rest = core::mem::take(&mut self.rest);

        // With -u, only these commands work before logging in.
        if !self.logged_in && !matches!(v, b"USER" | b"PASS" | b"QUIT" | b"SYST" | b"FEAT" | b"NOOP" | b"HELP" | b"OPTS") {
            self.reply(b"530 Please login with USER and PASS");
            return Flow::Continue;
        }

        match v {
            // ---- session / login
            b"USER" => self.cmd_user(&arg),
            b"PASS" => return self.cmd_pass(&arg),
            // Our listings are Unix `ls -l` style, so we say so.
            b"SYST" => self.reply(b"215 UNIX Type: L8"),
            b"QUIT" => {
                self.reply(b"221 goodbye");
                return Flow::Close;
            }
            b"NOOP" => self.reply(b"200 OK"),
            b"ALLO" => self.reply(b"202 ALLO command ignored"),
            // Transfers are synchronous, so there is never one to abort.
            b"ABOR" => self.reply(b"226 Aborted"),
            b"CLNT" => self.reply(b"200 OK"),
            // Keep FEAT in sync with what is actually implemented (RFC 2389).
            b"FEAT" => self.reply_multi(
                b"211-Features:",
                b" EPRT\r\n EPSV\r\n MDTM\r\n PASV\r\n REST STREAM\r\n SIZE\r\n TVFS\r\n UTF8\r\n",
                b"211 End",
            ),
            b"HELP" => self.reply_multi(
                b"214-The following commands are recognized:",
                b" ABOR ALLO APPE CDUP CWD DELE EPRT EPSV FEAT HELP LIST MDTM MKD MODE NLST NOOP\r\n \
                  OPTS PASS PASV PORT PWD QUIT REST RETR RMD RNFR RNTO SIZE STAT STOR STRU SYST\r\n \
                  TYPE USER XCUP XCWD XMKD XPWD XRMD\r\n",
                b"214 Help OK",
            ),
            b"OPTS" => self.cmd_opts(&arg),

            // ---- transfer parameters
            b"TYPE" => self.cmd_type(&arg),
            b"MODE" => self.reply(if eq_ic(trim(&arg), b"S") { b"200 Mode set to S" } else { b"504 Only stream mode supported" }),
            b"STRU" => self.reply(if eq_ic(trim(&arg), b"F") { b"200 Structure set to F" } else { b"504 Only file structure supported" }),

            // ---- directories
            b"PWD" | b"XPWD" => {
                // Quotes inside the name are doubled (RFC 959).
                let q = path::quote(&self.client_name(&self.cwd));
                self.reply(&cat(&[b"257 ", &q]));
            }
            b"CWD" | b"XCWD" => self.cmd_cwd(&arg),
            b"CDUP" | b"XCUP" => {
                // At the root, CDUP stays at the root.
                self.cwd = path::split_last(&self.cwd).0.to_vec();
                self.reply(b"250 CWD command successful");
            }

            // ---- data connection setup
            b"PORT" => self.cmd_port(&arg),
            b"EPRT" => self.cmd_eprt(&arg),
            b"PASV" => self.cmd_pasv(false, &arg),
            b"EPSV" => self.cmd_pasv(true, &arg),
            b"REST" => match parse_u64(trim(&arg)) {
                Some(n) => {
                    self.rest = n;
                    self.reply(format!("350 Restarting at {}. Send STORE or RETRIEVE", n).as_bytes());
                }
                None => self.reply(b"501 Invalid REST parameter"),
            },

            // ---- listings
            b"LIST" => self.cmd_list(&arg, true, false),
            b"NLST" => self.cmd_list(&arg, false, false),
            b"STAT" => {
                if arg.is_empty() {
                    self.cmd_status();
                } else {
                    // `STAT path` = LIST over the control connection.
                    self.cmd_list(&arg, true, true);
                }
            }

            // ---- files
            b"RETR" => self.cmd_retr(&arg, rest),
            b"STOR" => self.cmd_store(&arg, if rest > 0 { WriteMode::At(rest) } else { WriteMode::Truncate }),
            b"APPE" => self.cmd_store(&arg, WriteMode::Append),
            b"SIZE" | b"MDTM" => self.cmd_stat_file(&arg, v == b"SIZE"),
            b"DELE" => self.cmd_dele(&arg),
            b"MKD" | b"XMKD" => self.cmd_mkd(&arg),
            b"RMD" | b"XRMD" => self.cmd_rmd(&arg),
            // RNFR consumes the following RNTO itself, so a lone RNTO is an error.
            b"RNFR" => return self.cmd_rnfr(&arg),
            b"RNTO" => self.reply(b"503 Bad sequence of commands"),

            _ => {
                self.log(&cat(&[b"Unknown command '", &verb, b"'"]));
                self.reply(b"500 command not recognized");
            }
        }
        Flow::Continue
    }

    // ================================================================ login

    /// USER: without `-u` any name is accepted ("pretend login").
    fn cmd_user(&mut self, arg: &[u8]) {
        if self.sh.cfg.login.is_none() {
            self.reply(b"331 pretend login accepted");
            return;
        }
        self.user = Some(arg.to_vec());
        self.logged_in = false;
        self.reply(b"331 Password required");
    }

    /// PASS: checks the `-u user:pass` pair. After a wrong password we wait
    /// one second, and after three we hang up, to slow down guessing.
    fn cmd_pass(&mut self, arg: &[u8]) -> Flow {
        let Some((u, p)) = self.sh.cfg.login.as_ref() else {
            self.reply(b"230 fake user logged in");
            return Flow::Continue;
        };
        if self.logged_in {
            self.reply(b"230 Already logged in");
        } else if self.user.as_deref() == Some(u.as_slice()) && arg == p.as_slice() {
            self.logged_in = true;
            self.reply(b"230 Login successful");
        } else {
            self.failed_logins += 1;
            sys::sleep_ms(1000);
            if self.failed_logins >= 3 {
                self.reply(b"421 Too many failed logins");
                return Flow::Close;
            }
            self.reply(b"530 Login incorrect");
        }
        Flow::Continue
    }

    /// OPTS UTF8 ON/OFF: switches UTF-8 file name conversion (RFC 2640).
    fn cmd_opts(&mut self, arg: &[u8]) {
        let a = trim(arg);
        if eq_ic(a, b"UTF8 ON") || eq_ic(a, b"UTF8") {
            self.utf8 = true;
            self.reply(b"200 UTF8 set to on");
        } else if eq_ic(a, b"UTF8 OFF") {
            self.utf8 = false;
            self.reply(b"200 UTF8 set to off");
        } else {
            self.reply(b"501 Option not understood");
        }
    }

    /// TYPE A / I. Data is always sent as-is (binary); we only
    /// answer truthfully instead of always "Type set to I".
    fn cmd_type(&mut self, arg: &[u8]) {
        match trim(arg).first().map(|c| c.to_ascii_uppercase()) {
            Some(b'A') => {
                self.ascii = true;
                self.reply(b"200 Type set to A");
            }
            Some(b'I') | Some(b'L') => {
                self.ascii = false;
                self.reply(b"200 Type set to I");
            }
            _ => self.reply(b"504 Type not supported"),
        }
    }

    /// STAT without argument: session status.
    fn cmd_status(&self) {
        let mut body = format!(
            " Connected from {}\r\n Logged in{}\r\n TYPE: {}, STRUcture: File, MODE: Stream\r\n",
            String::from_utf8_lossy(&fmt_ipv4(self.peer.ip)),
            if self.sh.cfg.read_only { " (read only)" } else { "" },
            if self.ascii { "ASCII" } else { "BINARY" },
        )
        .into_bytes();
        body.extend_from_slice(b" InstantFTP ");
        body.extend_from_slice(VERSION.as_bytes());
        body.extend_from_slice(b"\r\n");
        self.reply_multi(b"211-FTP server status:", &body, b"211 End of status");
    }

    // ================================================================ directories

    /// CWD: changes the virtual directory if the target exists and is a folder.
    /// `..` works, but never above the shared root.
    fn cmd_cwd(&mut self, arg: &[u8]) {
        if arg.is_empty() {
            // An empty CWD is a no-op; kept for compatibility.
            self.reply(b"250 CWD command successful");
            return;
        }
        let local = self.local_arg(arg);
        let ok = match path::resolve(&self.cwd, &local) {
            Ok(v) if !self.is_protected(&v) => match path::to_native(&self.sh.root, &v, sys::SEP) {
                Some(n) if sys::metadata(&n).map(|m| m.is_dir).unwrap_or(false) => {
                    self.cwd = v;
                    true
                }
                _ => false,
            },
            _ => false,
        };
        self.reply(if ok { b"250 CWD command successful" } else { b"550 Could not change directory" });
    }

    /// MKD: creates a folder and returns its quoted path (RFC 959).
    fn cmd_mkd(&mut self, arg: &[u8]) {
        if self.read_only("550") {
            return;
        }
        let Some((v, n)) = self.resolve(arg) else { return };
        match sys::create_dir(&n) {
            Ok(()) => {
                let q = path::quote(&self.client_name(&v));
                self.reply(&cat(&[b"257 ", &q, b" Directory created"]));
            }
            Err(e) => self.reply_err(e),
        }
    }

    /// RMD: removes an empty folder. The shared root itself is refused.
    fn cmd_rmd(&mut self, arg: &[u8]) {
        if self.read_only("550") {
            return;
        }
        let Some((v, n)) = self.resolve(arg) else { return };
        if v == b"/" {
            self.reply(b"550 Permission denied");
            return;
        }
        match sys::remove_dir(&n) {
            Ok(()) => self.reply(b"250 RMD command successful"),
            Err(e) => self.reply_err(e),
        }
    }

    /// DELE: deletes a file. Blocked in read-only mode .
    fn cmd_dele(&mut self, arg: &[u8]) {
        if self.read_only("550") {
            return;
        }
        let Some((v, n)) = self.resolve(arg) else { return };
        if v == b"/" {
            self.reply(b"550 Permission denied");
            return;
        }
        match sys::delete_file(&n) {
            Ok(()) => self.reply(b"250 DELE command successful."),
            Err(e) => self.reply_err(e),
        }
    }

    /// RNFR + RNTO (rename/move). RFC 959 requires RNTO to follow RNFR
    /// immediately, so RNFR reads the next command itself: the pending name
    /// can never leak into a later, unrelated RNTO. RNTO alone is refused.
    /// If the next command is not RNTO, it is executed normally.
    fn cmd_rnfr(&mut self, arg: &[u8]) -> Flow {
        if self.read_only("550") {
            return Flow::Continue;
        }
        let Some((v, from)) = self.resolve(arg) else { return Flow::Continue };
        if v == b"/" {
            self.reply(b"550 Permission denied");
            return Flow::Continue;
        }
        if let Err(e) = sys::metadata(&from) {
            self.reply_err(e);
            return Flow::Continue;
        }
        self.reply(b"350 File Exists");
        let Some(next) = self.read_line() else { return Flow::Close };
        let is_rnto = next.len() >= 4 && eq_ic(&next[..4], b"RNTO") && (next.len() == 4 || next[4] == b' ');
        if !is_rnto {
            return self.dispatch(&next);
        }
        let arg = next.get(5..).unwrap_or(b"").to_vec();
        self.log(&cat(&[b"RNTO ", &arg]));
        let Some((v2, to)) = self.resolve(&arg) else { return Flow::Continue };
        if v2 == b"/" {
            self.reply(b"550 Permission denied");
            return Flow::Continue;
        }
        // MoveFileA semantics: fails if the target already exists (no silent overwrite).
        match sys::rename(&from, &to) {
            Ok(()) => self.reply(b"250 RNTO command successful"),
            Err(e) => self.reply_err(e),
        }
        Flow::Continue
    }

    // ================================================================ data connection

    /// PORT h1,h2,h3,h4,p1,p2 — active mode, IPv4 address and port.
    /// All six numbers must be present.
    fn cmd_port(&mut self, arg: &[u8]) {
        let nums: Vec<Option<u8>> = arg.split(|&c| c == b',').map(|p| parse_u8(trim(p))).collect();
        if nums.len() != 6 || nums.iter().any(|n| n.is_none()) {
            self.reply(b"501 Syntax error in PORT command");
            return;
        }
        let n: Vec<u8> = nums.into_iter().map(|x| x.unwrap_or(0)).collect();
        let addr = SockAddr { ip: [n[0], n[1], n[2], n[3]], port: (n[4] as u16) << 8 | n[5] as u16 };
        self.set_active(addr, b"200 PORT command successful");
    }

    /// EPRT |1|ip|port| — extended PORT (RFC 2428). The first character is
    /// the delimiter. Only protocol 1 (IPv4) exists on Windows 98.
    fn cmd_eprt(&mut self, arg: &[u8]) {
        let Some(&d) = arg.first() else {
            self.reply(b"501 Syntax error in EPRT command");
            return;
        };
        let f: Vec<&[u8]> = arg.split(|&c| c == d).collect();
        if f.len() != 5 || f[1] != b"1" {
            self.reply(b"522 Network protocol not supported, use (1)");
            return;
        }
        match (parse_ipv4(f[2]), parse_u16(f[3])) {
            (Some(ip), Some(port)) => self.set_active(SockAddr { ip, port }, b"200 EPRT command successful"),
            _ => self.reply(b"501 Syntax error in EPRT command"),
        }
    }

    /// Switches to active mode. Only connections back to the client itself, on
    /// a non-privileged port, are allowed: this blocks "FTP bounce" attacks,
    /// where the server is used to connect to third parties.
    /// Also leaves passive mode.
    fn set_active(&mut self, addr: SockAddr, ok: &[u8]) {
        if addr.ip != self.peer.ip || addr.port < 1024 {
            self.reply(b"500 Illegal PORT command");
            return;
        }
        self.mode = DataMode::Active(addr);
        self.reply(ok);
    }

    /// PASV (227 reply with IP and port) and EPSV (229 reply, port only).
    fn cmd_pasv(&mut self, extended: bool, arg: &[u8]) {
        if extended && eq_ic(trim(arg), b"ALL") {
            self.reply(b"200 EPSV ALL ok");
            return;
        }
        // One listening socket per session, reused by later PASV commands
        // (avoids TIME_WAIT trouble when -tp is a single port).
        match &self.pasv {
            None => self.pasv = self.open_pasv_listener(),
            // Anything already queued is a leftover from an earlier command
            // that failed before accepting; otherwise that dead connection would
            // reach the next transfer and hang it.
            Some(l) => l.drain_pending(),
        }
        let Some(port) = self.pasv.as_ref().and_then(|l| l.local_addr()).map(|a| a.port) else {
            self.reply(b"425 Can't open passive connection");
            return;
        };
        self.mode = DataMode::Passive;
        if extended {
            self.reply(format!("229 Entering Extended Passive Mode (|||{}|)", port).as_bytes());
            return;
        }
        // Announce the address the client reached us on, which is right even on
        // PCs with several network cards / VPNs. The first address from
        // gethostbyname() is often the wrong one.
        // -ha overrides it for NAT / port forwarding.
        let ip = self.sh.cfg.ext_ip.or_else(|| self.ctrl.local_addr().map(|a| a.ip)).unwrap_or([127, 0, 0, 1]);
        self.reply(format!("227 Entering Passive Mode ({},{},{},{},{},{})", ip[0], ip[1], ip[2], ip[3], port >> 8, port & 0xff).as_bytes());
    }

    /// Creates the passive listening socket: on a port chosen by the OS, or on
    /// the first free port of the `-tp` range, starting at a round-robin cursor
    /// so simultaneous sessions spread over the range.
    fn open_pasv_listener(&self) -> Option<Socket> {
        let bind = self.sh.cfg.bind_ip.unwrap_or([0; 4]);
        match self.sh.cfg.pasv_range {
            None => Socket::listen(bind, 0, 1).ok(),
            Some((lo, hi)) => {
                let count = (hi - lo) as u32 + 1;
                for _ in 0..count {
                    let i = self.sh.pasv_next.fetch_add(1, Ordering::Relaxed) % count;
                    if let Ok(s) = Socket::listen(bind, lo + i as u16, 1) {
                        return Some(s);
                    }
                }
                None
            }
        }
    }

    /// Opens the data connection. Call it right after the "150" reply.
    /// On failure sends "425" and returns `None`.
    ///
    /// Passive connections must come from the client's own IP, otherwise a
    /// third party could grab the transfer. Waiting is limited to
    /// 60 s.
    fn open_data(&mut self) -> Option<Socket> {
        let r = match self.mode {
            DataMode::None => {
                self.reply(b"425 Use PORT or PASV first");
                return None;
            }
            DataMode::Active(a) => Socket::connect(a).ok(),
            DataMode::Passive => match self.pasv.as_ref().map(|l| l.accept(DATA_ACCEPT_MS)) {
                Some(Ok((s, a))) if a.ip == self.peer.ip => Some(s),
                Some(Ok((_, a))) => {
                    self.log(&cat(&[
                        crate::i18n::t("Rejected data connection from another IP: ", "Rechazada conexion de datos desde otra IP: ")
                            .as_bytes(),
                        &fmt_ipv4(a.ip),
                    ]));
                    None
                }
                _ => None,
            },
        };
        match r {
            Some(s) => {
                s.tune_bulk();
                Some(s)
            }
            None => {
                self.reply(b"425 Can't open data connection");
                None
            }
        }
    }

    // ================================================================ listings

    /// LIST / NLST / STAT path.
    ///
    /// * `long`: `ls -l` style lines (LIST) or bare names (NLST).
    /// * `on_ctrl`: send over the control connection as a 213 reply (STAT).
    ///
    /// Accepts `ls`-like flags before the path (`-l`, `-a`, `-la`), a folder,
    /// a single file, or a wildcard in the last component (`fotos/*.jpg`).
    /// The path is kept when flags are given and, for a folder, its
    /// contents are listed.
    fn cmd_list(&mut self, arg: &[u8], mut long: bool, on_ctrl: bool) {
        let mut all = self.sh.cfg.show_hidden;
        let mut rest = trim(arg);
        while rest.first() == Some(&b'-') {
            let end = find(rest, b' ').unwrap_or(rest.len());
            for &c in &rest[1..end] {
                match c {
                    b'a' | b'A' => all = true,
                    b'l' => long = true,
                    _ => {} // unknown ls flags are ignored, as most servers do
                }
            }
            rest = trim(&rest[end..]);
        }
        let local = self.local_arg(rest);
        let v = match path::resolve_pattern(&self.cwd, &local) {
            Ok(v) if !self.is_protected(&v) => v,
            _ => {
                // No path: answer once and stop.
                self.reply(b"550 Path permission error");
                return;
            }
        };
        // Wildcard in the last component -> list the parent, filtered.
        let (parent, last) = path::split_last(&v);
        let (dir_v, pattern) = if path::has_wildcard(last) { (parent.to_vec(), Some(last.to_vec())) } else { (v.clone(), None) };
        let Some(dir_n) = path::to_native(&self.sh.root, &dir_v, sys::SEP) else {
            self.reply(b"550 Path too long");
            return;
        };

        let entries = match &pattern {
            Some(p) => sys::read_dir(&dir_n).map(|v| v.into_iter().filter(|(n, _)| path::wildcard_match(p, n)).collect()),
            None => match sys::metadata(&dir_n) {
                Ok(m) if m.is_dir => sys::read_dir(&dir_n),
                Ok(m) => Ok(vec![(path::split_last(&v).1.to_vec(), m)]), // a single file
                Err(e) => Err(e),
            },
        };
        let mut entries = match entries {
            Ok(e) => e,
            Err(e) => {
                self.reply_err(e);
                return;
            }
        };
        // Hide hidden/system files (unless -a) and the protected logs folder.
        let listing_dir = pattern.is_some() || sys::metadata(&dir_n).map(|m| m.is_dir).unwrap_or(false);
        entries.retain(|(name, m)| {
            if m.hidden && !all {
                return false;
            }
            if listing_dir {
                let sep: &[u8] = if dir_v == b"/" { b"" } else { b"/" };
                let child = cat(&[&dir_v, sep, name]);
                return !self.is_protected(&child);
            }
            true
        });
        // FAT/FAT32 return entries unsorted; sort like Explorer (case-insensitive).
        entries.sort_unstable_by(|a, b| cmp_ic(&a.0, &b.0));

        let now = from_filetime(sys::now_local_ft());
        let mut out = Vec::with_capacity(entries.len() * 64);
        for (name, m) in &entries {
            let n = self.client_name(name);
            if long {
                let t = from_filetime(sys::ft_to_local(m.mtime));
                out.extend_from_slice(&long_line(&n, m, &t, &now));
            } else {
                out.extend_from_slice(&n);
                out.extend_from_slice(b"\r\n");
            }
        }

        if on_ctrl {
            self.reply_multi(&cat(&[b"213-Status of ", rest, b":"]), &out, b"213 End of status");
            return;
        }
        self.reply(b"150 Opening connection");
        let Some(data) = self.open_data() else { return };
        if data.send_all(&out).is_err() {
            self.reply(b"426 Broken pipe");
            return;
        }
        drop(data); // closing the data connection marks the end of the listing
        self.reply(b"226 Transfer Complete");
    }

    /// SIZE (bytes, 64-bit) and MDTM (UTC, RFC 3659).
    fn cmd_stat_file(&mut self, arg: &[u8], size: bool) {
        let Some((_, n)) = self.resolve(arg) else { return };
        match sys::metadata(&n) {
            Err(e) => self.reply_err(e),
            Ok(m) if m.is_dir => self.reply(b"550 not a plain file."),
            Ok(m) if size => self.reply(format!("213 {}", m.size).as_bytes()),
            Ok(m) => self.reply(format!("213 {}", mdtm(&from_filetime(m.mtime))).as_bytes()),
        }
    }

    // ================================================================ transfers

    /// Allocates the 256 KB transfer buffer the first time it is needed, so
    /// sessions that only browse cost almost no memory.
    fn ensure_buf(&mut self) {
        if self.buf.len() != XFER_BUF {
            self.buf = vec![0u8; XFER_BUF];
        }
    }

    /// Logs "RETR ok /x - 1234 bytes, 56 ms, 21 KB/s" (integer math only).
    fn log_xfer(&self, what: &[u8], v: &[u8], bytes: u64, t0: u32) {
        let ms = sys::ticks_ms().wrapping_sub(t0).max(1) as u64;
        let kbs = bytes * 1000 / 1024 / ms;
        self.log(&cat(&[what, b" ", v, format!(" - {} bytes, {} ms, {} KB/s", bytes, ms, kbs).as_bytes()]));
    }

    /// RETR: sends a file, from offset `rest` if a REST came before (resume).
    fn cmd_retr(&mut self, arg: &[u8], rest: u64) {
        let Some((v, n)) = self.resolve(arg) else { return };
        let meta = match sys::metadata(&n) {
            Ok(m) if m.is_dir => {
                self.reply(b"550 not a plain file.");
                return;
            }
            Ok(m) => m,
            Err(e) => {
                self.reply_err(e);
                return;
            }
        };
        // Open the file before the 150 reply, so errors are reported as 550.
        let mut file = match File::open_read(&n) {
            Ok(f) => f,
            Err(e) => {
                self.reply_err(e);
                return;
            }
        };
        if rest > 0 && (rest > meta.size || file.seek(rest).is_err()) {
            self.reply(b"554 Invalid REST position");
            return;
        }
        self.reply(b"150 Opening BINARY mode data connection");
        let Some(data) = self.open_data() else { return };
        self.ensure_buf();
        let t0 = sys::ticks_ms();
        let mut total = 0u64;
        loop {
            let n = match file.read(&mut self.buf) {
                Ok(0) => break, // end of file
                Ok(n) => n,
                Err(e) => {
                    self.reply(format!("451 Read error: {}", error_text(e)).as_bytes());
                    return;
                }
            };
            // One reply only on failure: never 426 and then 550.
            if data.send_all(&self.buf[..n]).is_err() {
                self.log_xfer(crate::i18n::t("RETR interrupted", "RETR cortado").as_bytes(), &v, total, t0);
                self.reply(b"426 Broken pipe");
                return;
            }
            total += n as u64;
        }
        drop(data);
        self.log_xfer(b"RETR ok", &v, total, t0);
        self.reply(b"226 Transfer Complete");
    }

    /// STOR / APPE / REST+STOR: receives a file.
    /// `mode`: overwrite, append, or keep the first n bytes (resume upload).
    fn cmd_store(&mut self, arg: &[u8], mode: WriteMode) {
        if self.read_only("553") {
            return;
        }
        let Some((v, n)) = self.resolve(arg) else { return };
        let existed = match sys::metadata(&n) {
            Ok(m) if m.is_dir => {
                self.reply(b"550 Is a directory");
                return;
            }
            Ok(_) => true,
            Err(_) => false,
        };
        let mut file = match File::open_write(&n, mode) {
            Ok(f) => f,
            Err(e) => {
                self.reply_err(e);
                return;
            }
        };
        self.reply(b"150 Opening BINARY mode data connection");
        let Some(data) = self.open_data() else {
            // No data connection: don't leave an empty new file behind.
            drop(file);
            if !existed {
                let _ = sys::delete_file(&n);
            }
            return;
        };
        self.ensure_buf();
        let t0 = sys::ticks_ms();
        let mut total = 0u64;
        // A stalled upload must not block the session forever (min 60 s).
        let timeout = if self.sh.cfg.idle_timeout == 0 { 0 } else { self.idle_ms().max(60_000) };
        loop {
            let got = match data.recv(&mut self.buf, timeout) {
                Ok(0) => break, // client closed the data connection = end of file
                Ok(n) => n,
                Err(_) => {
                    // The partial file is kept so the client can resume (REST + STOR).
                    self.log_xfer(crate::i18n::t("STOR interrupted", "STOR cortado").as_bytes(), &v, total, t0);
                    self.reply(b"426 Connection closed; transfer aborted");
                    return;
                }
            };
            if let Err(e) = file.write_all(&self.buf[..got]) {
                // 112 / 39 = disk full on NT / Win9x.
                self.reply(if e == sys::E_DISK_FULL || e == 39 { &b"452 Insufficient storage space"[..] } else { &b"451 Write error"[..] });
                return;
            }
            total += got as u64;
        }
        drop(file);
        self.log_xfer(b"STOR ok", &v, total, t0);
        self.reply(b"226 Transfer Complete");
    }
}
