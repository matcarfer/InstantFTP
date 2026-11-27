// Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
//! # InstantFTP - instant, portable FTP server
//!
//! Copy InstantFTP.exe into any folder, double-click it, and that folder is
//! shared over FTP. A single dependency-free .exe that runs on Windows 98 SE
//! through Windows 11.
//!
//! Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer).
//! Free software under the GNU GPL v3 or later: see LICENSE and LICENSE.en.
//!
//! See README.md and ARCHITECTURE.md.

#![cfg_attr(all(win32, not(test)), no_std)]
#![cfg_attr(all(win32, not(test)), no_main)]

extern crate alloc;

mod args;
mod i18n;
mod listing;
mod log;
mod path;
mod session;
mod sys;
mod timefmt;
mod util;

use alloc::{boxed::Box, format, string::String, vec::Vec};
use args::ArgOutcome;
use core::sync::atomic::{AtomicU32, Ordering};
use session::Shared;
use sys::Socket;
use util::fmt_ipv4;

/// Version from Cargo.toml, shown in the banner, the 220 greeting and STAT.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(not(all(win32, not(test))))]
fn main() {
    std::process::exit(app_main());
}

/// Bytes -> printable string for console messages (invalid UTF-8 replaced).
fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Real entry point (called from `mainCRTStartup` on Windows).
pub fn app_main() -> i32 {
    // Language: -lang option, otherwise the language Windows is installed in.
    let argv = sys::args();
    i18n::set(args::lang_override(&argv).unwrap_or(if sys::system_spanish() { i18n::Lang::Es } else { i18n::Lang::En }));
    log::console(&tr!(
        "InstantFTP {} by Matias Ferrero (matcarfer) - GPL-3.0 - https://github.com/matcarfer/InstantFTP\r\n",
        "InstantFTP {} por Matias Ferrero (matcarfer) - GPL-3.0 - https://github.com/matcarfer/InstantFTP\r\n",
        VERSION
    ));

    let cfg = match args::parse(&argv) {
        ArgOutcome::Run(c) => c,
        ArgOutcome::Help => {
            log::console(&args::usage().replace('\n', "\r\n"));
            return 0;
        }
        ArgOutcome::Version => return 0,
        ArgOutcome::Error(e) => {
            log::console(&format!("Error: {}\r\n\r\n{}", e, args::usage().replace('\n', "\r\n")));
            return 1;
        }
    };

    // Shared folder: argument, or the folder where the .exe lives (double click).
    let exe_dir = sys::exe_dir();
    let wanted = cfg.root.clone().unwrap_or_else(|| exe_dir.clone());
    let root = match sys::full_path(&wanted) {
        Some(r) if sys::metadata(&r).map(|m| m.is_dir).unwrap_or(false) => r,
        _ => {
            log::console(&tr!(
                "Error: cannot share the folder '{}'\r\n",
                "Error: no se puede compartir la carpeta '{}'\r\n",
                lossy(&wanted)
            ));
            pause_if_double_clicked(&cfg);
            return 1;
        }
    };

    // Daily log in <exe folder>\logs, hidden and not reachable by FTP clients.
    let mut protected = None;
    let mut log_note = String::from(i18n::t("disabled", "desactivado"));
    if cfg.log {
        let logs = path::to_native(&exe_dir, b"/logs", sys::SEP).unwrap_or_else(|| b"logs".to_vec());
        let _ = sys::create_dir(&logs);
        if sys::metadata(&logs).map(|m| m.is_dir).unwrap_or(false) {
            sys::set_hidden(&logs);
            let logs_full = sys::full_path(&logs).unwrap_or(logs);
            protected = path::native_to_virtual(&root, &logs_full, sys::SEP);
            log_note = lossy(&log::today_file(&logs_full));
            log::init(Some(logs_full));
        } else {
            log_note = String::from(i18n::t(
                "could not create the logs folder (read-only folder?)",
                "no se pudo crear la carpeta logs (carpeta de solo lectura?)",
            ));
            log::init(None);
        }
    } else {
        log::init(None);
    }

    if !sys::net_init() {
        log::line(
            i18n::t(
                "Error: Winsock 2 is required (on Windows 95 install the 'Winsock 2 Update')",
                "Error: se necesita Winsock 2 (en Windows 95 instalar 'Winsock 2 Update')",
            )
            .as_bytes(),
        );
        pause_if_double_clicked(&cfg);
        return 1;
    }

    let bind_ip = cfg.bind_ip.unwrap_or([0; 4]);
    let mut port = cfg.port;
    let listener = match Socket::listen(bind_ip, port, 8) {
        Ok(l) => l,
        Err(_) if port == 21 => {
            // Port 21 busy (another server, or no permission): fall back to 2100.
            port = 2100;
            match Socket::listen(bind_ip, port, 8) {
                Ok(l) => l,
                Err(_) => return fail_port(&cfg, port),
            }
        }
        Err(_) => return fail_port(&cfg, port),
    };
    let port = listener.local_addr().map(|a| a.port).unwrap_or(port);

    let mut ips: Vec<[u8; 4]> = match (cfg.ext_ip, cfg.bind_ip) {
        (Some(ip), _) => alloc::vec![ip],
        (None, Some(ip)) => alloc::vec![ip],
        _ => sys::host_ips(),
    };
    if ips.iter().any(|ip| ip[0] != 127) {
        ips.retain(|ip| ip[0] != 127);
    }
    if ips.is_empty() {
        ips.push([127, 0, 0, 1]);
    }
    let url = |ip: [u8; 4]| {
        let mut u = format!("ftp://{}", lossy(&fmt_ipv4(ip)));
        if port != 21 {
            u.push_str(&format!(":{}", port));
        }
        u
    };

    log::line(tr!("=== InstantFTP {} started ===", "=== InstantFTP {} iniciado ===", VERSION).as_bytes());
    log::line(tr!("Sharing: {}", "Compartiendo: {}", lossy(&root)).as_bytes());
    let mode = if cfg.read_only { i18n::t("read only", "solo lectura") } else { i18n::t("read and write", "lectura y escritura") };
    let who = match &cfg.login {
        Some((u, _)) => tr!(", user '{}'", ", usuario '{}'", lossy(u)),
        None => String::from(i18n::t(", no user name (anonymous)", ", sin usuario (anonimo)")),
    };
    log::line(tr!("Access: {}{}", "Acceso: {}{}", mode, who).as_bytes());
    for ip in &ips {
        log::line(tr!("Ready! Connect to {}", "Listo! Conectate a {}", url(*ip)).as_bytes());
    }
    log::console(&tr!(
        "Log: {}\r\nClose this window (or press Ctrl+C) to stop the server.\r\n\r\n",
        "Log: {}\r\nCerra esta ventana (o Ctrl+C) para detener el servidor.\r\n\r\n",
        log_note
    ));
    sys::set_title(format!("InstantFTP - {} - {}", url(ips[0]), lossy(&root)).as_bytes());

    let shared: &'static Shared = Box::leak(Box::new(Shared {
        cfg,
        root,
        protected,
        clients: AtomicU32::new(0),
        next_id: AtomicU32::new(1),
        pasv_next: AtomicU32::new(0),
    }));

    loop {
        let (sock, peer) = match listener.accept(0) {
            Ok(x) => x,
            Err(_) => {
                // A transient error must not kill the server.
                sys::sleep_ms(100);
                continue;
            }
        };
        let max = shared.cfg.max_clients;
        if max != 0 && shared.clients.load(Ordering::Relaxed) >= max {
            let _ = sock.send_all(b"421 Too many connections, try again later\r\n");
            log::line(
                tr!("Rejected {}: too many clients ({})", "Rechazado {}: demasiados clientes ({})", lossy(&fmt_ipv4(peer.ip)), max)
                    .as_bytes(),
            );
            continue;
        }
        shared.clients.fetch_add(1, Ordering::Relaxed);
        let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
        let started = sys::spawn(Box::new(move || {
            session::run(shared, sock, peer, id);
            shared.clients.fetch_sub(1, Ordering::Relaxed);
        }));
        if !started {
            shared.clients.fetch_sub(1, Ordering::Relaxed);
            log::line(
                i18n::t("Error: could not create a thread for the client", "Error: no se pudo crear el hilo para el cliente").as_bytes(),
            );
        }
    }
}

/// Reports that the control port could not be opened and returns exit code 1.
fn fail_port(cfg: &args::Config, port: u16) -> i32 {
    log::line(
        tr!(
            "Error: could not open port {} (already in use?). Try -p 2121",
            "Error: no se pudo abrir el puerto {} (ya esta en uso?). Proba con -p 2121",
            port
        )
        .as_bytes(),
    );
    pause_if_double_clicked(cfg);
    1
}

/// Keeps the window open for a few seconds so an error is readable when the
/// program was started with a double click.
fn pause_if_double_clicked(_cfg: &args::Config) {
    #[cfg(windows)]
    sys::sleep_ms(10_000);
}
