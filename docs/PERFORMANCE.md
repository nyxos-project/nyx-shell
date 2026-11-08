# Nyx Shell Core: measurements

Handoff 01. Baselines of what runs today, and Shell Core and its Quickshell runtime
beside them. No targets are set here; these are the numbers Handoff 03 compares
against.

## Conditions

- 2026-11-07, the laptop: Intel Core Ultra 9 285H, eDP 2560×1600 @1.6, plus headless
  outputs (three or four monitors in Hyprland's list), Omarchy, Hyprland 0.56 (Lua
  config), Quickshell 0.3.2.
- Release builds (`lto = "thin"`, stripped). Shell Core ran `--read-only` with scratch
  XDG folders and its own socket, against the real session, while the user worked.
  The user's session was busy: terminals with animated titles gave Hyprland about 18
  window updates a second, and other builds were compiling.
- Test surfaces were pinned to a headless output (`NYX_SHELL_SCREEN`) with the
  keyboard off. Nothing dispatched to Hyprland.
- Scripts: `tools/measure.py` (memory and CPU from `/proc`), `tools/startup.py`,
  `tools/e2e_latency.py`, `tools/overlay_latency.py`, `nyx-shellctl bench`, and the
  `burst` test.

Memory is from `smaps_rollup`. "Private" is resident private memory; this machine
swaps idle pages out, so "swapped" is listed too, and **private plus swapped** is the
comparable footprint of a process.

## Idle cost, side by side

30 s, all processes measured over the same interval:

| Process | RSS | Private | Swapped | Private + swapped | CPU | Wakeups/s |
| --- | --- | --- | --- | --- | --- | --- |
| nyx-shell-core (palette build) | 18.1 MiB | 4.0 MiB | 2.8 MiB | **6.8 MiB** | 0.20 % | 16 |
| nyx-shell-qs, no surfaces loaded | 138.7 MiB | 56.2 MiB | 2.6 MiB | **58.8 MiB** | 0.00 % | 0 |
| Omarchy's shell (Quickshell) | 574.7 MiB | 523.8 MiB | 161.4 MiB | **685.2 MiB** | 4.60 % | 194 |
| nyx-dock | 52.8 MiB | 29.0 MiB | 35.2 MiB | 64.2 MiB | 0.20 % | 21 |
| nyx-control-center (hidden) | 16.3 MiB | 1.4 MiB | 7.9 MiB | 9.3 MiB | 0.00 % | 2 |
| nyx-notification-center (hidden) | 17.6 MiB | 2.6 MiB | 5.9 MiB | 8.5 MiB | 0.00 % | 2 |
| nyx-notifyd | 5.9 MiB | 3.5 MiB | 2.2 MiB | 5.7 MiB | 0.00 % | 0 |
| nyx-switcher | 60.9 MiB | 46.0 MiB | 20.1 MiB | 66.1 MiB | 0.17 % | 20 |
| nyx-clipboard popup (hidden) | 20.1 MiB | 4.4 MiB | 5.5 MiB | 9.9 MiB | 0.00 % | 2 |

During a busier half minute (about 18 window updates a second), freshly started:

| Process | RSS | Private | CPU | Wakeups/s |
| --- | --- | --- | --- | --- |
| nyx-shell-core (palette build) | 26.2 MiB | 13.4 MiB | 0.40 % | 46 |
| Omarchy's shell | 568.8 MiB | 526.1 MiB | 14.27 % | 457 |
| nyx-dock | 64.6 MiB | 35.7 MiB | 0.73 % | 57 |
| nyx-switcher | 65.4 MiB | 50.5 MiB | 0.73 % | 38 |

Shell Core's wakeups follow Hyprland's events: one read per batch of events, one
write per subscribed client. It has no timers while every client keeps up.

## Shell Core with and without the palette

The `palette` feature links `nyx-theme`, which links GTK, to put the resolved palette in
theme events. Both builds ran at the same time on the same events (30 s):

| Build | Binary | RSS | Private + swapped | CPU | Start to `READY=1` (median of 10) |
| --- | --- | --- | --- | --- | --- |
| palette (default) | 6.6 MB | 15.6 MiB | 5.8 MiB | 0.40 % | 28.5 ms (max 183 ms) |
| no palette | 6.1 MB | 7.5 MiB | 5.0 MiB | 0.37 % | 3.8 ms (max 12 ms) |

GTK costs little private memory but doubles RSS and adds about 25 ms to start-up,
almost all of it loading shared libraries. `READY=1` is sent once the first
compositor snapshot is in. Theme Engine 2.0 (Handoff 02) should offer tokens without
GTK so the palette feature can go.

## The Quickshell runtime

| State | RSS | Private + swapped | CPU (30 s) | Wakeups/s |
| --- | --- | --- | --- | --- |
| No surfaces (Quickshell itself) | 138.7 MiB | 58.8 MiB | 0.00 % | 0 |
| Examples loaded, all hidden | 143.8–157.2 MiB | 59–64 MiB | 0.30–0.37 % | 17–24 |
| Inspector shown (15 s) | 165.5 MiB | 84.8 MiB | 5.93 % | 179 |

- Quickshell's own floor is about 56 MiB private, against about 9 MiB for a hidden GTK
  panel process. One shared runtime pays that once; a separate Quickshell process per
  panel would pay it every time.
- A hidden surface must not rebuild on every desktop update. The first inspector
  bound its lists while hidden and cost 3 % of a core; loading its contents only while
  shown brought the hidden runtime to 0.3 %. Handoff 03 surfaces should do the same.
- The shown inspector re-renders on every update (titles included), hence 6 %. A
  diagnostic tool; real surfaces should bind to what they show.
- For scale: Omarchy's whole shell is 685 MiB here. A Nyx Quickshell runtime holding
  the dock, Control Center and Notification Center would replace three GTK processes
  (about 82 MiB together, private + swapped), so the move is roughly memory-neutral
  only if those surfaces stay light. Measure each one as it moves.

## Latency

| Path | Median | p95 | Max | Samples |
| --- | --- | --- | --- | --- |
| Request round trip (`GetSessionStatus`, one connection) | 49 µs | 140 µs | 1.8 ms | 2000 |
| Event from Shell Core to a client (`DesktopChanged`) | 81 µs | 327 µs | 1.5 ms | 310 |
| Hyprland title change → client delta | 226 µs | 514 µs | 731 µs | 450 |
| Hyprland focus change → client delta | 2.9 ms | | | 9 |
| `nyx-shellctl` as a keybind (start, connect, hello, request) | 1.3 ms | | | 20 |
| `ShowOverlay` → `OverlayChanged` (including starting `nyx-shellctl`) | 1.3–1.4 ms | | | 10 |
| `ShowOverlay` → layer surface in Hyprland | 5.3–5.6 ms | | 15 ms | 10 |

- Focus changes usually arrive with events that make the watcher read the window list
  again (one `j/clients` request), hence milliseconds rather than microseconds; few
  samples, as only the user's own focus changes were measured.
- The surface figure polls `hyprctl -j layers`, so it is coarse (a few milliseconds).

## Rapid workspace switching

`tests/burst.rs` against a fake Hyprland, release build, with a client that reads
continuously and follows the protocol (deltas, snapshot on `Resync`):

| Load | Deltas sent | Resyncs | Events dropped | Client matches a fresh snapshot |
| --- | --- | --- | --- | --- |
| 2,000 switches at 1,000 a second (held key) | 1,969 | 0 | 0 | immediately after the last |
| 200 switches at 20 a second | 199 | 0 | 0 | immediately after the last |
| 20,000 switches (40,000 events) flat out | 1,887 | 3 | 170 | 66 ms after starting |
| 2,000 switches in one write | 3 | 0 | 0 | 83 ms after starting |

- Events Hyprland writes together are applied together, so a burst becomes a few
  deltas. A client that can't keep up gets `Resync` and one snapshot instead of an
  unbounded queue; others are unaffected.

## Multiple monitors

Measured with three to four outputs in Hyprland's list. Shell Core's state grows by one
small record per monitor; no per-monitor work happens outside events. The Quickshell
runtime adds one surface per monitor only for persistent surfaces
(`MonitorSurfaces`); transient overlays are on one monitor at a time.

## Not measured

- Overlay latency with a real keybind on NyxOS (needs the VM).
- GPU memory of the Quickshell runtime.
- Sleep/resume and logout (they'd interrupt the user).
