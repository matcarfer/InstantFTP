<h1 align="center">InstantFTP</h1>

<p align="center">
  <b>v1.2.0</b> · 2026-11-22 · Windows 98 SE → 11 · 54 KB<br>
  <a href="https://github.com/matcarfer/InstantFTP/releases/download/v1.2.0/InstantFTP_v1.2.exe"><b>⬇ Download · Descargar InstantFTP_v1.2.exe</b></a><br>
  <sub>SHA-256 <code>19EEF05244951F05DFA8FC61CE7AF322D4D9F58C2D02F983D3EC58812012AB5E</code> · <a href="https://www.virustotal.com/gui/file/19eef05244951f05dfa8fc61ce7af322d4d9f58c2d02f983d3ec58812012ab5e">VirusTotal</a></sub>
</p>

<p align="center">
  <a href="https://cafecito.app/matcarfer">☕ Cafecito</a> · <a href="https://ko-fi.com/matcarfer">☕ Ko-fi</a> · <a href="https://www.patreon.com/matcarfer">❤ Patreon</a> · <a href="https://github.com/matcarfer/library">📚 All my tools · Todas mis herramientas</a>
</p>

## English

**Copy `InstantFTP.exe` into a folder, double-click it, and that folder is shared over FTP.**
One small .exe: no installer, no DLLs, no runtime. The window shows the address to
connect to (e.g. `ftp://192.168.0.15`); use Explorer, FileZilla, WinSCP, a phone app…
Close the window to stop it. A daily log is saved in a hidden `logs` folder that clients can't reach.

| Option | What it does |
|---|---|
| `-g` / `-r` | Read only |
| `-u <user:pass>` | Require user and password (default: anyone) |
| `-p <port>` | Port (default 21, or 2100 if busy) |
| `-tp <p>-<p2>` | Passive port range, for firewalls |
| `-ha <ip>` | Public IP for passive mode (NAT) |
| `-b <ip>` | Listen on one local IP only |
| `-t <sec>` / `-m <n>` | Idle timeout (300) / max clients (20) |
| `-a` · `-nolog` · `-lang en\|es` | Show hidden files · no log file · console language |
| `folder` | Share another folder |

Use options from a shortcut or a `.bat`: `InstantFTP.exe -g -u guest:1234`.
A few antivirus engines may flag it by heuristics (tiny .exe with no runtime): see the VirusTotal report. Not code-signed: if SmartScreen appears, click **More info → Run anyway**.

## Español

**Copiá `InstantFTP.exe` en una carpeta, hacé doble clic y esa carpeta queda compartida por FTP.**
Un solo .exe chico: sin instalador, sin DLLs, sin runtime. La ventana muestra la dirección
para conectarse (ej. `ftp://192.168.0.15`); usá el Explorador, FileZilla, WinSCP, una app del celular…
Cerrá la ventana para detenerlo. Guarda un log diario en una carpeta `logs` oculta, inaccesible por FTP.

Opciones: `-g` solo lectura · `-u usuario:clave` · `-p puerto` · `-tp` rango pasivo ·
`-ha` IP pública · `-b` IP local · `-t` / `-m` inactividad / máx. clientes · `-a` archivos ocultos ·
`-nolog` · `-lang es` · `carpeta`. Ejemplo: `InstantFTP.exe -g -u invitado:1234`.
Algunos antivirus pueden marcarlo por heurística (.exe muy chico, sin runtime): mirá el informe de VirusTotal. No está firmado: si aparece "Windows protegió tu PC", elegí **Más información → Ejecutar de todas formas**.

---

<sub>
<b>EN:</b> Freeware, closed source. Provided as is, without warranty; use at your own risk. Do not redistribute: link to this page instead. Donations are voluntary. See <a href="LICENSE">LICENSE</a>.<br>
<b>ES:</b> Gratis, código cerrado. Se ofrece tal cual, sin garantía; usalo bajo tu responsabilidad. No lo redistribuyas: compartí el link a esta página. Las donaciones son voluntarias. Ver <a href="LICENSE">LICENSE</a>.
</sub>
