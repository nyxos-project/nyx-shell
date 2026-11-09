# Nyx Shell on NyxOS

How Shell Core is packaged, switched on and checked on NyxOS. It is off by default
until Handoff 03 moves components onto it.

## What ships

`nyx-packages/pkgs/nyx-shell.nix` builds the workspace and installs:

| Path | What |
| --- | --- |
| `bin/nyx-shell-core`, `bin/nyx-shellctl` | The daemon and its command line |
| `lib/systemd/user/nyx-shell-core.service` | `Type=notify`, `Restart=on-failure`, at most 5 starts a minute |
| `lib/systemd/user/nyx-shell-qs.service` | The Quickshell runtime (`Wants=`/`After=` the core, not bound to it) |
| `share/nyx-shell/qml` | The `Nyx.Shell` module, `shell.qml`, the examples |
| `share/nyx-shell/shell.toml` | The NyxOS defaults, with Quickshell's store path as the trusted UI runtime |

Only `nyx-shell-core` starts with the session (`passthru.autostart`). `nyx-shell-qs`
is installed but not started: with no components in it, it would cost Quickshell's
56 MiB for nothing ([PERFORMANCE.md](PERFORMANCE.md)).

## Switching it on

The changes to nyx-packages and nyxos are in `packaging/` as patches against their
heads of 2026-11-07, kept out of those repositories while other work was in flight
there:

```sh
git -C ~/Projects/nyx-packages apply ~/Projects/nyx-shell/packaging/nyx-packages.patch
git -C ~/Projects/nyxos apply ~/Projects/nyx-shell/packaging/nyxos.patch
# nyx-packages gains the nyx-shell input: lock it (in the VM: scripts/dev-vm packages flake lock),
# then nyxos takes the new nyx-packages (nix flake update nyx-packages) before it can use nyx.shell.
```

The NixOS module (nyx-packages `modules/nixos/default.nix`) gets an app entry:

```nix
shell = "Nyx Shell Core and the Nyx.Shell Quickshell module";
```

and, when enabled, the defaults layer:

```nix
(lib.mkIf cfg.shell.enable {
  environment.etc."nyxos/defaults/shell.toml".source = "${nyxPkgs.nyx-shell}/share/nyx-shell/shell.toml";
})
```

The package's `bin` joins `/etc/nyx/trusted-bin` like every enabled Nyx app, which is
how Shell Core recognizes `nyx-shellctl` (all capabilities) and Nyx Settings
(`config.write`).

A host turns it on with `nyx.shell.enable = true;`. NyxOS's `modules/nixos/apps.nix`
leaves it off (`lib.mkDefault false`) until Handoff 03.

## Hyprland configuration

`nyxos/modules/home/hyprland.nix`, when `osConfig.nyx.shell.enable`:

```lua
-- Hyprland's event socket doesn't report monitor mode, scale or position changes;
-- this posts them as custom>>nyx-monitors, which nyx-desktop reads.
hl.on("monitor.layout_changed", function() hl.dispatch(hl.dsp.event("nyx-monitors")) end)

-- Shell Core's shortcuts, written by `nyx-shellctl shortcuts --apply` from [shortcuts].
local nyx_shell = io.open((os.getenv("HOME") or "") .. "/.config/hypr/nyx-shell.lua")
if nyx_shell then
  nyx_shell:close()
  dofile((os.getenv("HOME") or "") .. "/.config/hypr/nyx-shell.lua")
end
```

NyxOS ships no `[shortcuts]`: the keys `hyprland.nix` binds (Super+Space for Glimpse)
stay owned there, so no key has two owners. When a component moves to Shell Core, its
key moves into `[shortcuts]` and out of `hyprland.nix` in the same change.

## Session order

1. greetd → uwsm → Hyprland; `graphical-session.target`.
2. `nyx-shell-core.service`: lock, socket, watcher, settings, theme; `READY=1` after
   the first compositor snapshot (or 2 s).
3. `nyx-shell-qs.service` when enabled: Quickshell connects, components register.
4. The GTK panels start as before, independently; Shell Core follows their bus names.

Logout stops the target; Shell Core tells clients `Shutdown` and removes its socket.

## Checking it

```sh
systemctl --user status nyx-shell-core
nyx-shellctl status                 # state, Hyprland, components, overlays, settings errors
nyx-shellctl watch --latency        # events as they happen
journalctl --user -u nyx-shell-core
```

The examples, for trying overlays:

```sh
systemctl --user edit nyx-shell-qs  # [Service] Environment=NYX_SHELL_EXAMPLES=1
systemctl --user start nyx-shell-qs
nyx-shellctl invoke shell.inspector.toggle
nyx-shellctl invoke shell.osd-sample.show '{"level": 40}'
```

## Validation

Done on the laptop (Omarchy, read-only, a headless output; see PERFORMANCE.md):
startup and shutdown, live state against Hyprland, hotplug, the `Panel1` bus names,
theme events, the Quickshell runtime connecting, registering, reconnecting after a
Shell Core restart, overlay policy between a transient and a passive overlay.

Done in the NyxOS VM (2026-11-07, `rebuild test` from `packaging/*.patch` with
`nyx.shell.enable` on the dev host, Quickshell 0.3.0 from nixpkgs):

- The package builds and its 86 tests pass in the Nix sandbox.
- `nyx-shell-core.service` reaches `active` through `READY=1` (4.9 MB); the GTK panels'
  bus names show them `ready`.
- Theme from Theme Engine 2.0's snapshot (`source: theme-engine`, the engine's
  generation, Nyx Dark's colours in the inspector).
- Actions: Control Center and Notification Center open and close over `Panel1`, the
  window overview opens over the switcher's socket.
- Overlay exclusion both ways: the Control Center opening closes the inspector; the
  inspector opening makes the Control Center close itself.
- Focus: nothing dispatched when Hyprland gives the keyboard back itself (above).
- The monitor hook: a runtime scale change (1.0 → 1.25 → 1.0) reaches Shell Core.
- Hotplug of a headless output.
- `kill -9` of the core: systemd restarts it. Six kills in a minute: `start-limit-hit`,
  and the GTK panels keep answering. `kill -9` of Quickshell: restarted, components
  `ready` again.
- Found and fixed there: Quickshell 0.3.0 didn't connect a `Socket` again after a refused
  attempt, so the runtime stayed disconnected after a core crash; `ShellCore` now makes
  a new socket for each attempt. Checked again after the fix: two `kill -9`s of the core,
  components `ready` again each time without touching Quickshell.
- Shortcuts: `SUPER + ALT + I` written, bound (`Nyx Shell: shell.inspector.toggle`) and
  working; `SUPER + SPACE` refused as Glimpse's, nothing written.
- Security: root (which file permissions don't stop) refused by the uid check and
  logged; `nobody` stopped by the 0700 folder; an unrecognized program of the same user
  (`nc`) refused `ShowOverlay`, `InvokeAction` and `UpdateShellConfig` (`forbidden`).
- Found there: Nyx Locker didn't set logind's `LockedHint`, so Shell Core couldn't see
  the lock. Fixed in nyx-greeter `ab65ecd`; Shell Core also had to watch the session's
  own path, not `session/auto`, to hear the change.

The full list, for a run of its own (`nyxos/scripts/dev-vm`), where Shell Core may act:

| Check | How | Expect |
| --- | --- | --- |
| Starts with the session | reboot, `nyx-shellctl status` | `ready`, Hyprland connected, the GTK panels `ready` |
| Actions | `nyx-shellctl invoke shell.control-center.toggle` (and notification-center, switcher, glimpse) | each opens; `actions` lists them available |
| Overlay exclusion with GTK panels | inspector open, then Super+D; then the reverse | the other closes each time |
| Focus restoration | focus a window, open the inspector with the keyboard, Escape | focus back on the window; clicking another window while open keeps that one |
| Monitor hook | `hyprctl eval 'hl.monitor({ output = "Virtual-1", scale = 1.25 })'` | a delta with the new scale |
| Multi-monitor | `hyprctl output create headless`, then remove it | monitor added and removed; overlays move to the active monitor |
| Crash recovery | `kill -9` the core; `kill -9` Quickshell | systemd restarts each; Quickshell reconnects and registers again; components `failed` then `ready` |
| Crash loop | `kill -9` the core 6 times within a minute | systemd stops restarting it; panels keep working |
| Lock | lock with the keybind | every overlay hidden, `locked` in status, `ShowOverlay` refused; unlock restores |
| Shortcuts | `[shortcuts] "shell.inspector.toggle" = "SUPER + ALT + I"`, `nyx-shellctl shortcuts --apply` | bind works; a key already bound is reported and nothing is written |
| Security | a script as another user connects | refused; as the same user, read-only |
| Theme | change the theme in Nyx Settings (or `nyx-theme set …`) | `ThemeChanged` with `source: theme-engine` and the engine's generation; surfaces restyle |
