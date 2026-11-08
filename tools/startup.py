#!/usr/bin/env python3
"""Time from starting nyx-shell-core to its READY=1 (sd_notify), N times.

    tools/startup.py BINARY SOCKET N [EXTRA ARGS...]

Each run listens on SOCKET (pass a scratch path), is given its own NOTIFY_SOCKET,
and is stopped with SIGTERM once ready.
"""
import os
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time


def main():
    binary, sock_path, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
    extra = sys.argv[4:]
    times = []
    for _ in range(n):
        with tempfile.TemporaryDirectory() as d:
            notify = os.path.join(d, "notify")
            s = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
            s.bind(notify)
            s.settimeout(10)
            env = dict(os.environ, NOTIFY_SOCKET=notify, RUST_LOG="warn")
            t = time.monotonic()
            p = subprocess.Popen([binary, "--socket", sock_path, *extra], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            while True:
                msg = s.recv(256).decode()
                if "READY=1" in msg:
                    times.append((time.monotonic() - t) * 1000)
                    break
            p.send_signal(signal.SIGTERM)
            p.wait(timeout=5)
            s.close()
    print(f"{n} starts to READY=1: median {statistics.median(times):.1f} ms, min {min(times):.1f} ms, max {max(times):.1f} ms")


if __name__ == "__main__":
    main()
