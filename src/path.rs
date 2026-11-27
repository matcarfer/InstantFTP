// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! Virtual path handling.
//!
//! A naive server would use the *process-wide* current directory (`chdir`) for
//! every session and only rejected path components made of dots.  That allowed
//! escaping the shared folder with drive letters (`RETR c:\boot.ini`), with a
//! root prefix collision (`C:\ftp` vs `C:\ftp2`) and broke when two clients were
//! connected at once.  Here every session keeps its own *virtual* directory
//! ("/", "/fotos/2004", ...) and every path is validated component by component
//! before being mapped under the shared root.

use crate::util::eq_ic;
use alloc::vec::Vec;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PathError {
    /// Bad characters, device names, too long, wildcards...
    Invalid,
    /// `..` would leave the shared folder.
    Escapes,
}

/// Longest path the Win32 ANSI APIs accept (MAX_PATH minus the terminator).
pub const MAX_NATIVE: usize = 259;

const DEVICES: &[&[u8]] = &[
    b"CON", b"PRN", b"AUX", b"NUL", b"CLOCK$", b"CONIN$", b"CONOUT$", b"COM1", b"COM2", b"COM3", b"COM4", b"COM5", b"COM6", b"COM7",
    b"COM8", b"COM9", b"LPT1", b"LPT2", b"LPT3", b"LPT4", b"LPT5", b"LPT6", b"LPT7", b"LPT8", b"LPT9",
];

/// True if a single file name component is safe to hand to the file system.
pub fn valid_component(c: &[u8], allow_wildcards: bool) -> bool {
    if c.is_empty() || c.len() > 255 {
        return false;
    }
    // Only dots/spaces: "...", ". .", ".. " -> Windows 9x maps "..." to "..\..",
    // and trailing dots/spaces are stripped, so ".. " would become "..".
    if c.iter().all(|&b| b == b'.' || b == b' ') {
        return false;
    }
    for &b in c {
        match b {
            0..=31 | b'<' | b'>' | b':' | b'"' | b'|' | b'/' | b'\\' => return false,
            b'*' | b'?' if !allow_wildcards => return false,
            _ => {}
        }
    }
    // Reserved DOS device names (also with an extension: "nul.txt", "con .x").
    // Opening these on Windows 98 can hang or crash the machine ("con\con").
    let base_end = c.iter().position(|&b| b == b'.').unwrap_or(c.len());
    let mut base = &c[..base_end];
    while let [rest @ .., b' '] = base {
        base = rest;
    }
    !DEVICES.iter().any(|d| eq_ic(base, d))
}

/// Splits a path argument into components on both `/` and `\`.
fn components(p: &[u8]) -> impl Iterator<Item = &[u8]> {
    p.split(|&c| c == b'/' || c == b'\\')
}

/// Resolves `arg` (absolute or relative to `cwd`) into a normalized virtual path.
/// Result always starts with '/', never ends with '/' (except the root itself).
pub fn resolve(cwd: &[u8], arg: &[u8]) -> Result<Vec<u8>, PathError> {
    resolve_inner(cwd, arg, false)
}

/// Same as `resolve` but the *last* component may contain `*` / `?`.
pub fn resolve_pattern(cwd: &[u8], arg: &[u8]) -> Result<Vec<u8>, PathError> {
    resolve_inner(cwd, arg, true)
}

/// Shared implementation of `resolve` / `resolve_pattern`.
/// Walks the components of `arg` starting from `cwd` (or from the root if `arg`
/// is absolute), applying `.` and `..` and validating every other component.
fn resolve_inner(cwd: &[u8], arg: &[u8], wildcard_last: bool) -> Result<Vec<u8>, PathError> {
    let mut parts: Vec<&[u8]> = Vec::new();
    let absolute = matches!(arg.first(), Some(b'/') | Some(b'\\'));
    if !absolute {
        for c in components(cwd) {
            if !c.is_empty() {
                parts.push(c);
            }
        }
    }
    let comps: Vec<&[u8]> = components(arg).collect();
    let last_idx = comps.iter().rposition(|c| !c.is_empty() && *c != b".");
    for (i, c) in comps.iter().enumerate() {
        match *c {
            b"" | b"." => {}
            b".." => {
                if parts.pop().is_none() {
                    return Err(PathError::Escapes);
                }
            }
            c => {
                let wild = wildcard_last && Some(i) == last_idx;
                if !valid_component(c, wild) {
                    return Err(PathError::Invalid);
                }
                parts.push(c);
            }
        }
    }
    let mut out = Vec::with_capacity(arg.len() + cwd.len() + 1);
    if parts.is_empty() {
        out.push(b'/');
    }
    for p in parts {
        out.push(b'/');
        out.extend_from_slice(p);
    }
    if out.len() > MAX_NATIVE {
        return Err(PathError::Invalid);
    }
    Ok(out)
}

/// Maps a virtual path onto the native file system under `root`.
/// `root` is a native directory without trailing separator, except for drive
/// roots like `C:\` (or `/` on the test backend).
pub fn to_native(root: &[u8], vpath: &[u8], sep: u8) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(root.len() + vpath.len());
    out.extend_from_slice(root);
    let rel = vpath.strip_prefix(b"/").unwrap_or(vpath);
    if !rel.is_empty() {
        if out.last() != Some(&sep) {
            out.push(sep);
        }
        out.extend(rel.iter().map(|&c| if c == b'/' { sep } else { c }));
    }
    if out.len() > MAX_NATIVE {
        None
    } else {
        Some(out)
    }
}

/// Splits "/a/b/c" into ("/a/b", "c"). Root returns ("/", "").
pub fn split_last(vpath: &[u8]) -> (&[u8], &[u8]) {
    match vpath.iter().rposition(|&c| c == b'/') {
        Some(0) => (b"/", &vpath[1..]),
        Some(i) => (&vpath[..i], &vpath[i + 1..]),
        None => (b"/", vpath),
    }
}

/// True if `vpath` is `prefix` or lives below it (case-insensitive, like Windows).
pub fn is_within(vpath: &[u8], prefix: &[u8]) -> bool {
    if prefix == b"/" {
        return true;
    }
    if vpath.len() < prefix.len() || !eq_ic(&vpath[..prefix.len()], prefix) {
        return false;
    }
    vpath.len() == prefix.len() || vpath[prefix.len()] == b'/'
}

/// Inverse of `to_native`: if `native` is inside `root`, returns its virtual path.
pub fn native_to_virtual(root: &[u8], native: &[u8], sep: u8) -> Option<Vec<u8>> {
    let mut r = root;
    if r.last() == Some(&sep) {
        r = &r[..r.len() - 1];
    }
    if native.len() <= r.len() || !eq_ic(&native[..r.len()], r) || native[r.len()] != sep {
        return None;
    }
    let rest = &native[r.len()..];
    Some(rest.iter().map(|&c| if c == sep { b'/' } else { c }).collect())
}

/// True if a name contains `*` or `?` (used by LIST to detect patterns).
pub fn has_wildcard(c: &[u8]) -> bool {
    c.iter().any(|&b| b == b'*' || b == b'?')
}

/// Case-insensitive glob match supporting `*` and `?`.
pub fn wildcard_match(pat: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while n < name.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p].eq_ignore_ascii_case(&name[n])) {
            p += 1;
            n += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = p;
            mark = n;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

/// Quotes a path for a 257 reply, doubling embedded quotes (RFC 959).
pub fn quote(vpath: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vpath.len() + 2);
    out.push(b'"');
    for &c in vpath {
        if c == b'"' {
            out.push(b'"');
        }
        out.push(c);
    }
    out.push(b'"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(cwd: &str, a: &str) -> Result<String, PathError> {
        resolve(cwd.as_bytes(), a.as_bytes()).map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn basic_resolution() {
        assert_eq!(r("/", "").unwrap(), "/");
        assert_eq!(r("/", "a").unwrap(), "/a");
        assert_eq!(r("/a", "b/c").unwrap(), "/a/b/c");
        assert_eq!(r("/a", "/b").unwrap(), "/b");
        assert_eq!(r("/a", "\\b\\c").unwrap(), "/b/c");
        assert_eq!(r("/a/b", "..").unwrap(), "/a");
        assert_eq!(r("/a/b", "../..").unwrap(), "/");
        assert_eq!(r("/a", "./b/./c/").unwrap(), "/a/b/c");
        assert_eq!(r("/", "//x//y").unwrap(), "/x/y");
    }

    #[test]
    fn escapes_are_blocked() {
        assert_eq!(r("/", ".."), Err(PathError::Escapes));
        assert_eq!(r("/a", "../.."), Err(PathError::Escapes));
        assert_eq!(r("/", "/../windows"), Err(PathError::Escapes));
        // classic escape tricks: drive letters, ADS, "..." (Win9x = ..\..)
        assert_eq!(r("/", "c:\\boot.ini"), Err(PathError::Invalid));
        assert_eq!(r("/", "c:boot.ini"), Err(PathError::Invalid));
        assert_eq!(r("/", "file.txt:stream"), Err(PathError::Invalid));
        assert_eq!(r("/", "..."), Err(PathError::Invalid));
        assert_eq!(r("/", ".. "), Err(PathError::Invalid));
        assert_eq!(r("/", "a/. ./b"), Err(PathError::Invalid));
    }

    #[test]
    fn devices_and_chars() {
        assert_eq!(r("/", "con"), Err(PathError::Invalid));
        assert_eq!(r("/", "x/NUL.txt"), Err(PathError::Invalid));
        assert_eq!(r("/", "com1 .log"), Err(PathError::Invalid));
        assert_eq!(r("/", "console.txt").unwrap(), "/console.txt");
        assert_eq!(r("/", "a*b"), Err(PathError::Invalid));
        assert_eq!(r("/", "a|b"), Err(PathError::Invalid));
        assert!(resolve_pattern(b"/", b"dir/*.txt").is_ok());
        assert!(resolve_pattern(b"/", b"d*r/x.txt").is_err());
        let long = "a".repeat(300);
        assert_eq!(r("/", &long), Err(PathError::Invalid));
    }

    #[test]
    fn native_mapping() {
        assert_eq!(to_native(b"C:\\", b"/", b'\\').unwrap(), b"C:\\".to_vec());
        assert_eq!(to_native(b"C:\\", b"/a/b", b'\\').unwrap(), b"C:\\a\\b".to_vec());
        assert_eq!(to_native(b"D:\\ftp", b"/", b'\\').unwrap(), b"D:\\ftp".to_vec());
        assert_eq!(to_native(b"D:\\ftp", b"/x", b'\\').unwrap(), b"D:\\ftp\\x".to_vec());
        assert_eq!(native_to_virtual(b"D:\\ftp", b"D:\\FTP\\logs", b'\\').unwrap(), b"/logs".to_vec());
        assert_eq!(native_to_virtual(b"C:\\", b"C:\\logs", b'\\').unwrap(), b"/logs".to_vec());
        assert_eq!(native_to_virtual(b"D:\\ftp", b"D:\\ftp2\\logs", b'\\'), None);
    }

    #[test]
    fn within_and_split() {
        assert!(is_within(b"/logs", b"/logs"));
        assert!(is_within(b"/LOGS/x.log", b"/logs"));
        assert!(!is_within(b"/logs2", b"/logs"));
        assert!(is_within(b"/anything", b"/"));
        assert_eq!(split_last(b"/a/b"), (&b"/a"[..], &b"b"[..]));
        assert_eq!(split_last(b"/a"), (&b"/"[..], &b"a"[..]));
    }

    #[test]
    fn globbing() {
        assert!(wildcard_match(b"*.txt", b"Notes.TXT"));
        assert!(wildcard_match(b"a?c", b"abc"));
        assert!(!wildcard_match(b"a?c", b"ac"));
        assert!(wildcard_match(b"*", b""));
        assert!(wildcard_match(b"*a*b*", b"xxaYYbzz"));
        assert!(!wildcard_match(b"*.txt", b"x.txt.bak"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote(b"/say \"hi\""), b"\"/say \"\"hi\"\"\"".to_vec());
    }
}
