// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! Console + daily log file (`logs\AAAA-MM-DD.log`, a new file each day,
//! appended if the server is started several times the same day).

use crate::sys::{self, color, File, Lock, WriteMode};
use crate::timefmt::{from_filetime, hms, ymd};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicPtr, Ordering};

struct State {
    dir: Option<Vec<u8>>,
    day: String,
    file: Option<File>,
}

struct Logger {
    lock: Lock,
    state: UnsafeCell<State>,
}
unsafe impl Sync for Logger {}

static LOGGER: AtomicPtr<Logger> = AtomicPtr::new(core::ptr::null_mut());

/// `dir`: folder for the daily files, or None to log to the console only.
pub fn init(dir: Option<Vec<u8>>) {
    let l = Box::new(Logger { lock: Lock::new(), state: UnsafeCell::new(State { dir, day: String::new(), file: None }) });
    LOGGER.store(Box::into_raw(l), Ordering::Release);
}

/// Path of today's log file (for the startup message).
pub fn today_file(dir: &[u8]) -> Vec<u8> {
    let now = from_filetime(sys::now_local_ft());
    let mut p = dir.to_vec();
    if p.last() != Some(&sys::SEP) {
        p.push(sys::SEP);
    }
    p.extend_from_slice(ymd(&now).as_bytes());
    p.extend_from_slice(b".log");
    p
}

/// Writes one line with a time stamp to the console (white) and to the daily file.
pub fn line(text: &[u8]) {
    line_c(color::WHITE, text);
}

/// Writes one line with a time stamp to the console and to the daily file.
/// On the console the time stamp is dark gray and the text uses color `c`
/// (see `sys::color`); the log file is always plain text.
pub fn line_c(c: u8, text: &[u8]) {
    let now = from_filetime(sys::now_local_ft());
    let mut l = Vec::with_capacity(text.len() + 12);
    l.extend_from_slice(hms(&now).as_bytes());
    l.push(b' ');
    let stamp = l.len();
    l.extend_from_slice(text);
    l.extend_from_slice(b"\r\n");
    let print = || {
        sys::set_color(Some(color::DARK_GRAY));
        sys::out(&l[..stamp]);
        sys::set_color(Some(c));
        sys::out(&l[stamp..]);
        sys::set_color(None);
    };

    let ptr = LOGGER.load(Ordering::Acquire);
    if ptr.is_null() {
        print();
        return;
    }
    let logger = unsafe { &*ptr };
    logger.lock.with(|| {
        print();
        let st = unsafe { &mut *logger.state.get() };
        let Some(dir) = st.dir.as_ref() else { return };
        let day = ymd(&now);
        if day != st.day {
            // New day (or first line): switch to a new file.
            st.day = day;
            st.file = File::open_write(&today_file(dir), WriteMode::Append).ok();
        }
        if let Some(f) = st.file.as_mut() {
            if f.write_all(&l).is_err() {
                st.file = None;
            }
        }
    });
}

/// Plain console output without time stamp and without touching the log file.
pub fn console(text: &str) {
    sys::out(text.as_bytes());
}

/// Like `console`, in color `c` (see `sys::color`).
pub fn console_c(c: u8, text: &str) {
    sys::set_color(Some(c));
    sys::out(text.as_bytes());
    sys::set_color(None);
}
