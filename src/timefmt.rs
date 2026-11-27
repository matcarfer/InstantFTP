// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! Date handling without any OS or float support (works the same on Win98 and 11).
//! Times are Windows FILETIME values: 100 ns ticks since 1601-01-01.

use alloc::{format, string::String};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: u32,
    pub month: u32, // 1..=12
    pub day: u32,   // 1..=31
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Seconds between 1601-01-01 and 1970-01-01.
pub const EPOCH_DIFF_SECS: u64 = 11_644_473_600;

/// Converts a FILETIME (in whatever zone it was expressed) to a calendar date.
pub fn from_filetime(ft: u64) -> DateTime {
    let secs = ft / 10_000_000;
    let days = secs / 86_400;
    let rem = (secs % 86_400) as u32;
    // Days since 1601-01-01 -> days since 1970-01-01 (may be negative).
    let z = days as i64 - (EPOCH_DIFF_SECS / 86_400) as i64;
    let (y, m, d) = civil_from_days(z);
    DateTime { year: y.max(0) as u32, month: m, day: d, hour: rem / 3600, minute: (rem / 60) % 60, second: rem % 60 }
}

/// Howard Hinnant's algorithm, integer only.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// "20041207153000" for MDTM (RFC 3659, UTC).
pub fn mdtm(t: &DateTime) -> String {
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}

/// `ls -l` style date: "Dec  7 15:30" for the last ~6 months, "Dec  7  2004" otherwise.
pub fn ls_date(t: &DateTime, now: &DateTime) -> String {
    let mon = MONTHS[(t.month.clamp(1, 12) - 1) as usize];
    let months_ago = (now.year as i64 * 12 + now.month as i64) - (t.year as i64 * 12 + t.month as i64);
    if (0..6).contains(&months_ago) {
        format!("{} {:>2} {:02}:{:02}", mon, t.day, t.hour, t.minute)
    } else {
        format!("{} {:>2}  {}", mon, t.day, t.year)
    }
}

/// "2026-11-20" (daily log file name).
pub fn ymd(t: &DateTime) -> String {
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

/// "15:30:07" (log line prefix).
pub fn hms(t: &DateTime) -> String {
    format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ft(unix: u64) -> u64 {
        (unix + EPOCH_DIFF_SECS) * 10_000_000
    }

    #[test]
    fn conversions() {
        assert_eq!(from_filetime(ft(0)), DateTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0 });
        // 2004-06-07 12:34:56 UTC = 1086611696
        let t = from_filetime(ft(1_086_611_696));
        assert_eq!(mdtm(&t), "20040607123456");
        // leap day 2024-02-29 00:00:00 = 1709164800
        assert_eq!(mdtm(&from_filetime(ft(1_709_164_800))), "20240229000000");
        // before 1970 (1601 epoch itself)
        assert_eq!(from_filetime(0).year, 1601);
        assert_eq!(from_filetime(0).month, 1);
    }

    #[test]
    fn listing_dates() {
        let now = DateTime { year: 2026, month: 11, day: 20, hour: 8, minute: 0, second: 0 };
        let recent = DateTime { year: 2026, month: 9, day: 3, hour: 7, minute: 5, second: 0 };
        let old = DateTime { year: 2004, month: 6, day: 7, hour: 0, minute: 0, second: 0 };
        assert_eq!(ls_date(&recent, &now), "Sep  3 07:05");
        assert_eq!(ls_date(&old, &now), "Jun  7  2004");
        assert_eq!(ymd(&now), "2026-11-20");
        assert_eq!(hms(&recent), "07:05:00");
    }
}
