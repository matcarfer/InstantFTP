#!/usr/bin/env python3
# Copyright (C) 2026 Matías Ferrero (GitHub: matcarfer). SPDX-License-Identifier: GPL-3.0-or-later
"""End-to-end test: runs the server binary in a temp folder (like a double
click: no arguments, shares its own folder) and drives it with ftplib.

    cargo build && python3 tests/e2e.py target/debug/InstantFTP

Works against the Windows .exe too (run it by hand, then use --connect HOST PORT).
"""
import ftplib, io, os, shutil, socket, subprocess, sys, tempfile, threading, time

PASS = 0


def ok(cond, msg):
    global PASS
    if not cond:
        raise AssertionError(msg)
    PASS += 1
    print("  ok -", msg)


def expect_err(fn, code, msg):
    try:
        fn()
    except ftplib.all_errors as e:
        ok(str(e).startswith(code), f"{msg} -> {str(e)[:60]}")
        return
    raise AssertionError(f"{msg}: expected error {code}")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start(binary, folder, args):
    exe = os.path.join(folder, os.path.basename(binary))
    shutil.copy(binary, exe)
    port = free_port()
    # Server output goes to a file, never to an unread pipe: on Windows a pipe
    # buffer is only a few KB and the server would block writing its log.
    out = open(os.path.join(tempfile.gettempdir(), f"instantftp-e2e-{port}.txt"), "wb")
    env = dict(os.environ, LANG="C", LC_ALL="C")  # English unless -lang is given
    p = subprocess.Popen([exe, "-p", str(port)] + args, cwd=os.path.dirname(exe), stdout=out, stderr=subprocess.STDOUT, env=env)
    p.out_file = out
    for _ in range(100):
        try:
            socket.create_connection(("127.0.0.1", port), 0.2).close()
            return p, port
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("server did not start")


def stop(p):
    """Kills the server and returns (and with --verbose prints) its console output."""
    p.kill()
    p.wait()
    p.out_file.close()
    text = open(p.out_file.name, "rb").read().decode("utf8", "replace")
    if "--verbose" in sys.argv:
        print(text)
    os.remove(p.out_file.name)
    return text


def connect(port, user="anonymous", pw="x", passive=True):
    f = ftplib.FTP()
    f.connect("127.0.0.1", port, timeout=10)
    f.login(user, pw)
    f.set_pasv(passive)
    return f


def main_tests(binary):
    d = tempfile.mkdtemp()
    with open(os.path.join(d, "hola.txt"), "wb") as fh:
        fh.write(b"hola mundo\n")
    os.mkdir(os.path.join(d, "sub"))
    with open(os.path.join(d, ".oculto"), "w") as fh:
        fh.write("x")
    p, port = start(binary, d, [])
    try:
        f = connect(port)
        ok(f.getwelcome().startswith("220 InstantFTP"), "banner")
        ok(f.pwd() == "/", "PWD at root")
        names = f.nlst()
        ok("hola.txt" in names and "sub" in names and os.path.basename(sys.argv[1]) in names, f"NLST shares exe folder {names}")
        ok("logs" not in names, "logs folder hidden from listing")
        ok(".oculto" not in names, "hidden files not listed by default")
        lines = []
        f.retrlines("LIST", lines.append)
        ok(any(l.startswith("drwxrwxrwx") and l.endswith(" sub") for l in lines), "LIST dir line with x bits")
        ok(any(l.startswith("-rw-rw-rw-") and " 11 " in l and l.endswith("hola.txt") for l in lines), "LIST file line + size")
        lines = []
        f.retrlines("LIST -la", lines.append)
        ok(any(l.endswith(".oculto") for l in lines), "LIST -la shows hidden")
        lines = []
        f.retrlines("LIST *.txt", lines.append)
        ok(len(lines) == 1 and lines[0].endswith("hola.txt"), "LIST wildcard")
        lines = []
        f.retrlines("LIST -l sub", lines.append)
        ok(lines == [], "LIST -l <dir> lists contents of dir (empty)")

        buf = io.BytesIO()
        f.retrbinary("RETR hola.txt", buf.write)
        ok(buf.getvalue() == b"hola mundo\n", "RETR")
        ok(f.size("hola.txt") == 11, "SIZE")
        ok(len(f.sendcmd("MDTM hola.txt").split()[1]) == 14, "MDTM")
        expect_err(lambda: f.size("sub"), "550", "SIZE on dir")

        data = os.urandom(3 * 1024 * 1024 + 17)
        f.storbinary("STOR sub/big.bin", io.BytesIO(data))
        ok(open(os.path.join(d, "sub", "big.bin"), "rb").read() == data, "STOR 3MB binary intact")
        buf = io.BytesIO()
        f.retrbinary("RETR /sub/big.bin", buf.write)
        ok(buf.getvalue() == data, "RETR 3MB binary intact")

        buf = io.BytesIO()
        f.retrbinary("RETR hola.txt", buf.write, rest=5)
        ok(buf.getvalue() == b"mundo\n", "REST + RETR (resume download)")
        f.storbinary("STOR part.bin", io.BytesIO(b"AAAAAAAAAA"))
        f.storbinary("STOR part.bin", io.BytesIO(b"BBB"), rest=4)
        ok(open(os.path.join(d, "part.bin"), "rb").read() == b"AAAABBB", "REST + STOR (resume upload)")
        f.storbinary("APPE part.bin", io.BytesIO(b"CC"))
        ok(open(os.path.join(d, "part.bin"), "rb").read() == b"AAAABBBCC", "APPE")

        ok(f.mkd("nueva").startswith("/nueva"), "MKD returns path")
        f.cwd("nueva")
        ok(f.pwd() == "/nueva", "CWD relative")
        f.cwd("..")
        ok(f.pwd() == "/", "CWD ..")
        f.sendcmd("CDUP")
        ok(f.pwd() == "/", "CDUP at root stays at root")
        expect_err(lambda: f.sendcmd("CWD .."), "550", "CWD .. above root")
        expect_err(lambda: f.cwd("noexiste"), "550", "CWD missing dir")
        f.rename("part.bin", "nueva/renombrado.bin")
        ok(os.path.exists(os.path.join(d, "nueva", "renombrado.bin")), "RNFR/RNTO")
        expect_err(lambda: f.sendcmd("RNTO x"), "503", "RNTO without RNFR")
        f.delete("nueva/renombrado.bin")
        f.rmd("nueva")
        ok(not os.path.exists(os.path.join(d, "nueva")), "DELE + RMD")
        expect_err(lambda: f.rmd("/"), "550", "RMD root refused")

        # security: path escapes
        for bad in ["../etc/passwd", "/../etc/passwd", "...", ".. ", "c:\\boot.ini", "c:boot.ini", "x:stream", "con", "nul.txt"]:
            expect_err(lambda b=bad: f.retrbinary("RETR " + b, lambda _: None), "550", f"blocked path {bad!r}")
        expect_err(lambda: f.retrbinary("RETR logs/x.log", lambda _: None), "550", "logs folder not readable")
        expect_err(lambda: f.cwd("logs"), "550", "logs folder not enterable")
        expect_err(lambda: f.sendcmd("PORT 8,8,8,8,0,80"), "500", "FTP bounce PORT refused")
        expect_err(lambda: f.sendcmd("PORT 1,2,3"), "501", "malformed PORT")

        resp = f.sendcmd("FEAT")
        ok("UTF8" in resp and "EPSV" in resp and "REST STREAM" in resp, "FEAT")
        ok(f.sendcmd("SYST").startswith("215"), "SYST")
        ok(f.sendcmd("STAT").startswith("211"), "STAT")
        ok("hola.txt" in f.sendcmd("STAT hola.txt"), "STAT path on control conn")
        ok(f.sendcmd("TYPE A").startswith("200"), "TYPE A")
        ok(f.sendcmd("TYPE I").startswith("200"), "TYPE I")
        expect_err(lambda: f.sendcmd("XYZZY"), "500", "unknown command")
        # pipelined commands in one packet
        f.sock.sendall(b"NOOP\r\nPWD\r\n")
        ok(f.getresp().startswith("200") and f.getresp().startswith("257"), "pipelined commands")

        # EPSV + active mode
        f.af = socket.AF_INET6  # forces ftplib to use EPSV
        buf = io.BytesIO()
        f.retrbinary("RETR hola.txt", buf.write)
        ok(buf.getvalue() == b"hola mundo\n", "EPSV transfer")
        f.af = socket.AF_INET
        f.set_pasv(False)
        buf = io.BytesIO()
        f.retrbinary("RETR hola.txt", buf.write)
        ok(buf.getvalue() == b"hola mundo\n", "active mode (PORT) transfer")
        f.set_pasv(True)

        # UTF-8 names
        f.sendcmd("OPTS UTF8 ON")
        f.storbinary("STOR año_ñandú.txt", io.BytesIO(b"x"))
        ok("año_ñandú.txt" in f.nlst(), "UTF-8 file names")

        # concurrent sessions keep separate directories
        g = connect(port)
        g.cwd("sub")
        ok(f.pwd() == "/" and g.pwd() == "/sub", "independent cwd per session")
        results = []

        def dl(conn):
            b = io.BytesIO()
            conn.retrbinary("RETR /sub/big.bin", b.write)
            results.append(b.getvalue() == data)

        t = [threading.Thread(target=dl, args=(c,)) for c in (f, g)]
        [x.start() for x in t]
        [x.join() for x in t]
        ok(results == [True, True], "two simultaneous downloads")
        g.quit()
        f.quit()

        logs = os.listdir(os.path.join(d, "logs"))
        ok(len(logs) == 1 and logs[0].endswith(".log") and len(logs[0]) == len("2026-11-20.log"), f"daily log file {logs}")
        text = open(os.path.join(d, "logs", logs[0]), "rb").read()
        ok(b"RETR ok /sub/big.bin" in text and b"PASS ****" in text and b"PASS x" not in text, "log content, password masked")
    finally:
        console = stop(p)
        shutil.rmtree(d, ignore_errors=True)
    ok("Sharing:" in console and "Ready! Connect to ftp://" in console, "console in English by default")


def option_tests(binary):
    d = tempfile.mkdtemp()
    p, port = start(binary, d, ["-g", "-u", "ana:secreto", "-t", "2", "-lang", "es"])
    try:
        f = ftplib.FTP()
        f.connect("127.0.0.1", port, timeout=10)
        expect_err(lambda: f.sendcmd("LIST"), "530", "commands need login with -u")
        expect_err(lambda: f.login("ana", "mal"), "530", "wrong password")
        f.login("ana", "secreto")
        ok(True, "login with -u")
        expect_err(lambda: f.storbinary("STOR x", io.BytesIO(b"x")), "553", "-g blocks STOR")
        expect_err(lambda: f.delete(os.path.basename(sys.argv[1])), "550", "-g blocks DELE")
        expect_err(lambda: f.mkd("x"), "550", "-g blocks MKD")
        expect_err(lambda: f.rename(os.path.basename(sys.argv[1]), "y"), "550", "-g blocks RNFR")
        time.sleep(3)
        try:
            f.voidcmd("NOOP")
            ok(False, "idle timeout")
        except ftplib.all_errors as e:
            ok("421" in str(e) or isinstance(e, (EOFError, OSError)), "idle timeout closes session")
    finally:
        console = stop(p)
        shutil.rmtree(d, ignore_errors=True)
    ok("Compartiendo:" in console and "solo lectura" in console, "console in Spanish with -lang es")


if __name__ == "__main__":
    b = os.path.abspath(sys.argv[1])
    print("== default (double click) mode")
    main_tests(b)
    print("== options -g -u -t -lang es")
    option_tests(b)
    print(f"\nALL {PASS} CHECKS PASSED")
