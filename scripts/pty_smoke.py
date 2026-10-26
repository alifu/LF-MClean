#!/usr/bin/env python3
"""Smoke test for the real terminal (not part of CI): menu -> scan -> Ctrl-C, expecting exit 0.

Usage: python3 -I scripts/pty_smoke.py [path-to-lfmclean]

Runs the binary in a pseudo-terminal with HOME set to an empty temporary folder, so nothing of
the real home is scanned or touched. The screen is drawn as a diff, so words can arrive in pieces:
the script waits for short whole tokens only.
"""
import fcntl
import os
import pty
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time

BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/debug/lfmclean"
STEP_TIMEOUT = 20.0
CTRL_C = b"\x03"
ROWS, COLUMNS = 30, 100


def read_until(fd, needle, timeout):
    """Reads output until `needle` appears; returns everything read, or None on timeout."""
    seen = b""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return seen if needle in seen else None
        if not chunk:
            break
        seen += chunk
        if needle in seen:
            return seen
    return None


def main():
    home = tempfile.mkdtemp(prefix="lfmclean-smoke-")
    try:
        master, slave = pty.openpty()
        # A new pty is 0x0, which the program rightly answers by drawing nothing.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0))
        env = {**os.environ, "HOME": home, "TERM": "xterm-256color"}
        child = subprocess.Popen(
            [BINARY], stdin=slave, stdout=slave, stderr=slave, env=env, close_fds=True
        )
        os.close(slave)
        if read_until(master, b"LF-MClean", STEP_TIMEOUT) is None:
            child.kill()
            sys.exit("FAIL: the menu never appeared")
        os.write(master, b"1")
        if read_until(master, b"Nothing", STEP_TIMEOUT) is None:
            child.kill()
            sys.exit("FAIL: the scan result never appeared")
        os.write(master, CTRL_C)
        try:
            code = child.wait(timeout=STEP_TIMEOUT)
        except subprocess.TimeoutExpired:
            child.kill()
            sys.exit("FAIL: Ctrl-C did not end the program")
        if code != 0:
            sys.exit(f"FAIL: exit status {code}")
        print("ok: menu -> scan -> Ctrl-C, exit 0")
    finally:
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()
