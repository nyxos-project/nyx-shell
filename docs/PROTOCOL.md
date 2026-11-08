# Nyx Shell Core IPC, version 1

The contract between `nyx-shell-core` and its clients. Types are defined once, in
`crates/nyx-shell-proto`; this document describes the wire format they produce.

## Transport

- Unix stream socket `$XDG_RUNTIME_DIR/nyx-shell/core.sock` (the folder is 0700, the
  socket 0600). `NYX_SHELL_SOCKET` overrides the path for tests.
- One JSON object per line, UTF-8, `\n`-terminated. A line longer than 1 MiB is refused
  with `too_large` and the connection is closed.
- The server checks the peer's uid (`SO_PEERCRED`) and refuses other users. What a client
  may do depends on the program it runs ([SECURITY.md](SECURITY.md)).

## Frames

Request (client to server):

```json
{"v": 1, "id": 7, "op": "GetDesktopSnapshot", "body": {}}
```

- `v`: the protocol version the client speaks. Outside the server's range
  (`MIN_PROTOCOL_VERSION ..= PROTOCOL_VERSION`, currently `1..=1`) the reply is
  `unsupported_version`.
- `id`: chosen by the client, echoed in the reply.
- `body`: the operation's argument object; may be omitted when it has no fields.

Reply:

```json
{"id": 7, "ok": { ... }}
{"id": 7, "err": {"code": "forbidden", "msg": "needs desktop.act"}}
```

Event (server to client, only for topics the client subscribed to, plus `Resync`,
`ActionInvoked` and `Shutdown`):

```json
{"ev": "DesktopChanged", "seq": 12, "ts": 1794100000123456, "body": { ... }}
```

- `seq` counts events on this connection from 1, with no gaps unless the server says
  so with `Resync`. A client that sees a gap fetches snapshots again.
- `ts`: microseconds since the Unix epoch when the server produced the event (for
  latency measurement).

## Error codes

| Code | Meaning |
| --- | --- |
| `bad_request` | Not JSON, missing `v`/`id`/`op`, or a body that doesn't fit the operation |
| `unsupported_version` | `v` outside the server's range |
| `unknown_op` | No such operation |
| `forbidden` | The client lacks the capability |
| `not_found` | No such window, workspace, monitor, overlay, component or action |
| `conflict` | The id is held by another live connection |
| `invalid` | Well-formed but not allowed (a passive overlay asking for the keyboard, a config value that fails validation) |
| `unavailable` | The target isn't running (a GTK panel, Hyprland) |
| `read_only` | The server runs with `--read-only` and won't dispatch |
| `locked` | The session is locked |
| `too_large` | Line over 1 MiB |
| `internal` | A bug; the message says what |

## Handshake

`Hello` is optional; a client that skips it is named `unknown`.

```json
{"v":1,"id":1,"op":"Hello","body":{"client":"nyx-shell-qs","min":1,"max":1}}
```

Reply: `{"protocol":1,"server":"nyx-shell-core 0.1.0","session":"…","capabilities":["desktop.read",…],"read_only":false}`.
If the client's range doesn't overlap the server's, the reply is `unsupported_version`.

## Topics and subscriptions

| Subscribe op | Topic | Reply carries | Events |
| --- | --- | --- | --- |
| `SubscribeSessionEvents` | `session` | `status` | `SessionChanged {status}`, `ComponentChanged {component}` |
| `SubscribeDesktopChanges` | `desktop` | `state` | `DesktopChanged {delta}` |
| `SubscribeOverlayState` | `overlays` | `overlays` | `OverlayChanged {overlay}`, `OverlayRemoved {id}` |
| `SubscribeActionAvailability` | `actions` | `actions` | `ActionsChanged {actions}` |
| `SubscribeConfigChanges` | `config` | `config` | `ConfigChanged {config}` |
| `SubscribeThemeChanges` | `theme` | `theme` | `ThemeChanged {theme}` |

Every subscribe reply also has `seq`: the last event number sent on this connection
before the snapshot, so events with a higher `seq` are newer than the snapshot.
`Unsubscribe {topic}` stops a topic.

Always sent, unsubscribed:

- `Resync {topics}`: events were dropped because the client fell behind; fetch these
  topics again.
- `ActionInvoked {action, args, invocation}`: to the component that owns an action.
- `Shutdown {}`: the server is stopping.

## Operations

Capabilities are in brackets.

### Session

- `GetSessionStatus` [desktop.read] → `SessionStatus`:

```json
{"version":"0.1.0","session":"…","state":"ready","read_only":false,"uptime_ms":1234,
 "locked":false,
 "compositor":{"connected":true,"generation":42,"last_event_ms_ago":15},
 "ipc":{"clients":2,"dropped_events":0,"resyncs":0},
 "components":[ComponentStatus…],
 "overlays":[{"id":"nyx.inspector","owner":"client:nyx-shell-qs","visible":false}],
 "config":{"generation":3,"errors":[]},
 "theme":{"generation":2}}
```

  `state` is `starting`, `ready`, `degraded` (Hyprland unreachable) or `stopping`.

### Desktop

- `GetDesktopSnapshot` [desktop.read] → `DesktopState`:

```json
{"generation":42,"connected":true,
 "monitors":[{"name":"eDP-2","stable_id":"Samsung Display Corp.|ATNA60DL01-0|",
   "description":"…","x":0,"y":0,"width":2560,"height":1600,
   "logical_width":1600,"logical_height":1000,"scale":1.6,"transform":0,
   "refresh_rate":120.0,"focused":true,"enabled":true,"virtual":false,
   "active_workspace":1,"special_workspace":null,"reserved":[0,26,0,0]}],
 "workspaces":[{"id":1,"name":"1","monitor":"eDP-2","active":true,"special":false,
   "user":true,"has_fullscreen":false,"windows":["55ef7100a730"]}],
 "windows":[{"address":"55ef7100a730","stable_id":"18000369","app_id":"firefox",
   "initial_app_id":"firefox","title":"…","workspace":1,"monitor":"eDP-2","focused":true,
   "floating":false,"fullscreen":"none","hidden":false,"minimized":false,"pinned":false,
   "urgent":false,"focus_order":0}],
 "active":{"monitor":"eDP-2","workspace":1,"window":"55ef7100a730"}}
```

  - Window addresses are lowercase hex without `0x`.
  - `fullscreen` is `none`, `maximized` or `fullscreen`.
  - `minimized`: the window is on Omarchy's `special:minimized` workspace.
  - `stable_id` for monitors is make|model|serial, or the connector name when the
    screen reports none. Connector names are not stable identities.
  - Titles are untrusted text, cut to 512 characters with control characters removed.
    Render them as plain text.
  - `refresh_rate` is `null` when Hyprland didn't report one.

- `SubscribeDesktopChanges` [desktop.read] → `{seq, state}`. Event `DesktopChanged`:

```json
{"generation":43,"base":42,"connected":null,
 "monitors":{"upsert":[],"remove":[]},
 "workspaces":{"upsert":[{…}],"remove":[]},
 "windows":{"upsert":[{…}],"remove":["abc123"]},
 "active":{"monitor":"eDP-2","workspace":2,"window":null}}
```

  Fields that didn't change are `null` (`connected`, `active`) or empty lists. Apply
  only when `base` equals your generation; otherwise call `GetDesktopSnapshot`.

- `RequestWorkspaceAction` [desktop.act] `{action, workspace?, name?}`:
  - `{"action":"focus","workspace":"3"}`: show workspace 3 (an id, or `name:…`).
  - `{"action":"toggle_special","name":"scratch"}`: show or hide a special workspace.
- `RequestWindowAction` [desktop.act] `{window, action, workspace?, monitor?, follow?}`:
  `focus`, `close`, `toggle_floating`, `toggle_maximized`,
  `move_to_workspace` (`workspace`, `follow`), `move_to_monitor` (`monitor`).
  The window and monitor must exist in the current state; workspace strings are
  limited to ids, `name:` and `special:` names of `[A-Za-z0-9_-]`.

Both reply `{}` once Hyprland accepted the dispatch, `read_only` in read-only mode,
`unavailable` when Hyprland is unreachable.

### Overlays

`OverlayPolicy`:

```json
{"id":"nyx.inspector","category":"transient","group":"panels","keyboard":"exclusive",
 "fullscreen":"show","placement":"active","restore_focus":true}
```

- `id`: `[a-z0-9._:-]`, at most 64 characters.
- `category`: `passive`, `transient`, `persistent`, `presentation`, `secure-external`.
- `keyboard`: `none`, `on-demand`, `exclusive`. Passive and presentation overlays must
  use `none`.
- `fullscreen`: `show`, `hide`, `block`.
- `placement`: `active`, `pointer`, `all`, `monitor:NAME`.
- Defaults: `group` none (`panels` for transient), `keyboard` `none`, `fullscreen`
  `hide`, `placement` `active`, `restore_focus` true for transient overlays.
- `[overlays.ID]` in `shell.toml` overrides `group`, `fullscreen`, `placement` and
  `restore_focus` of a registered overlay.

`OverlayState`:

```json
{"id":"nyx.inspector","owner":"client:nyx-shell-qs","category":"transient",
 "group":"panels","keyboard":"exclusive","requested":true,"visible":true,
 "monitor":"eDP-2","reason":null,"revision":5}
```

`visible` is what the surface should do. `requested` is what was asked. When they
differ, `reason` says why: `locked`, `fullscreen`. `monitor` is `null` for `all`.

- `RegisterOverlay` [overlay.manage] `{policy}` → `{overlay}`. Owned by this connection;
  removed when it disconnects. Registering an id another live connection owns is
  `conflict`. `secure-external` can't be registered over IPC (`invalid`).
- `UnregisterOverlay` [overlay.manage] `{id}` → `{}` (owner only).
- `ShowOverlay` [overlay.manage] `{id, monitor?}` → `{overlay}`. Hides the others in its
  group. `locked` while locked; `invalid` for `block` over fullscreen.
- `HideOverlay` [overlay.manage] `{id}` → `{overlay}`.
- `ToggleOverlay` [overlay.manage] `{id}` → `{overlay}`.
- `GetOverlayState` [desktop.read] → `{overlays}`.
- `SubscribeOverlayState` [desktop.read] → `{seq, overlays}`.

Built-in entries `control-center`, `notification-center` and `switcher` (owner
`panel1:BUSNAME`) are driven over D-Bus `Panel1`.

### Components

`ComponentManifest`:

```json
{"id":"nyx.inspector","kind":"surface","process":"nyx-shell-qs",
 "requires":[],"optional":[],"actions":["shell.inspector.toggle"],
 "overlays":["nyx.inspector"],"restart":"on-failure","capabilities":[]}
```

- `kind`: `surface`, `service`, `runtime`, `bridge`.
- `restart`: `never`, `on-failure`, `always` (descriptive; systemd restarts processes).

`ComponentStatus`: the manifest plus `state` (`registered`, `starting`, `ready`,
`degraded`, `stopped`, `failed`), `effective` (the state after dependencies: `degraded`
when a required dependency isn't ready), `detail`, `owner`, `since_ms`.

- `RegisterComponent` [component.register] `{manifest}` → `{component}` (state
  `starting`).
- `ReportReady` [component.register] `{id}` → `{component}`.
- `ReportHealth` [component.register] `{id, state, detail?}`; `state` is `ready`,
  `degraded` or `stopped` → `{component}`.
- `GetComponentStatus` [desktop.read] `{id?}` → `{components}`.

### Actions

`ActionInfo`: `{"id":"shell.control-center.toggle","title":"Control Center","owner":"panel1:io.github.design_nexus.nyx.ControlCenter","available":true}`.

- `ListActions` [desktop.read] → `{actions}`.
- `InvokeAction` [action.invoke] `{id, args?}` → `{routed}` where `routed` is `dbus`,
  `socket`, `launch`, `overlay` or `component:ID`. `args` is an object passed to
  components only, at most 4 KiB.
- `SubscribeActionAvailability` [desktop.read] → `{seq, actions}`.
- Owners receive `ActionInvoked {action, args, invocation}`.

### Configuration

- `GetShellConfig` [desktop.read] → `{generation, config, errors}`. `config` is the merged,
  validated `ShellConfig`; `errors` lists sections that fell back to defaults.
- `UpdateShellConfig` [config.write] `{key, value}` → `{generation}`. `key` is dotted
  (`focus.restore`); the result must validate or the reply is `invalid` and nothing is
  written.
- `ResetShellConfig` [config.write] `{key}` → `{generation}`: removes the user's value.
- `SubscribeConfigChanges` [desktop.read] → `{seq, config}`.

### Theme

- `GetThemeGeneration` [desktop.read] → `ThemeState`:

```json
{"generation":2,"mode":"omarchy","id":"omarchy","name":"Omarchy","light":false,
 "palette":{"bg":"#1a1b26","surface":"…","text":"…","dim_text":"…","accent":"…",
   "border":"…","muted":"…","highlight":"…","danger":"…","glow":"…","shadow":"…"}}
```

  `palette` is `null` in builds without the `palette` feature.
- `SubscribeThemeChanges` [desktop.read] → `{seq, theme}`.

### Other

- `Unsubscribe` [none] `{topic}` → `{}`.

## Versioning

- Adding an operation, an event, or an optional field is compatible and keeps `v` at 1.
  Clients ignore fields and events they don't know.
- Removing or changing the meaning of anything raises `PROTOCOL_VERSION`; the server
  keeps answering the previous version while clients move (`MIN_PROTOCOL_VERSION`).

## Mapping to the handoff's proposed names

The handoff's message families are implemented with the same names. Additions:
`Hello`, `UnregisterOverlay`, `GetOverlayState`, `ResetShellConfig`, `Unsubscribe`, and
the `ComponentChanged`, `Resync`, `ActionInvoked` and `Shutdown` events.
