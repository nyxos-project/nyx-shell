# Notes for Handoff 03: moving components onto Shell Core

What Handoff 01 leaves in place, how each component maps onto it, and what was learned
building and measuring it.

## What a component gets

- `ShellCore` (QML singleton): the connection, `desktop`, `overlays`, `theme`,
  `config`, `session`, `actions`; `request()`, `showOverlay()`, `hideOverlay()`,
  `toggleOverlay()`, `invoke()`, `windowAction()`, `focusWorkspace()`; `reconnected`
  and `actionInvoked` signals.
- `DesktopState`: lookups and derived views (active monitor and window, windows on a
  workspace, fullscreen on a monitor).
- `OverlaySurface`: a layer surface whose visibility, monitor and keyboard mode follow
  Shell Core's overlay policy; registers itself on every connection; Escape hides
  transient overlays.
- `MonitorSurfaces`: one delegate per allowed screen for persistent surfaces (headless
  outputs and per-screen settings handled).
- `ShellComponent`: the manifest, registration on every connection, ready/health
  reports, actions arriving as `invoked`.
- `Theme`, `Motion`, `KeyboardScope`, `LoadingState`, `ErrorState`.

A component must not open its own Hyprland socket, keep its own monitor list, or
implement its own mutual exclusion.

## Component by component

| Component | Overlay | Keyboard | Fullscreen | Notes |
| --- | --- | --- | --- | --- |
| Dock | persistent, `MonitorSurfaces` | none (on-demand while a menu is open) | `hide`, plus its own overlap rule | The swipe plugin's `nyxswipe` events need a route: either Shell Core forwards `custom`-style events, or the dock keeps that one reader. Exclusive zone from its `reserve()` logic |
| Control Center | transient, group `panels` | on-demand | `show` today; decide | `Panel1` goes once it registers an overlay; `shell.control-center.toggle` becomes a component action. Providers stay subscribe-on-show |
| Notification Center | transient, group `panels`; pop-ups passive | on-demand; pop-ups none | pop-ups `hide` (today they draw over fullscreen) | `nyx-notifyd` stays a separate service |
| Switcher | transient, group `panels` | exclusive | `show` | Keep the socket fast path for Alt+Tab (1 ms); the overview can be an action |
| Bar | persistent, top, `MonitorSurfaces` | none | `hide` for true fullscreen | Its plugin host stays; the `workspaces` plugin should read Shell Core's state instead of its own watcher |
| Desktop Widgets | persistent, bottom, `MonitorSurfaces` | none (on-demand in edit mode) | `hide` | Its lock check can use Shell Core's `locked` |
| OSD (new) | passive | none | `show` | The example shows the shape |
| Glimpse, Assistant Aura | Handoff 05 | | | Aura is `presentation` |

## Learned

- **A hidden surface must not rebuild on updates.** The first inspector bound its
  lists while hidden: 3 % of a core under a busy session. A `Loader` active only while
  shown: 0.3 %. Window titles change many times a second in terminals.
- **Bind to what is shown.** A shown surface that binds to the whole window list
  redraws on every title change (6 % for the inspector).
- **Quickshell's floor is 56 MiB private**, against about 9 MiB for a hidden GTK panel.
  One shared runtime pays it once; measure each surface as it moves
  ([PERFORMANCE.md](PERFORMANCE.md)).
- **Titles are untrusted**: `textFormat: Text.PlainText` everywhere.
- **Register on `reconnected`**, not once: Shell Core restarts and the runtime keeps
  going.
- **Hyprland gives the keyboard back** when a layer surface closes; overlays don't need
  to manage focus themselves (ARCHITECTURE.md, Focus).
- **Hyprland doesn't report runtime monitor changes** on its event socket; the NyxOS
  hook covers Shell Core. Components that keep their own watcher until they move keep
  that bug.
- **Testing**: `NYX_SHELL_SCREEN` pins surfaces to a headless output and never falls
  back to another one; `NYX_SHELL_NO_KEYBOARD=1` keeps them off the keyboard;
  `QS_NO_RELOAD_POPUP=1` and `QS_DISABLE_CRASH_HANDLER=1` keep Quickshell's own
  windows away.

## Open items

- `Panel1` has no close signal, so Shell Core asks `IsOpen` when a client looks. A
  `Closed(s id)` signal in nyx-core's `panel.rs` would make it exact until the panels
  move.
- Refresh rate and runtime monitor geometry belong in `nyx-desktop`'s model; Shell Core
  reads them itself for now.
- Theme events come from Theme Engine 2.0's snapshot. The opt-in `palette` feature
  (GTK, for machines without the engine) can go once every NyxOS session runs
  `nyx-themed`. The `Theme` singleton could also move onto the engine's own
  `Nyx.Theme` QML module, which reads the same snapshot.
- `ipc.ui_runtimes` is read at start; changing it needs a restart.
- Nyx Locker doesn't set logind's `LockedHint` (checked in the VM: `LockedHint=no` while
  locked), so Shell Core can't see the lock on NyxOS; Omarchy's shell lock doesn't set
  it either. A `SetLockedHint` call in Nyx Locker fixes it (SECURITY.md, known limits).
- Persistent overlays should be shown again by their component after a Shell Core
  restart (they register again; showing is theirs to ask).
