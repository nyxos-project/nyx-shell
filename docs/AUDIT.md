# Nyx Shell Core: repository audit and reuse report

Handoff 01, stage A. Audited 2026-11-07 from the checkouts in `~/Projects` (read-only:
no builds, tests or installs). Every statement names the file it comes from.

## Summary

- **There is no Shell Coordinator.** No repository has a `nyx-shell`, `shelld` or a
  coordinator process. The desktop was designed without one:
  `nyx/docs/desktop/ARCHITECTURE.md` §3 says "No central desktop daemon", and
  `nyx-desktop/src/lib.rs` says each process subscribes on its own. The integration
  audit (`nyx-integration-audit/2026-10-30`) advises against "an always-running central
  action daemon". Shell Core reverses that decision; the superseding entry is in
  `nyx/docs/desktop/ARCHITECTURE.md` §16.
- **What exists instead** is two shared pieces in nyx-core:
  - `nyx-desktop` (`nyx-core/crates/nyx-desktop`): Hyprland discovery, the socket2 event
    parser, a monitor/workspace/window model, a reconnecting watcher thread, and
    Lua-first window dispatchers. Without its `glib` feature it is plain std, so a daemon
    can use it. The watcher alone measured 3.2 MiB RSS (§8 of the architecture doc).
  - `Panel1` (`nyx-desktop/src/panel.rs`): peer-to-peer mutual exclusion over D-Bus.
    Each panel serves `io.github.design_nexus.nyx.Panel1` at
    `/io/github/design_nexus/nyx/Panel` (`Show`, `Hide`, `Toggle`, `IsOpen`, signal
    `Opened(s id)`) and hides when another panel announces itself. Used by the Control
    Center, the Notification Center and the Switcher.
- **Every shell component is GTK4 + gtk4-layer-shell Rust.** The only Quickshell in the
  NyxOS session is the Assistant HUD, which is not enabled by default. Omarchy's own shell
  is one Quickshell process; measured at 453 MiB private memory against about 10 MiB per
  hidden Nyx panel (architecture doc §14). The Quickshell migration (Handoff 03) has to
  be measured before it ships.
- **No component needs a rewrite to work with Shell Core.** The ones that speak `Panel1`
  join its overlay policy through a bridge, unchanged. The rest keep working beside it
  until Handoff 03 moves them.

## Inventory of the existing coordination ("Shell Coordinator" equivalent)

What a coordinator would do, where it is done today, and what Shell Core does with it.

| Responsibility | Today | Shell Core |
| --- | --- | --- |
| Compositor state | Each process runs `nyx_desktop::watch` (dock 2 connections, bar host plus its `workspaces` plugin, widgets, switcher, Control Center while its mode tile shows): 6 to 7 event-socket connections, each with its own model | One watcher, one normalized state with a generation number, served to clients. Existing processes keep theirs until Handoff 03 |
| Focused monitor | Synchronous `j/monitors` on the GTK thread at show time: `nyx-control-center/apps/control-center/src/panel.rs::target_monitor`, `nyx-notification-center/apps/notification-center/src/panel.rs::focused_monitor` (identical code) | `active.monitor` in the state; overlay placement resolved by the core |
| Overlay mutual exclusion | `Panel1` among Control Center, Notification Center, Switcher only. Dock popovers, bar popovers, widgets' edit mode, Glimpse, Clipboard, Capture, Assistant HUD are outside it | Declarative overlay policy registry; `Panel1` bridge keeps the GTK panels in it |
| Fullscreen behaviour | Three different rules: dock `nyx-dock-core/src/hide.rs::should_hide_on` (any fullscreen or overlap), bar `nyx-bar-core::should_hide` (true fullscreen only), widgets `host.rs::update_visibility` (fullscreen, covered, locked). Control Center, Notification Center pop-ups and Switcher ignore it | One `fullscreen` rule per overlay entry (`show`, `hide`, `block`) |
| Lock awareness | Widgets read logind `LockedHint`; Passwords polls Hyprland `solitaryBlockedBy`; nobody else | Core reads `LockedHint` once; every overlay is hidden and show requests refused while locked |
| Action routing | None. Keybinds call each binary (`nyx-control-center --toggle`, `nyx-glimpse --toggle`, `nyx-switcher-send …`). Per-app `io.github.design_nexus.nyx.Control` (`nyx-core/src/control.rs`) serves the Assistant | Fixed action table routed to D-Bus, the switcher socket, a fixed launch, or a registered component |
| Component health | systemd units only (`Restart=on-failure`) | Component registry with readiness, health and dependencies; GTK panels tracked by bus-name ownership |
| Theme propagation | Each process polls once a second (`nyx-design/crates/nyx-theme/src/lib.rs::watch_omarchy`): six pollers. `nyx-themed` (`nyx-system/apps/themed`) watches with inotify and writes Hyprland/foot/hyprlock files | One watcher, a theme generation number, events to clients |
| Shortcuts | Owned blocks written by each app (`nyx-switcher --integrate`, Glimpse, Clipboard), loaded by NyxOS `hyprland.nix` | Owned `~/.config/hypr/nyx-shell.lua` with a duplicate check against `hyprctl -j binds`; empty by default so existing binds keep their keys |

## Components

For each: language and UI, process boundary, IPC, Hyprland use, what must be kept, and
whether it can integrate without a rewrite.

### Nyx Dock (`nyx-dock`)

- Rust, GTK4, gtk4-layer-shell. Crates `apps/dock`, `crates/nyx-dock-core`; a C++
  Hyprland plugin `hyprland-plugin/main.cpp` (`nyx-dock-swipe.so`).
- Resident user unit; GApplication uniqueness. Always shown; no toggle.
- IPC: none exposed. Consumes Unity `LauncherEntry` signals, MPRIS; runs
  `nyx-switcher-send overview`.
- Hyprland: `watch_local`, plus a second socket2 reader for the plugin's custom
  `nyxswipe>>` / `nyxswipeend>>` events (`src/swipe.rs`), `j/cursorpos`, a 150 ms
  `j/activewindow` poll while window-control dots show (`controls.rs`), dispatchers via
  `nyx_desktop::actions`.
- Layer Top, keyboard None (OnDemand while a menu is open). Per-monitor surfaces keyed by
  `stable_id`. Smart hiding with fullscreen/overlap.
- Keep: hiding modes with settle-and-reverify, swipe following, thumbnails, app cards,
  stacks, Trash, badges, window-controls pill, uwsm launching.
- **Integration:** reads Shell Core state in Handoff 03; no change now. It does not use
  `Panel1`, so its menus stay outside overlay policy until then.

### Control Center (`nyx-control-center`)

- Rust, GTK4, gtk4-layer-shell. `apps/control-center`, `crates/nyx-widgets`,
  `crates/nyx-providers`.
- Resident, hidden until toggled; `--toggle|--show|--hide` forwarded through
  GApplication. Bus name `io.github.design_nexus.nyx.ControlCenter`.
- IPC: `Panel1` (id `control-center`), `ControlCenterPlugins1`. Consumes BlueZ, UPower,
  power-profiles, MPRIS, `Home.Devices1`, `Notifications1`, many CLIs.
- Hyprland: synchronous focused-monitor lookup; a watcher only while the mode tile shows.
- Layer Overlay, keyboard OnDemand, a full-screen catcher surface. No fullscreen rule.
- Keep: providers run only while subscribed, passwords to `nmcli` on stdin, Omarchy state
  changed only through Omarchy's commands.
- **Integration:** joins overlay policy through the `Panel1` bridge today; action
  `shell.control-center.toggle` calls `Panel1.Toggle`.

### Notification Center (`nyx-notification-center`)

- Rust, GTK4. `apps/notification-center` (UI), `apps/notifyd` (`nyx-notifyd`, zbus, no
  GTK), `crates/nyx-notify-store` (SQLite).
- Two resident services. notifyd owns `io.github.design_nexus.nyx.Notifications`
  (`Notifications1`, JSON payloads) and, in Nyx mode, `org.freedesktop.Notifications`.
- Panel: `Panel1` (id `notification-center`), Overlay, OnDemand, catcher; pop-ups Overlay,
  keyboard None, drawn over fullscreen windows.
- Keep: read/dismissed/pinned flags, "gone" markers, DND schedule, retention, code masking,
  the freedesktop compatibility suite, the Omarchy bridge.
- **Integration:** `Panel1` bridge now. notifyd stays a separate service in every phase:
  the notification name must outlive any UI.

### Desktop Widgets (`nyx-desktop-widgets`), NyxOS-only

- Rust, GTK4. Plugins through `nyx-plugin-host` (systemd scopes, bubblewrap, JSON lines).
- `DesktopWidgets1` (`Edit`, `ListWidgets`, `Move`, `Resize`, `Reload`, `RestartPlugin`).
- Layer Bottom, keyboard None except edit mode, input region only over interactive
  widgets. Hides on lock, fullscreen, coverage.
- **Integration:** Handoff 03. Its plugin protocol stays.

### Switcher (`nyx-switcher`)

- Rust, GTK4. `nyx-switcher` plus `nyx-switcher-send` (plain std, no GTK).
- Unix socket `$XDG_RUNTIME_DIR/nyx-switcher.sock`, one text line per request
  (`next windows <µs>`, `overview [APP] <µs>`, `hide <µs>`); `Panel1` id `switcher` on
  `io.github.design_nexus.nyx.Switcher`.
- Layer Overlay, keyboard **Exclusive** (it must see the modifier release).
- Keep: about 1 ms press latency, the fallback to Hyprland's cycling when it isn't
  running, owned-block shortcut editing with rollback.
- **Integration:** `Panel1` bridge; action `shell.switcher.overview` writes `overview` to
  its socket. The Alt+Tab path stays direct (latency).

### Nyx Bar (`nyx-bar`), NyxOS-only

- Rust, GTK4. Every item is a plugin; `nyx-plugin` CLI; `Bar1` D-Bus interface.
- Layer Top, exclusive zone = height, hides for true fullscreen.
- The `workspaces` plugin runs its own watcher in its own process.
- **Integration:** Handoff 03.

### Glimpse (`nyx-glimpse`)

- Rust, GTK4. `nyx-glimpse` (quits when hidden), `nyx-glimpsed` (indexer).
- Overlay, Exclusive keyboard, full-monitor transparent surface, monitor chosen by the
  compositor. Activated by `nyx-glimpse --toggle` (Super+Space).
- Talks to Passwords (peer-to-peer zbus socket) and Clipboard without seeing secrets.
- **Integration:** action `shell.glimpse.toggle` runs `nyx-glimpse --toggle` (fixed argv).
  Overlay policy in Handoff 05.

### Assistant (`nyx-assistant`)

- Rust workspace plus QML HUD (`apps/assistant/quickshell/shell.qml`, a standalone `qs`).
- Socket `$XDG_RUNTIME_DIR/nyx-assistant/assistant.sock` (NDJSON). Aura:
  `assistant-visual`, Overlay, keyboard None, empty input region, hides on fullscreen.
  It insets for Omarchy's bar by name and doesn't know `nyx-bar` (a gap on NyxOS).
- Known gap: `Control::Approve` accepted from any same-user socket client (Secrets are
  re-confirmed through the broker; privileged actions through polkit `auth_admin`).
- **Integration:** Handoff 05. The Aura becomes a `presentation` overlay; inference
  stays in the Assistant.

### Clipboard, Capture

- Clipboard: `nyx-clipboardd` (`Clipboard1`, per-program scopes through `nyx-peer`) and a
  resident popup (Overlay, Exclusive keyboard).
- Capture: several Overlay surfaces; deduplicates its own Exclusive overlays.
- **Integration:** Handoff 03 for overlay policy; no change now.

### Lock screen, polkit agent, authentication (security-sensitive)

- **NyxOS:** Nyx Locker (`nyx-greeter/apps/locker`, GTK4 + `gtk4-session-lock`,
  ext-session-lock). hypridle starts `nyx-lock.service`; non-zero exit other than 3 hands
  over to hyprlock. PAM per method in `nyx-lock-core`. Hyprland keeps the session locked
  if the locker dies.
- **Omarchy:** `design-nexus.auth-lock` (nyx-biometrics), a Quickshell plugin inside
  Omarchy's shell process, so it isn't isolated from the bar.
- Polkit agent: `nyx-prompt` (system bus agent, PAM in polkit's helper).
- Broker: `nyx-authd` (`Auth1`, caller identification by executable, no approve method,
  grants dropped on lock and sleep); root `nyx-auth-keyd`.
- **Integration:** none, by design. Shell Core reads logind `LockedHint` only. It has no
  unlock, authenticate or approve operation.

### OSD

There is no OSD implementation in any Nyx repository. Nyx code calls `omarchy-osd` or
passes `--no-osd`. NyxOS has no media-key binds. The OSD example in `qml/examples` is a
policy test, not a product.

### Hyprland (`nyx-hyprland`, `Hyprland`)

- One Nyx patch: `gestures:workspace_swipe_wrap` (`Hyprland` branch
  `workspace-swipe-wrap`, commit `2d135555f`; packaged by `nyx-hyprland` and
  `nyx-packages/pkgs/hyprland-workspace-swipe-wrap.patch`).
- The only Nyx compositor extension is the dock's runtime plugin events.
- Everything Shell Core needs is upstream: socket1 (`j/…`, `dispatch`, `eval`), socket2
  events, Lua `hl.dsp.*`. **No compositor change is needed for this handoff.**
- Gap to document for later: Hyprland doesn't report which layer surface holds keyboard
  focus, so focus restoration infers it from the active window.
- Gap found while validating: socket2 sends nothing when a monitor's mode, scale or
  position changes at runtime (`hl.monitor` through `eval`, a settings change); only the
  Lua event `monitor.layout_changed` fires. Every `nyx-desktop` user keeps the old
  geometry until the next monitor event. Shell Core closes it without a patch: NyxOS's
  Hyprland config posts `custom>>nyx-shell-monitors` from that Lua event
  (`hl.dsp.event`), and Shell Core reads the monitors again when it arrives.

### Theme (`nyx-design`, `nyx-themes`, `nyx-system/apps/themed`)

- `nyx-theme` crate: TOML themes to GTK CSS. `~/.config/nyx/theme.toml` is the suite
  choice; Follow Omarchy reads `~/.local/state/omarchy/current/theme/colors.toml`.
- The crate always links GTK (`gtk4` is not optional).
- No theme generation or version exists; change detection is by mtime.
- **Integration:** Shell Core's theme adapter watches the same files once and publishes a
  generation. The palette comes from `nyx_theme::suite_theme` behind the default
  `palette` cargo feature (which links GTK); without the feature, events carry identity
  only. Theme Engine 2.0 (Handoff 02) replaces the adapter's source.

### Settings and configuration (`nyx-settings`, `nyx-system/crates/nyx-config`)

- Settings writes one file per component (`dock.toml`, `switcher.toml`,
  `control-center.toml`, Hyprland `settings.lua`), not a shared schema.
- `nyx-config` already implements NyxOS's layers: `/etc/nyxos/defaults` <
  `/etc/nyxos/overrides` < `~/.config/nyx/DOMAIN.toml`, with `set`, `reset` and atomic
  writes. Only `nyx-themed` uses it.
- **Integration:** Shell Core uses `nyx-config` domain `shell`. No new settings database.

### Shared crates

| Crate | Where | Used for |
| --- | --- | --- |
| `nyx-desktop` | nyx-core | Hyprland model, watcher, dispatchers (headless, no `glib`) |
| `nyx-peer` | nyx-core | Mapping a socket peer's executable to a trusted program |
| `nyx-config` | nyx-system | The `shell` settings domain |
| `nyx-theme` | nyx-design | The palette for theme events (optional `palette` feature) |

Missing everywhere: a logging crate (code uses `eprintln!`), a single-instance helper for
non-GTK daemons. Shell Core uses `tracing` and its own lock file.

### Packaging and startup

- `nyx-packages` turns `passthru.autostart` into user units `WantedBy=graphical-session.target`
  and builds from GitHub source inputs with git dependencies pinned by hash
  (`lib/git-hashes.json`).
- NyxOS: greetd, uwsm, Hyprland with a Lua config generated by Home Manager; no
  `exec-once`. Desktop components are enabled per app in `modules/nixos/apps.nix`.
- Shell Core ships the same way, disabled by default (`nyx.shell.enable`).

## Drift and risks found

- nyx-core is pinned at six different revisions across the shell components; they run
  different versions of the same model and parser.
- Theme polling: six once-a-second timers.
- Three copies of system providers (network, bluetooth, audio, power, MPRIS): `nyx-providers`,
  `nyx_plugin_sdk::services`, the dock's MPRIS.
- Several Exclusive-keyboard overlays (Glimpse, Clipboard, Capture, nyx-prompt, Assistant
  HUD during approval) with no arbitration between apps.
- The lock path differs: NyxOS emits logind `Lock`; Omarchy's shell lock doesn't.
