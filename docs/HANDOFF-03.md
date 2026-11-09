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

- Shell Core follows the GTK panels' open state through their layer surfaces
  (`nyx-desktop`'s `layers`), exactly and without changes to them; `Panel1` stays only
  for announcing and toggling until the panels move.
- Components still on their own `nyx-desktop` watcher get refresh rates, open layers and
  the monitors hook by moving their nyx-core pin to `a93881d` or later.
- Theme events come from Theme Engine 2.0's snapshot. The opt-in `palette` feature
  (GTK, for machines without the engine) can go once every NyxOS session runs
  `nyx-themed`. The `Theme` singleton could also move onto the engine's own
  `Nyx.Theme` QML module, which reads the same snapshot.
- Nyx Locker sets logind's `LockedHint` (nyx-greeter `ab65ecd`, shipped by nyx-packages
  `f15fd58`), and the hyprlock fallback clears it after unlocking (nyxos `e0690d3`).
- Persistent overlays should be shown again by their component after a Shell Core
  restart (they register again; showing is theirs to ask).
- **Handoff 03 (stages A to G) is in** (see [MIGRATION.md](MIGRATION.md), Results). Left:
  - NyxOS packages for the Dock, the Notification Center and the Switcher still start the
    GTK interfaces; each moves like the Control Center's did (build the backend, install
    its unit and `qml/<Component>` in `share/nyx-shell/components/<app>`, autostart the
    backend instead of the GTK unit).
  - The Switcher's quick-tap gap (a modifier released in the first ~6 ms): a release
    bind in the switcher's integration, or a keyboard-state API in Quickshell.
  - Plugin CSS and frame nodes in `Nyx.PluginView`; submenus in `Nyx.Ui.Menu`.
  - The components run in standalone mode on the laptop; on NyxOS they should register
    overlays with Shell Core instead of keeping `Panel1` (the panels' bus names stay for
    command lines and bar widgets).
  - Stage J (removing the GTK code) waits until the QML versions have run for a while;
    `--ui gtk` is the way back until then.
