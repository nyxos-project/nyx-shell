#!/usr/bin/env python3
"""Hyprland event to Shell Core client, measured on focus and title changes, read-only.

    tools/e2e_latency.py SECONDS

Reads Hyprland's event socket directly and, at the same time, Shell Core's desktop
deltas as an ordinary client. Each `activewindowv2>>ADDR` is matched with the first
delta whose active window is ADDR (a newer focus change replaces an unmatched one),
and each `windowtitlev2>>ADDR,TITLE` with the first delta carrying that title. Nothing is sent to Hyprland; the focus changes are
whatever the user does meanwhile.
"""
import json
import os
import selectors
import socket
import statistics
import sys
import time


def hypr_socket2():
    base = os.path.join(os.environ["XDG_RUNTIME_DIR"], "hypr")
    sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    path = os.path.join(base, sig, ".socket2.sock")
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    return s


def core_socket():
    path = os.environ.get("NYX_SHELL_SOCKET") or os.path.join(os.environ["XDG_RUNTIME_DIR"], "nyx-shell/core.sock")
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.sendall(b'{"v":1,"id":1,"op":"SubscribeDesktopChanges"}\n')
    return s


def lines(sock, buf):
    data = sock.recv(65536)
    if not data:
        raise SystemExit("closed")
    buf += data
    *done, rest = buf.split(b"\n")
    return done, rest


def main():
    seconds = float(sys.argv[1])
    h, c = hypr_socket2(), core_socket()
    sel = selectors.DefaultSelector()
    sel.register(h, selectors.EVENT_READ, "h")
    sel.register(c, selectors.EVENT_READ, "c")
    bufs = {"h": b"", "c": b""}
    waiting = {}
    titles = {}
    lat = []
    title_lat = []
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        for key, _ in sel.select(timeout=0.5):
            done, bufs[key.data] = lines(key.fileobj, bufs[key.data])
            now = time.monotonic()
            for line in done:
                if key.data == "h":
                    text = line.decode(errors="replace")
                    if text.startswith("activewindowv2>>"):
                        addr = text.split(">>", 1)[1].strip().removeprefix("0x").lower()
                        # Only the latest focus counts: quick changes merge into one delta.
                        waiting = {addr: now} if addr else {}
                    elif text.startswith("windowtitlev2>>"):
                        addr, _, title = text.split(">>", 1)[1].partition(",")
                        title = "".join(ch for ch in title if ch.isprintable())[:512]
                        titles[addr.lower()] = (title, now)
                else:
                    m = json.loads(line)
                    if m.get("ev") == "DesktopChanged":
                        active = (m["body"].get("active") or {}).get("window")
                        if active and active in waiting:
                            lat.append((now - waiting.pop(active)) * 1e6)
                        for w in m["body"]["windows"]["upsert"]:
                            t = titles.get(w["address"])
                            if t and t[0] == w["title"]:
                                title_lat.append((now - t[1]) * 1e6)
                                del titles[w["address"]]
    for name, xs in (("focus changes", lat), ("title changes", title_lat)):
        if not xs:
            print(f"no {name} in {seconds:.0f} s")
            continue
        xs.sort()
        q = lambda f: xs[int((len(xs) - 1) * f)]
        print(f"{len(xs)} {name} in {seconds:.0f} s: Hyprland event → Shell Core client median {statistics.median(xs):.0f} µs, p95 {q(0.95):.0f} µs, max {xs[-1]:.0f} µs")


if __name__ == "__main__":
    main()
