# Arquitectura de InstantFTP

[English](ARCHITECTURE.md) | **Español**

Este documento explica cómo está armado el programa, para que cualquier persona
o IA pueda orientarse en 5 minutos. Los comentarios del código están en inglés;
la documentación, en inglés (principal) y castellano.

## Mapa de archivos

```
InstantFTP/
├── src/
│   ├── main.rs        Arranque: opciones, carpeta compartida, log, puerto, bucle accept()
│   ├── session.rs     Una conexión FTP: lectura de comandos, despacho, transferencias
│   ├── path.rs        Rutas virtuales y validación (la barrera de seguridad)
│   ├── args.rs        Opciones de línea de comandos + partición de la línea estilo MSVCRT
│   ├── i18n.rs        Idioma de la consola: inglés por defecto, castellano
│   ├── listing.rs     Formato de líneas LIST (estilo `ls -l`)
│   ├── timefmt.rs     Fechas sin float ni sistema operativo (FILETIME -> calendario)
│   ├── log.rs         Consola + log diario logs\AAAA-MM-DD.log
│   ├── util.rs        Ayudantes para cadenas de bytes (números, IPs, comparación)
│   └── sys/
│       ├── mod.rs     Contrato de la capa de sistema (tabla de funciones)
│       ├── win32.rs   Backend real: Win32 + Winsock 2, no_std, compatible con Win98
│       └── hosted.rs  Backend de pruebas: Rust std (Linux / macOS / Windows)
├── tests/
│   ├── e2e.py         67 pruebas con un cliente FTP real (ftplib)
│   └── check_win98.py Verifica imports, subsistema PE e instrucciones del .exe
├── i586-win9x-windows-gnu.json   Target de Rust: i586 sin SSE, soft-float
├── .cargo/config.toml            Alias `cargo win` y flags del linker
├── build.rs                      Elige el backend de `sys`
```

## Flujo de una conexión

```
main.rs                         session.rs
───────                         ──────────
Socket::listen(21) ──accept()──▶ run()
  (un hilo por cliente)            ├─ "220 InstantFTP ready"
                                   └─ loop: read_line() ─▶ dispatch()
                                                              ├─ USER/PASS/SYST/FEAT...
                                                              ├─ CWD/PWD/MKD/RMD/DELE/RNFR
                                                              │     └─ resolve() ─▶ path.rs
                                                              ├─ PASV/EPSV/PORT/EPRT
                                                              └─ LIST/RETR/STOR/APPE
                                                                    └─ open_data() ─▶ conexión de datos
```

## Las tres ideas clave

### 1. Rutas virtuales (seguridad)

Cada sesión guarda su directorio actual como ruta **virtual** (`/fotos/2004`),
nunca usa el directorio actual del proceso. Toda ruta que manda un cliente pasa
por `Session::resolve` → `path::resolve`, que:

- normaliza `.` y `..` sin poder subir por encima de `/`;
- rechaza `:` (letras de unidad, *alternate data streams*), caracteres inválidos,
  comodines, nombres de dispositivo (`CON`, `NUL`, `COM1`...) y componentes de
  puntos/espacios (`...`, `.. `);
- bloquea la carpeta `logs` protegida;
- recién ahí la convierte en ruta nativa debajo de la carpeta compartida.

**Regla de oro**: ningún comando nuevo puede tocar el disco con una ruta del
cliente sin pasar por `resolve`.

### 2. Capa de sistema intercambiable

`session.rs`, `main.rs` y `log.rs` solo llaman a `crate::sys::*`. Hay dos
implementaciones con la misma API (ver la tabla en `src/sys/mod.rs`):

- `win32.rs`: llamadas directas a `KERNEL32.dll` y `WS2_32.dll`, sin C runtime.
  Es lo que va en el .exe.
- `hosted.rs`: `std` de Rust. Permite correr el servidor completo en Linux y
  probarlo con clientes reales.

`build.rs` define `cfg(win32)` al compilar para Windows; el código usa
`cfg(all(win32, not(test)))`, así `cargo test` siempre usa el backend `std`.

### 3. Compatibilidad Windows 98 SE → 11

| Problema | Solución |
|---|---|
| La `std` moderna de Rust exige Windows 7/10 | `#![no_std]` + `alloc`, Win32 a mano |
| Pentium/PII/PIII no tienen SSE2 | target propio `i586`, ABI soft-float, sin MMX/SSE |
| Win9x rechaza ejecutables con subsistema > 4.0 | flags del linker `--subsystem console:4.0` |
| Win9x no tiene APIs Unicode (`...W`) | solo funciones `...A` |
| `CreateThread` con id NULL falla en Win9x | siempre se pasa un puntero |
| `FILE_SHARE_DELETE` no existe en Win9x | no se usa |
| `SetFilePointerEx` no existe en Win9x | `SetFilePointer` con dword alto |
| Sin C runtime (`msvcrt` cambia entre versiones) | asignador sobre `HeapAlloc`, entry `mainCRTStartup` propio |

`tests/check_win98.py` hace cumplir esto en el CI: si alguien usa una función
que no existe en Windows 98, el build falla.

## Idioma

Los textos de consola y log pasan por `i18n::t("English", "Castellano")` o la macro
`tr!("fmt en", "fmt es", args)`. El idioma se elige al arrancar: `-lang`, si no el de
Windows (`GetUserDefaultLangID`), si no inglés. Las respuestas FTP no se traducen.
Los textos de consola van en ASCII (la consola de Win9x usa página de códigos OEM).

## Tamaño del ejecutable (~53 KB)

Se logra con `opt-level = "z"`, LTO, `panic = immediate-abort`, y sobre todo con
`function-sections` en el target, que permite al linker descartar el código no
usado de `compiler_builtins`. Si un cambio hace crecer mucho el .exe,
probablemente arrastró `core::fmt` con formatos nuevos o floats: preferir
concatenar bytes con `util::cat`.
