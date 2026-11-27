// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! `LIST` line format (Unix `ls -l` style, understood by every FTP client).

use crate::sys::Meta;
use crate::timefmt::{ls_date, DateTime};
use alloc::{format, vec::Vec};

/// One long listing line, CRLF terminated.
/// Folders get `drwxrwxrwx` (with `x`): without it some
/// clients refuse to enter them; folders now get proper `x` bits, sizes are
/// 64-bit (files > 4 GB are fine) and recent files show the time.
pub fn long_line(name: &[u8], m: &Meta, mtime_local: &DateTime, now_local: &DateTime) -> Vec<u8> {
    let perms: &str = match (m.is_dir, m.readonly) {
        (true, false) => "drwxrwxrwx",
        (true, true) => "dr-xr-xr-x",
        (false, false) => "-rw-rw-rw-",
        (false, true) => "-r--r--r--",
    };
    let mut line = format!("{} 1 ftp ftp {:>13} {} ", perms, m.size, ls_date(mtime_local, now_local)).into_bytes();
    line.extend_from_slice(name);
    line.extend_from_slice(b"\r\n");
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        let now = DateTime { year: 2026, month: 11, day: 20, hour: 8, minute: 0, second: 0 };
        let old = DateTime { year: 2004, month: 6, day: 7, hour: 1, minute: 2, second: 3 };
        let f = Meta { size: 5_000_000_000, ..Meta::default() };
        assert_eq!(long_line(b"big.iso", &f, &old, &now), b"-rw-rw-rw- 1 ftp ftp    5000000000 Jun  7  2004 big.iso\r\n".to_vec());
        let d = Meta { is_dir: true, readonly: true, ..Meta::default() };
        assert_eq!(long_line(b"Mis Fotos", &d, &now, &now), b"dr-xr-xr-x 1 ftp ftp             0 Nov 20 08:00 Mis Fotos\r\n".to_vec());
    }
}
