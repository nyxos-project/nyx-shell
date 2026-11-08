#!/usr/bin/env python3
"""Overlay activation: ShowOverlay to Shell Core's OverlayChanged, and to the layer
surface appearing in Hyprland, N times.

    tools/overlay_latency.py SHELLCTL OVERLAY N

Uses `SHELLCTL show/hide OVERLAY` (a whole process each time, included in the figure)
and reads `hyprctl -j layers` (polled, so the surface figure is coarse).
"""
import json
import os
import socket
import statistics
import subprocess
import sys
import time


def main():
    shellctl, overlay, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
    namespace = "nyx-shell-" + overlay
    path = os.environ.get("NYX_SHELL_SOCKET") or os.path.join(os.environ["XDG_RUNTIME_DIR"], "nyx-shell/core.sock")
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.sendall(b'{"v":1,"id":1,"op":"SubscribeOverlayState"}\n')
    f = s.makefile("r")
    f.readline()
    core, surface = [], []

    def wait_event(visible):
        while True:
            m = json.loads(f.readline())
            if m.get("ev") == "OverlayChanged" and m["body"]["overlay"]["id"] == overlay and m["body"]["overlay"]["visible"] == visible:
                return

    def mapped():
        layers = json.loads(subprocess.run(["hyprctl", "-j", "layers"], capture_output=True, text=True).stdout)
        return any(l["namespace"] == namespace for mon in layers.values() for lvl in mon["levels"].values() for l in lvl)

    for _ in range(n):
        t = time.monotonic()
        subprocess.run([shellctl, "show", overlay], stdout=subprocess.DEVNULL, check=True)
        wait_event(True)
        core.append((time.monotonic() - t) * 1000)
        while not mapped():
            if time.monotonic() - t > 5:
                raise SystemExit("the surface never appeared")
        surface.append((time.monotonic() - t) * 1000)
        subprocess.run([shellctl, "hide", overlay], stdout=subprocess.DEVNULL, check=True)
        wait_event(False)
        while mapped():
            time.sleep(0.005)
        time.sleep(0.1)
    print(f"{n} shows of {overlay}: request → OverlayChanged median {statistics.median(core):.1f} ms; → layer surface in Hyprland median {statistics.median(surface):.1f} ms (max {max(surface):.1f} ms)")


if __name__ == "__main__":
    main()
