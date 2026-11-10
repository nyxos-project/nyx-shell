pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

// The connection to Nyx Shell Core (docs/PROTOCOL.md): one socket per Quickshell
// process, the six topics subscribed, state kept current from snapshots and deltas,
// reconnecting with backoff. Surfaces bind to `desktop`, `overlays`, `theme`, `config`.
// Each attempt uses a new Socket: some Quickshell versions don't connect a Socket
// again after a refused attempt.
//
// Where Shell Core isn't installed (Omarchy), the same API runs standalone
// (Standalone.qml). NYX_SHELL_MODE picks: `core` waits for Shell Core however long it
// takes (NyxOS), `standalone` never connects, and unset tries Shell Core once and
// goes standalone if nothing answers.
Singleton {
    id: root

    readonly property string mode: Quickshell.env("NYX_SHELL_MODE") || "auto"
    property bool standalone: mode === "standalone"
    property var _local: null

    readonly property string socketPath: {
        const s = Quickshell.env("NYX_SHELL_SOCKET");
        if (s)
            return s;
        const r = Quickshell.env("XDG_RUNTIME_DIR");
        return r ? r + "/nyx-shell/core.sock" : "";
    }
    property string clientName: "nyx-shell-qs"

    // True once connected and every snapshot has arrived.
    property bool ready: false
    property var sock: null
    readonly property bool connected: sock !== null && sock.connected
    property string lastError: ""
    property var hello: null
    property var session: null
    property var desktop: ({ generation: 0, connected: false, monitors: [], workspaces: [], windows: [], active: {} })
    property var overlays: ({})
    property var actions: []
    property var config: null
    property var theme: null

    // After every (re)connection, once the snapshots are in: register again.
    signal reconnected
    signal actionInvoked(string action, var args, int invocation)
    signal componentChanged(var component)

    property int _next: 0
    property var _pending: ({})
    property int _seq: 0
    property var _since: ({})
    property int _backoff: 250
    property int _waiting: 0

    function request(op, body, callback) {
        if (root._local) {
            root._local.request(op, body, callback);
            return;
        }
        const sock = root.sock;
        if (!sock || !sock.connected) {
            if (callback)
                callback(null, { code: "unavailable", msg: "not connected to Nyx Shell Core" });
            return;
        }
        root._next += 1;
        const id = root._next;
        if (callback)
            root._pending[id] = callback;
        const req = { v: 1, id: id, op: op };
        if (body !== undefined && body !== null)
            req.body = body;
        sock.write(JSON.stringify(req) + "\n");
        sock.flush();
    }

    function overlay(id) {
        return root.overlays[id] || null;
    }
    function showOverlay(id, monitor) {
        request("ShowOverlay", monitor ? { id: id, monitor: monitor } : { id: id }, _merge);
    }
    function hideOverlay(id) {
        request("HideOverlay", { id: id }, _merge);
    }
    function toggleOverlay(id) {
        request("ToggleOverlay", { id: id }, _merge);
    }
    // Pin a persistent overlay at `size` ([w, h]): Shell Core shows it and places it.
    function pinOverlay(id, size, monitor, callback) {
        const b = { id: id, pinned: true, size: size };
        if (monitor)
            b.monitor = monitor;
        request("PinOverlay", b, (ok, err) => {
            _merge(ok, err);
            if (callback)
                callback(ok, err);
        });
    }
    function unpinOverlay(id) {
        request("PinOverlay", { id: id, pinned: false }, _merge);
    }
    function invoke(action, args) {
        request("InvokeAction", args ? { id: action, args: args } : { id: action }, null);
    }
    function windowAction(address, action, extra) {
        request("RequestWindowAction", Object.assign({ window: address, action: action }, extra || {}), null);
    }
    function focusWorkspace(workspace) {
        request("RequestWorkspaceAction", { action: "focus", workspace: String(workspace) }, null);
    }

    function _merge(ok, err) {
        if (!ok || !ok.overlay)
            return;
        const cur = root.overlays[ok.overlay.id];
        if (cur && cur.revision > ok.overlay.revision && cur.owner === ok.overlay.owner)
            return;
        const o = Object.assign({}, root.overlays);
        o[ok.overlay.id] = ok.overlay;
        root.overlays = o;
    }

    function _overlayList(list) {
        const o = {};
        for (const s of list)
            o[s.id] = s;
        root.overlays = o;
    }

    function _patch(list, change, key) {
        const out = list.filter(x => change.remove.indexOf(x[key]) < 0);
        for (const u of change.upsert) {
            const i = out.findIndex(x => x[key] === u[key]);
            if (i >= 0)
                out[i] = u;
            else
                out.push(u);
        }
        return out;
    }

    function _applyDelta(d) {
        if (d.base !== root.desktop.generation) {
            _fetch("desktop");
            return;
        }
        const cur = root.desktop;
        root.desktop = {
            generation: d.generation,
            connected: d.connected === null ? cur.connected : d.connected,
            monitors: _patch(cur.monitors, d.monitors, "name"),
            workspaces: _patch(cur.workspaces, d.workspaces, "id").sort((a, b) => a.id - b.id),
            windows: _patch(cur.windows, d.windows, "address").sort((a, b) => a.focus_order - b.focus_order),
            active: d.active === null ? cur.active : d.active
        };
    }

    function _fetch(topic) {
        switch (topic) {
        case "session":
            request("GetSessionStatus", null, ok => { if (ok) root.session = ok; });
            break;
        case "desktop":
            request("GetDesktopSnapshot", null, ok => { if (ok) root.desktop = ok; });
            break;
        case "overlays":
            request("GetOverlayState", null, ok => { if (ok) _overlayList(ok.overlays); });
            break;
        case "actions":
            request("ListActions", null, ok => { if (ok) root.actions = ok.actions; });
            break;
        case "config":
            request("GetShellConfig", null, ok => { if (ok) root.config = ok; });
            break;
        case "theme":
            request("GetThemeGeneration", null, ok => { if (ok) root.theme = ok; });
            break;
        }
    }

    function _subscribe() {
        const subs = [["SubscribeSessionEvents", "session", ok => root.session = ok.status], ["SubscribeDesktopChanges", "desktop", ok => root.desktop = ok.state], ["SubscribeOverlayState", "overlays", ok => _overlayList(ok.overlays)], ["SubscribeActionAvailability", "actions", ok => root.actions = ok.actions], ["SubscribeConfigChanges", "config", ok => root.config = ok.config], ["SubscribeThemeChanges", "theme", ok => root.theme = ok.theme]];
        root._waiting = subs.length;
        for (const s of subs) {
            const topic = s[1], take = s[2];
            request(s[0], null, (ok, err) => {
                if (ok) {
                    root._since[topic] = ok.seq;
                    take(ok);
                }
                root._waiting -= 1;
                if (root._waiting === 0) {
                    root.ready = true;
                    root.reconnected();
                }
            });
        }
    }

    function _stale(topic, seq) {
        return (root._since[topic] || 0) >= seq;
    }

    function _event(m) {
        if (root._seq !== 0 && m.seq !== root._seq + 1) {
            console.warn("nyx-shell: event", m.seq, "after", root._seq, "; fetching again");
            for (const t of ["session", "desktop", "overlays", "actions", "config", "theme"])
                _fetch(t);
        }
        root._seq = m.seq;
        const b = m.body;
        switch (m.ev) {
        case "DesktopChanged":
            if (!_stale("desktop", m.seq))
                _applyDelta(b);
            break;
        case "OverlayChanged":
            if (!_stale("overlays", m.seq)) {
                const o = Object.assign({}, root.overlays);
                o[b.overlay.id] = b.overlay;
                root.overlays = o;
            }
            break;
        case "OverlayRemoved":
            if (!_stale("overlays", m.seq)) {
                const o = Object.assign({}, root.overlays);
                delete o[b.id];
                root.overlays = o;
            }
            break;
        case "SessionChanged":
            root.session = b.status;
            break;
        case "ComponentChanged":
            root.componentChanged(b.component);
            break;
        case "ActionsChanged":
            root.actions = b.actions;
            break;
        case "ConfigChanged":
            root.config = b.config;
            break;
        case "ThemeChanged":
            root.theme = b.theme;
            break;
        case "Resync":
            for (const t of b.topics)
                _fetch(t);
            break;
        case "ActionInvoked":
            root.actionInvoked(b.action, b.args, b.invocation);
            break;
        case "Shutdown":
            root.lastError = "Nyx Shell Core is stopping";
            break;
        }
    }

    function _reply(m) {
        const cb = root._pending[m.id];
        delete root._pending[m.id];
        if (m.err) {
            root.lastError = m.err.code + ": " + m.err.msg;
            if (!cb)
                console.warn("nyx-shell:", root.lastError);
        }
        if (cb)
            cb(m.err ? null : m.ok, m.err || null);
    }

    function _reset() {
        const pending = root._pending;
        root._pending = {};
        for (const id in pending)
            pending[id](null, { code: "unavailable", msg: "disconnected" });
        root.ready = false;
        root._seq = 0;
        root._since = {};
    }

    // Called by Standalone.qml.
    function _overlayChanged(s) {
        const o = Object.assign({}, root.overlays);
        o[s.id] = s;
        root.overlays = o;
    }
    function _overlayRemoved(id) {
        const o = Object.assign({}, root.overlays);
        delete o[id];
        root.overlays = o;
    }
    function _standaloneReady() {
        root.hello = { protocol: 1, server: "standalone" };
        root.ready = true;
        root.reconnected();
    }

    function _goStandalone() {
        if (root._local)
            return;
        if (root.sock) {
            const old = root.sock;
            root.sock = null;
            old.destroy();
        }
        root.standalone = true;
        root.lastError = "";
        root._local = standaloneComponent.createObject(root, { core: root });
    }

    Component {
        id: standaloneComponent
        Standalone {}
    }

    function _connect() {
        if (root.standalone) {
            _goStandalone();
            return;
        }
        if (root.sock) {
            const old = root.sock;
            root.sock = null;
            old.destroy();
        }
        if (root.socketPath === "") {
            if (root.mode !== "core")
                _goStandalone();
            return;
        }
        // Assigned before connecting, so a refusal reported at once isn't ignored.
        const s = socketComponent.createObject(root);
        root.sock = s;
        s.connected = true;
    }

    function _line(data) {
        let m;
        try {
            m = JSON.parse(data);
        } catch (e) {
            root.lastError = "unreadable line from Nyx Shell Core";
            return;
        }
        if (m.ev !== undefined)
            root._event(m);
        else
            root._reply(m);
    }

    Component {
        id: socketComponent
        Socket {
            id: socket
            path: root.socketPath
            connected: false
            parser: SplitParser {
                splitMarker: "\n"
                onRead: data => {
                    if (socket === root.sock)
                        root._line(data);
                }
            }
            onConnectionStateChanged: {
                if (socket !== root.sock)
                    return;
                if (connected) {
                    root._backoff = 250;
                    root.lastError = "";
                    root.request("Hello", { client: root.clientName, min: 1, max: 1 }, ok => root.hello = ok);
                    root._subscribe();
                } else {
                    root._reset();
                    if (root.mode === "auto" && !root.hello)
                        root._goStandalone();
                    else
                        retry.restart();
                }
            }
            onError: error => {
                if (socket !== root.sock)
                    return;
                if (root.mode === "auto" && !root.hello) {
                    root._goStandalone();
                    return;
                }
                root.lastError = "can't reach Nyx Shell Core at " + root.socketPath;
                retry.restart();
            }
        }
    }

    Timer {
        id: retry
        interval: root._backoff
        onTriggered: {
            root._backoff = Math.min(root._backoff * 2, 5000);
            root._connect();
        }
    }

    Component.onCompleted: _connect()
}
