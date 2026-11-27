// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! Small byte-string helpers (file names on Windows 9x are ANSI bytes, not UTF-8,
//! so the whole server works on `[u8]` instead of `str`).

use alloc::vec::Vec;

/// ASCII case-insensitive comparison.
pub fn eq_ic(a: &[u8], b: &[u8]) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Case-insensitive ordering used to sort directory listings.
pub fn cmp_ic(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let la = a.iter().map(|c| c.to_ascii_lowercase());
    let lb = b.iter().map(|c| c.to_ascii_lowercase());
    la.cmp(lb)
}

/// Removes leading and trailing spaces and tabs.
pub fn trim(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if *first == b' ' || *first == b'\t' {
            s = rest
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if *last == b' ' || *last == b'\t' {
            s = rest
        } else {
            break;
        }
    }
    s
}

/// Parses an unsigned decimal number. Rejects empty input, signs and overflow.
pub fn parse_u64(s: &[u8]) -> Option<u64> {
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    let mut v: u64 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u64)?;
    }
    Some(v)
}

/// Decimal 0..=65535 (ports).
pub fn parse_u16(s: &[u8]) -> Option<u16> {
    parse_u64(s).and_then(|v| if v <= 0xffff { Some(v as u16) } else { None })
}

/// Decimal 0..=255 (PORT command bytes, IPv4 octets).
pub fn parse_u8(s: &[u8]) -> Option<u8> {
    parse_u64(s).and_then(|v| if v <= 0xff { Some(v as u8) } else { None })
}

/// Strict dotted-quad IPv4 parser ("192.168.0.1").
pub fn parse_ipv4(s: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split(|&c| c == b'.') {
        if n == 4 {
            return None;
        }
        out[n] = parse_u8(part)?;
        n += 1;
    }
    if n == 4 {
        Some(out)
    } else {
        None
    }
}

/// `[192,168,0,1]` -> `b"192.168.0.1"`.
pub fn fmt_ipv4(ip: [u8; 4]) -> Vec<u8> {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]).into_bytes()
}

/// Concatenates byte slices.
pub fn cat(parts: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        v.extend_from_slice(p);
    }
    v
}

/// Position of the first `needle` byte.
pub fn find(hay: &[u8], needle: u8) -> Option<usize> {
    hay.iter().position(|&c| c == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_u64(b"0"), Some(0));
        assert_eq!(parse_u64(b"18446744073709551615"), Some(u64::MAX));
        assert_eq!(parse_u64(b"18446744073709551616"), None);
        assert_eq!(parse_u64(b"-1"), None);
        assert_eq!(parse_u64(b""), None);
        assert_eq!(parse_u16(b"65535"), Some(65535));
        assert_eq!(parse_u16(b"65536"), None);
    }

    #[test]
    fn ipv4() {
        assert_eq!(parse_ipv4(b"192.168.1.10"), Some([192, 168, 1, 10]));
        assert_eq!(parse_ipv4(b"1.2.3"), None);
        assert_eq!(parse_ipv4(b"1.2.3.4.5"), None);
        assert_eq!(parse_ipv4(b"256.1.1.1"), None);
        assert_eq!(fmt_ipv4([10, 0, 0, 1]), b"10.0.0.1".to_vec());
    }

    #[test]
    fn trimming() {
        assert_eq!(trim(b"  a b \t"), b"a b");
        assert_eq!(trim(b"   "), b"");
    }
}
