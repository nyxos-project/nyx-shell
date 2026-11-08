# Nyx Shell

The coordination layer of the NyxOS shell: **Nyx Shell Core**, a small Rust daemon that
holds the one Hyprland model and serves it to shell surfaces, and **`Nyx.Shell`**, the
Quickshell module those surfaces are built with.

- One normalized desktop state (monitors, workspaces, windows) with generation numbers,
  snapshots and deltas.
- Overlay policy: groups, fullscreen and lock rules, placement; the GTK panels join it
  through their existing `Panel1` D-Bus interface.
- Focus given back when a transient overlay closes, only when it's safe.
- Fixed action routes (no command execution), optional shortcuts in an owned Hyprland
  file.
- Component registry and health, the `shell` settings domain, theme events.
- A versioned Unix-socket protocol that Quickshell speaks directly.

NyxOS-only: shipped through nyx-packages, not installed on Omarchy.

| | |
| --- | --- |
| [docs/AUDIT.md](docs/AUDIT.md) | What existed before, and what's reused |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Layers, modules, process boundaries, recovery |
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | IPC version 1 |
| [docs/SECURITY.md](docs/SECURITY.md) | Trust boundaries and capabilities |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | Measurements against what runs today |
| [docs/NYXOS.md](docs/NYXOS.md) | Integration in NyxOS |
| [docs/HANDOFF-03.md](docs/HANDOFF-03.md) | Notes for moving components to Quickshell |

## Layout

- `crates/nyx-shell-proto`: the protocol types and the settings schema (no runtime).
- `crates/nyx-shell-core`: the coordinator library.
- `apps/shell-core`: `nyx-shell-core`, the daemon.
- `apps/shellctl`: `nyx-shellctl`, diagnostics, tests and keybinds.
- `qml/Nyx/Shell`: the shared Quickshell module; `qml/shell.qml` is the runtime
  (`nyx-shell-qs`), `qml/examples` the inspector and OSD examples.
- `data`: systemd user units and the NyxOS defaults for `shell.toml`.
- `tools`: measurement scripts.

## Build and test

```sh
cargo build
cargo clippy --all-targets
cargo test            # unit tests, an end-to-end test against a fake Hyprland, a burst test
```

Theme events take their colours from Theme Engine 2.0's snapshot. `--features palette`
also resolves them without the engine, by linking `nyx-theme` (GTK).

## Trying it without changing the desktop

```sh
NYX_SHELL_SOCKET=/tmp/nyx-shell/core.sock target/debug/nyx-shell-core --read-only &
NYX_SHELL_SOCKET=/tmp/nyx-shell/core.sock target/debug/nyx-shellctl status
```

Read-only mode follows the desktop and never acts on it. Quickshell surfaces can be
pinned to a headless output with `NYX_SHELL_SCREEN=HEADLESS-N` and kept off the
keyboard with `NYX_SHELL_NO_KEYBOARD=1`:

```sh
QML_IMPORT_PATH=$PWD/qml NYX_SHELL_EXAMPLES=1 NYX_SHELL_SCREEN=HEADLESS-N \
  NYX_SHELL_NO_KEYBOARD=1 QS_NO_RELOAD_POPUP=1 QS_DISABLE_CRASH_HANDLER=1 \
  NYX_SHELL_SOCKET=/tmp/nyx-shell/core.sock qs -p qml/shell.qml
```
