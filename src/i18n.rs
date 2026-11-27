// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! # User interface language (English by default, Spanish as second language)
//!
//! Only console / log text is translated. FTP protocol replies stay in English
//! (clients parse them, and the RFCs define them in English).
//!
//! The language is chosen once at start-up:
//! 1. `-lang en` / `-lang es` on the command line, if present;
//! 2. otherwise Spanish when Windows is installed in Spanish
//!    (`GetUserDefaultLangID`, available since Windows 95);
//! 3. otherwise English.
//!
//! Adding a language: add a variant to [`Lang`], extend [`tr!`] and every
//! call site will fail to compile until it has the new text (by design).
//! Console strings must stay ASCII: the Windows 9x console uses an OEM code
//! page, so accents would be garbled.

use core::sync::atomic::{AtomicU8, Ordering};

/// Supported user interface languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

/// Current language, stored as a byte so every thread can read it lock-free.
static LANG: AtomicU8 = AtomicU8::new(0);

/// Sets the language for the whole process (call once at start-up).
pub fn set(l: Lang) {
    LANG.store(if l == Lang::Es { 1 } else { 0 }, Ordering::Relaxed);
}

/// Current language.
pub fn get() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::Es
    } else {
        Lang::En
    }
}

/// Parses a `-lang` value ("en", "es", "english", "espanol", case-insensitive).
pub fn parse(v: &[u8]) -> Option<Lang> {
    let l: alloc::vec::Vec<u8> = v.iter().map(|c| c.to_ascii_lowercase()).collect();
    match l.as_slice() {
        b"en" | b"eng" | b"english" => Some(Lang::En),
        b"es" | b"spa" | b"espanol" | b"castellano" => Some(Lang::Es),
        _ => None,
    }
}

/// Picks the text for the current language: `t("Hello", "Hola")`.
pub fn t(en: &'static str, es: &'static str) -> &'static str {
    match get() {
        Lang::En => en,
        Lang::Es => es,
    }
}

/// `format!` with one format string per language:
/// `tr!("Sharing: {}", "Compartiendo: {}", path)`.
#[macro_export]
macro_rules! tr {
    ($en:literal, $es:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::get() {
            $crate::i18n::Lang::En => alloc::format!($en $(, $arg)*),
            $crate::i18n::Lang::Es => alloc::format!($es $(, $arg)*),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_and_switching() {
        assert_eq!(parse(b"ES"), Some(Lang::Es));
        assert_eq!(parse(b"english"), Some(Lang::En));
        assert_eq!(parse(b"fr"), None);
        set(Lang::Es);
        assert_eq!(t("Hello", "Hola"), "Hola");
        assert_eq!(tr!("{} files", "{} archivos", 3), "3 archivos");
        set(Lang::En);
        assert_eq!(tr!("{} files", "{} archivos", 3), "3 files");
    }
}
