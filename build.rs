// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! Build script: decides which OS backend `src/sys` compiles.
//!
//! * Target OS Windows (the real .exe)            -> `cfg(win32)`: `no_std` + raw Win32.
//! * Anything else, or `--features hosted`        -> the `std` test backend.
//!
//! * The Windows 9x target also gets the program icon (`assets/icon.rc`).
//!
//! `cargo test` always uses the `std` backend (the code checks
//! `all(win32, not(test))`), so tests run natively on Linux, macOS and Windows.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(win32)");
    println!("cargo::rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let hosted = std::env::var_os("CARGO_FEATURE_HOSTED").is_some();
    if windows && !hosted {
        println!("cargo::rustc-cfg=win32");
    }
    icon();
}

/// Compiles `assets/icon.rc` with MinGW's windres and links it, only for the real Windows 9x target.
fn icon() {
    println!("cargo::rerun-if-changed=assets");
    if !std::env::var("TARGET").is_ok_and(|t| t.contains("win9x")) {
        return;
    }
    let obj = format!("{}/icon.o", std::env::var("OUT_DIR").unwrap());
    let ok = std::process::Command::new("i686-w64-mingw32-windres")
        .args(["-I", "assets", "assets/icon.rc", "-O", "coff", "-o", &obj])
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "i686-w64-mingw32-windres failed (binutils-mingw-w64-i686)");
    println!("cargo::rustc-link-arg-bins={obj}");
}
