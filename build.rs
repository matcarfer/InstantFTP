// Copyright (c) 2026 matcarfer. All rights reserved. Proprietary, not for distribution.
//! Build script: decides which OS backend `src/sys` compiles.
//!
//! * Target OS Windows (the real .exe)            -> `cfg(win32)`: `no_std` + raw Win32.
//! * Anything else, or `--features hosted`        -> the `std` test backend.
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
}
