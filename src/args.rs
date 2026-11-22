// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! Command line parsing. All options are optional: double-clicking the .exe
//! shares the folder it lives in, on port 21, anonymous, with a daily log.

use crate::i18n::{self, t, Lang};
use crate::tr;
use crate::util::{parse_ipv4, parse_u16, parse_u64};
use alloc::{string::String, vec::Vec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Control port (-p). Default 21, falls back to 2100.
    pub port: u16,
    /// Passive port range (-tp 50000-50010).
    pub pasv_range: Option<(u16, u16)>,
    /// Address announced in PASV replies (-ha), for NAT.
    pub ext_ip: Option<[u8; 4]>,
    /// Listen only on this local address (-b).
    pub bind_ip: Option<[u8; 4]>,
    /// Read only: no uploads, deletes, renames or mkdir (-g / -r).
    pub read_only: bool,
    /// Folder to share. None = folder of the executable.
    pub root: Option<Vec<u8>>,
    /// Require this user/password (-u user:pass). None = anyone gets in.
    pub login: Option<(Vec<u8>, Vec<u8>)>,
    /// Idle timeout in seconds (-t), 0 = never.
    pub idle_timeout: u32,
    /// Max simultaneous clients (-m), 0 = unlimited.
    pub max_clients: u32,
    /// Write the daily log in <exe folder>\logs (disable with -nolog).
    pub log: bool,
    /// Show hidden/system files to clients (-a).
    pub show_hidden: bool,
    /// Console/log language forced with `-lang` (None = follow Windows).
    pub lang: Option<Lang>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 21,
            pasv_range: None,
            ext_ip: None,
            bind_ip: None,
            read_only: false,
            root: None,
            login: None,
            idle_timeout: 300,
            max_clients: 20,
            log: true,
            show_hidden: false,
            lang: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArgOutcome {
    Run(Config),
    Help,
    Version,
    Error(String),
}

/// English help text shown by `-h` / `/?` and after an invalid option.
/// Kept ASCII-only: the Windows 9x console uses an OEM code page.
const USAGE_EN: &str = "\
InstantFTP - instant, portable FTP server (Windows 98 SE to Windows 11)

Usage: InstantFTP [options] [folder]

Without options it shares the folder where the .exe is, on port 21, with no
user name, and keeps a daily log in logs\\YYYY-MM-DD.log

Options:
  -p <port>          Control port (default 21; falls back to 2100)
  -tp <p>[-<p2>]     Port or port range for passive mode (for firewalls)
  -ha <ip>           Public IP announced in PASV replies (NAT / port forwarding)
  -b <ip>            Listen on this local IP only
  -g, -r             Read only (no uploads, deletes or renames)
  -u <user:pass>     Require this user name and password (default: anyone)
  -t <seconds>       Disconnect idle clients (default 300, 0 = never)
  -m <n>             Maximum simultaneous clients (default 20, 0 = no limit)
  -a                 Show hidden and system files
  -nolog             Do not write the log to disk
  -lang <en|es>      Console language (default: Windows language)
  -h, /?             This help
  -v                 Version
  folder             Folder to share (default: the folder of the .exe)

InstantFTP by matcarfer - freeware - https://github.com/matcarfer/InstantFTP
";

/// Spanish help text (same content as `USAGE_EN`).
const USAGE_ES: &str = "\
InstantFTP - servidor FTP instantaneo y portable (Windows 98 SE a Windows 11)

Uso: InstantFTP [opciones] [carpeta]

Sin opciones comparte la carpeta donde esta el .exe, en el puerto 21,
sin usuario, y guarda un log diario en logs\\AAAA-MM-DD.log

Opciones:
  -p <puerto>        Puerto de control (default 21, si falla usa 2100)
  -tp <p>[-<p2>]     Puerto o rango para modo pasivo (util con firewall)
  -ha <ip>           IP publica a anunciar en PASV (para NAT/port forwarding)
  -b <ip>            Escuchar solo en esta IP local
  -g, -r             Solo lectura (no permite subir, borrar ni renombrar)
  -u <user:clave>    Exigir usuario y clave (por defecto entra cualquiera)
  -t <segundos>      Desconectar clientes inactivos (default 300, 0 = nunca)
  -m <n>             Maximo de clientes simultaneos (default 20, 0 = sin limite)
  -a                 Mostrar archivos ocultos y de sistema
  -nolog             No guardar log en disco
  -lang <en|es>      Idioma de la consola (default: el de Windows)
  -h, /?             Esta ayuda
  -v                 Version
  carpeta            Carpeta a compartir (default: la carpeta del .exe)

InstantFTP por matcarfer - gratis - https://github.com/matcarfer/InstantFTP
";

/// Help text in the current language.
pub fn usage() -> &'static str {
    t(USAGE_EN, USAGE_ES)
}

/// Finds `-lang <value>` before full parsing, so that even parse errors are
/// shown in the requested language.
pub fn lang_override(args: &[Vec<u8>]) -> Option<Lang> {
    let i = args.iter().position(|a| a.eq_ignore_ascii_case(b"-lang"))?;
    args.get(i + 1).and_then(|v| i18n::parse(v))
}

/// Takes the value that must follow an option (`-p 2121`) or fails.
fn need<'a>(it: &mut core::slice::Iter<'a, Vec<u8>>, opt: &str) -> Result<&'a [u8], String> {
    it.next().map(|v| v.as_slice()).ok_or_else(|| tr!("{} needs a value", "{} requiere un valor", opt))
}

/// Parses the arguments (program name already removed).
/// Options are case-insensitive and may appear in any order; one non-option
/// argument is the folder to share.
pub fn parse(args: &[Vec<u8>]) -> ArgOutcome {
    match parse_inner(args) {
        Ok(o) => o,
        Err(e) => ArgOutcome::Error(e),
    }
}

/// Implementation of `parse`; errors are messages in the current language.
fn parse_inner(args: &[Vec<u8>]) -> Result<ArgOutcome, String> {
    let mut cfg = Config::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let a = arg.as_slice();
        let is_opt = a.len() > 1 && (a[0] == b'-' || (a[0] == b'/' && a.len() == 2 && a[1] == b'?'));
        if !is_opt {
            if cfg.root.is_some() {
                return Err(String::from(t("only one folder can be given", "solo se puede indicar una carpeta")));
            }
            // `"C:\Mis Archivos\"` arrives as `C:\Mis Archivos"` with MS quoting rules.
            let mut root = a.to_vec();
            while root.last() == Some(&b'"') {
                root.pop();
            }
            cfg.root = Some(root);
            continue;
        }
        let lower: Vec<u8> = a.iter().map(|c| c.to_ascii_lowercase()).collect();
        match lower.as_slice() {
            b"-p" => {
                let v = need(&mut it, "-p")?;
                cfg.port = parse_u16(v).filter(|&p| p != 0).ok_or(t("-p must be a port (1-65535)", "-p debe ser un puerto (1-65535)"))?;
            }
            b"-tp" => {
                let v = need(&mut it, "-tp")?;
                let (lo, hi) = match v.iter().position(|&c| c == b'-') {
                    Some(i) => (parse_u16(&v[..i]), parse_u16(&v[i + 1..])),
                    None => (parse_u16(v), parse_u16(v)),
                };
                match (lo, hi) {
                    (Some(lo), Some(hi)) if lo != 0 && lo <= hi => cfg.pasv_range = Some((lo, hi)),
                    _ => {
                        return Err(String::from(t(
                            "-tp must be a port or range, e.g. 50000-50010",
                            "-tp debe ser un puerto o rango, ej: 50000-50010",
                        )))
                    }
                }
            }
            b"-ha" => {
                let v = need(&mut it, "-ha")?;
                cfg.ext_ip = Some(parse_ipv4(v).ok_or(t("-ha must be an IP address", "-ha debe ser una direccion IP"))?);
            }
            b"-b" => {
                let v = need(&mut it, "-b")?;
                cfg.bind_ip = Some(parse_ipv4(v).ok_or(t("-b must be an IP address", "-b debe ser una direccion IP"))?);
            }
            b"-g" | b"-r" => cfg.read_only = true,
            b"-a" => cfg.show_hidden = true,
            b"-nolog" => cfg.log = false,
            b"-lang" => {
                let v = need(&mut it, "-lang")?;
                cfg.lang = Some(i18n::parse(v).ok_or(t("-lang must be en or es", "-lang debe ser en o es"))?);
            }
            b"-u" => {
                let v = need(&mut it, "-u")?;
                match v.iter().position(|&c| c == b':') {
                    Some(i) if i > 0 => cfg.login = Some((v[..i].to_vec(), v[i + 1..].to_vec())),
                    _ => return Err(String::from(t("-u must be user:password", "-u debe ser usuario:clave"))),
                }
            }
            b"-t" => {
                let v = need(&mut it, "-t")?;
                cfg.idle_timeout = parse_u64(v)
                    .filter(|&t| t <= 86_400)
                    .ok_or(t("-t must be seconds (0-86400)", "-t debe ser segundos (0-86400)"))? as u32;
            }
            b"-m" => {
                let v = need(&mut it, "-m")?;
                cfg.max_clients = parse_u64(v).filter(|&m| m <= 10_000).ok_or(t("-m must be a number", "-m debe ser un numero"))? as u32;
            }
            b"-h" | b"-?" | b"/?" | b"--help" => return Ok(ArgOutcome::Help),
            b"-v" | b"--version" => return Ok(ArgOutcome::Version),
            _ => return Err(tr!("unknown option '{}'", "opcion desconocida '{}'", String::from_utf8_lossy(a))),
        }
    }
    Ok(ArgOutcome::Run(cfg))
}

/// Splits a Windows command line (GetCommandLineA) following the MSVCRT rules,
/// dropping the program name.
#[cfg_attr(not(all(win32, not(test))), allow(dead_code))]
pub fn split_cmdline(line: &[u8]) -> Vec<Vec<u8>> {
    let mut args = Vec::new();
    let mut i = 0;
    // Program name: quotes toggle, no backslash escaping.
    let mut inq = false;
    while i < line.len() {
        let c = line[i];
        if c == b'"' {
            inq = !inq;
        } else if (c == b' ' || c == b'\t') && !inq {
            break;
        }
        i += 1;
    }
    loop {
        while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
            i += 1;
        }
        if i >= line.len() {
            break;
        }
        let mut cur = Vec::new();
        let mut inq = false;
        while i < line.len() {
            let c = line[i];
            if c == b'\\' {
                let start = i;
                while i < line.len() && line[i] == b'\\' {
                    i += 1;
                }
                let n = i - start;
                if i < line.len() && line[i] == b'"' {
                    cur.extend(core::iter::repeat_n(b'\\', n / 2));
                    if n % 2 == 1 {
                        cur.push(b'"');
                        i += 1;
                    }
                } else {
                    cur.extend(core::iter::repeat_n(b'\\', n));
                }
                continue;
            }
            if c == b'"' {
                if inq && i + 1 < line.len() && line[i + 1] == b'"' {
                    cur.push(b'"');
                    i += 2;
                    continue;
                }
                inq = !inq;
                i += 1;
                continue;
            }
            if (c == b' ' || c == b'\t') && !inq {
                break;
            }
            cur.push(c);
            i += 1;
        }
        args.push(cur);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<Vec<u8>> {
        v.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn defaults_for_double_click() {
        assert_eq!(parse(&[]), ArgOutcome::Run(Config::default()));
    }

    #[test]
    fn options() {
        let ArgOutcome::Run(c) = parse(&a(&["-p", "2121", "-tp", "50000-50010", "-ha", "1.2.3.4", "-g", "-u", "ana:x:y", "D:\\ftp"]))
        else {
            panic!()
        };
        assert_eq!(c.port, 2121);
        assert_eq!(c.pasv_range, Some((50000, 50010)));
        assert_eq!(c.ext_ip, Some([1, 2, 3, 4]));
        assert!(c.read_only);
        assert_eq!(c.login, Some((b"ana".to_vec(), b"x:y".to_vec())));
        assert_eq!(c.root, Some(b"D:\\ftp".to_vec()));
        let ArgOutcome::Run(c) = parse(&a(&["-tp", "6000"])) else { panic!() };
        assert_eq!(c.pasv_range, Some((6000, 6000)));
    }

    #[test]
    fn errors() {
        assert!(matches!(parse(&a(&["-p"])), ArgOutcome::Error(_)));
        assert!(matches!(parse(&a(&["-p", "70000"])), ArgOutcome::Error(_)));
        assert!(matches!(parse(&a(&["-tp", "10-5"])), ArgOutcome::Error(_)));
        assert!(matches!(parse(&a(&["-x"])), ArgOutcome::Error(_)));
        assert!(matches!(parse(&a(&["a", "b"])), ArgOutcome::Error(_)));
        assert!(matches!(parse(&a(&["-lang", "fr"])), ArgOutcome::Error(_)));
        let ArgOutcome::Run(c) = parse(&a(&["-LANG", "es"])) else { panic!() };
        assert_eq!(c.lang, Some(Lang::Es));
        assert_eq!(lang_override(&a(&["-p", "21", "-lang", "en"])), Some(Lang::En));
        assert_eq!(parse(&a(&["/?"])), ArgOutcome::Help);
    }

    #[test]
    fn cmdline_split() {
        assert_eq!(split_cmdline(b"\"C:\\Program Files\\x.exe\" -p 21"), a(&["-p", "21"]));
        assert_eq!(split_cmdline(b"x.exe \"C:\\Mis Docs\""), a(&["C:\\Mis Docs"]));
        assert_eq!(split_cmdline(b"x.exe \"C:\\Mis Docs\\\""), a(&["C:\\Mis Docs\""]));
        assert_eq!(split_cmdline(b"x.exe a\\\\b \"q\\\"x\""), a(&["a\\\\b", "q\"x"]));
        assert_eq!(split_cmdline(b"x.exe"), Vec::<Vec<u8>>::new());
        // trailing quote stripped from root
        let ArgOutcome::Run(c) = parse(&split_cmdline(b"x.exe \"C:\\Mis Docs\\\"")) else { panic!() };
        assert_eq!(c.root, Some(b"C:\\Mis Docs".to_vec()));
    }
}
