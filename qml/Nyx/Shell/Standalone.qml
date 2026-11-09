import QtQuick
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io

// Nyx.Shell without Shell Core (Omarchy, where Shell Core isn't installed): the same
// desktop state, overlay rules and component registry, kept in this process.
//
// The desktop state has Shell Core's schema (docs/PROTOCOL.md), read from Hyprland
// through Quickshell's own connection; the overlay rules are those of Shell Core's
// overlay.rs (groups, fullscreen hide and block, placement). Window titles are
// untrusted text here too. There is no lock state and no focus restoration: Hyprland
// gives the keyboard back itself when a layer surface closes.
QtObject {
    id: local

    required property var core

    property int generation: 0
    property var overlayStates: ({})
    property var components: ({})
    property bool loaded: false

    function request(op, body, cb) {
        const reply = (ok, err) => {
            if (cb)
                cb(ok, err || null);
        };
        const b = body || {};
        switch (op) {
        case "Hello":
            return reply({ protocol: 1, server: "standalone", capabilities: [], read_only: false });
        case "GetDesktopSnapshot":
            return reply(core.desktop);
        case "GetOverlayState":
            return reply({ overlays: Object.values(overlayStates) });
        case "RegisterOverlay":
            return reply({ overlay: registerOverlay(b.policy) });
        case "UnregisterOverlay":
            unregisterOverlay(b.id);
            return reply({});
        case "ShowOverlay":
        case "HideOverlay":
        case "ToggleOverlay": {
            const cur = overlayStates[b.id];
            if (!cur)
                return reply(null, { code: "not_found", msg: "no overlay " + b.id });
            const show = op === "ShowOverlay" || (op === "ToggleOverlay" && !cur.requested);
            const err = setRequested(b.id, show, b.monitor || null);
            return err ? reply(null, err) : reply({ overlay: overlayStates[b.id] });
        }
        case "RegisterComponent":
            return reply({ component: setComponent(b.manifest.id, Object.assign({}, b.manifest, { state: "starting", effective: "starting" })) });
        case "ReportReady":
        case "ReportHealth": {
            const c = components[b.id];
            if (!c)
                return reply(null, { code: "not_found", msg: "no component " + b.id });
            const st = op === "ReportReady" ? "ready" : b.state;
            return reply({ component: setComponent(b.id, Object.assign({}, c, { state: st, effective: st, detail: b.detail || null })) });
        }
        case "InvokeAction": {
            const owner = Object.values(components).find(c => (c.actions || []).indexOf(b.id) >= 0);
            if (!owner)
                return reply(null, { code: "not_found", msg: "no action " + b.id });
            core.actionInvoked(b.id, b.args || null, 0);
            return reply({ routed: "component:" + owner.id });
        }
        case "RequestWindowAction":
            return windowAction(b) ? reply({}) : reply(null, { code: "not_found", msg: "no window " + b.window });
        case "RequestWorkspaceAction":
            return workspaceAction(b) ? reply({}) : reply(null, { code: "invalid", msg: "unknown workspace action" });
        case "GetShellConfig":
            return reply({ generation: 0, config: null, errors: [] });
        case "Unsubscribe":
            return reply({});
        default:
            return reply(null, { code: "unavailable", msg: op + " needs Nyx Shell Core" });
        }
    }

    function setComponent(id, c) {
        const m = Object.assign({}, components);
        m[id] = c;
        components = m;
        core.componentChanged(c);
        return c;
    }

    // Overlays ------------------------------------------------------------------

    function registerOverlay(p) {
        const transient = p.category === "transient";
        const s = {
            id: p.id,
            owner: "client:nyx-shell-qs",
            category: p.category,
            group: p.group !== undefined ? p.group : (transient ? "panels" : null),
            keyboard: p.keyboard || "none",
            fullscreen: p.fullscreen || "hide",
            placement: p.placement || "active",
            restore_focus: p.restore_focus !== undefined ? p.restore_focus : transient,
            requested: false,
            visible: false,
            monitor: null,
            reason: null,
            revision: 0
        };
        const cur = overlayStates[p.id];
        if (cur) {
            s.requested = cur.requested;
            s.monitor = cur.monitor;
            s.revision = cur.revision;
        }
        putOverlay(evaluate(s));
        return overlayStates[p.id];
    }

    function unregisterOverlay(id) {
        if (!overlayStates[id])
            return;
        const o = Object.assign({}, overlayStates);
        delete o[id];
        overlayStates = o;
        core._overlayRemoved(id);
    }

    function putOverlay(s) {
        const o = Object.assign({}, overlayStates);
        o[s.id] = s;
        overlayStates = o;
        core._overlayChanged(s);
    }

    function placementMonitor(s, monitor) {
        if (monitor)
            return monitor;
        if (s.placement === "all")
            return null;
        if (s.placement.indexOf("monitor:") === 0)
            return s.placement.substr(8);
        return (core.desktop.active && core.desktop.active.monitor) || null;
    }

    function fullscreenOn(name) {
        const d = core.desktop;
        const m = d.monitors.find(x => x.name === name);
        if (!m)
            return false;
        return d.windows.some(w => !w.hidden && w.fullscreen === "fullscreen" && (w.workspace === m.active_workspace || w.workspace === m.special_workspace));
    }

    function evaluate(s) {
        const out = Object.assign({}, s);
        let reason = null;
        if (out.requested && out.fullscreen === "hide") {
            const mons = out.monitor ? [out.monitor] : core.desktop.monitors.map(m => m.name);
            if (mons.length > 0 && mons.every(n => fullscreenOn(n)))
                reason = "fullscreen";
        }
        out.reason = reason;
        out.visible = out.requested && reason === null;
        return out;
    }

    function setRequested(id, show, monitor) {
        const cur = overlayStates[id];
        if (show) {
            const mon = placementMonitor(cur, monitor);
            if (cur.fullscreen === "block" && mon && fullscreenOn(mon))
                return { code: "invalid", msg: "a fullscreen window is on " + mon };
            if (cur.category !== "passive" && cur.group) {
                for (const other of Object.values(overlayStates)) {
                    if (other.id !== id && other.group === cur.group && other.requested)
                        putOverlay(evaluate(Object.assign({}, other, { requested: false, revision: other.revision + 1 })));
                }
            }
            putOverlay(evaluate(Object.assign({}, cur, { requested: true, monitor: mon, revision: cur.revision + 1 })));
        } else if (cur.requested) {
            putOverlay(evaluate(Object.assign({}, cur, { requested: false, revision: cur.revision + 1 })));
        }
        return null;
    }

    // Fullscreen windows coming and going change what may be shown.
    function reevaluate() {
        for (const s of Object.values(overlayStates)) {
            const n = evaluate(s);
            if (n.visible !== s.visible || n.reason !== s.reason)
                putOverlay(Object.assign(n, { revision: s.revision + 1 }));
        }
    }

    // Actions on the compositor --------------------------------------------------

    // The Lua form first (Hyprland 0.55+), the classic one when it's refused, as
    // nyx-desktop's actions do.
    function dispatch(lua, classic) {
        hypr("dispatch " + lua, reply => {
            if (reply.trim() !== "ok")
                hypr("dispatch " + classic, () => {});
        });
    }

    // One request on Hyprland's command socket; `cb` gets the whole reply.
    function hypr(command, cb) {
        const s = _requestComponent.createObject(local, { command: command, callback: cb });
        s.connected = true;
    }

    property Component _requestComponent: Component {
        Socket {
            id: req
            property string command
            property var callback
            property string buf: ""
            path: Hyprland.requestSocketPath
            parser: SplitParser {
                splitMarker: ""
                onRead: data => req.buf += data
            }
            // Hyprland closes the socket after its reply, which Qt reports as an error
            // before the disconnection; finish once, after what was read so far.
            function finish() {
                const cb = callback;
                callback = null;
                if (cb) {
                    cb(buf);
                    destroy();
                }
            }
            onConnectedChanged: {
                if (connected) {
                    write(command);
                    flush();
                } else {
                    Qt.callLater(finish);
                }
            }
            onError: Qt.callLater(finish)
        }
    }

    function windowAction(b) {
        const w = core.desktop.windows.find(x => x.address === b.window);
        if (!w)
            return false;
        const t = "window = \"address:0x" + w.address + "\"";
        switch (b.action) {
        case "focus":
            dispatch("hl.dsp.focus({ " + t + " })", "focuswindow address:0x" + w.address);
            break;
        case "close":
            dispatch("hl.dsp.window.close({ " + t + " })", "closewindow address:0x" + w.address);
            break;
        case "toggle_floating":
            dispatch("hl.dsp.window.float({ action = \"toggle\", " + t + " })", "togglefloating address:0x" + w.address);
            break;
        case "toggle_maximized":
            dispatch("hl.dsp.window.fullscreen({ mode = \"maximized\", action = \"toggle\", " + t + " })", "fullscreen 1");
            break;
        case "move_to_workspace": {
            const ws = String(b.workspace || "");
            if (!/^(-?[0-9]+|(name|special):[A-Za-z0-9_-]+)$/.test(ws))
                return false;
            const follow = b.follow === true;
            dispatch("hl.dsp.window.move({ workspace = \"" + ws + "\", follow = " + follow + ", " + t + " })", (follow ? "movetoworkspace " : "movetoworkspacesilent ") + ws + ",address:0x" + w.address);
            break;
        }
        default:
            return false;
        }
        return true;
    }

    function workspaceAction(b) {
        if (b.action === "focus") {
            const ws = String(b.workspace || "");
            if (!/^(-?[0-9]+|name:[A-Za-z0-9_-]+)$/.test(ws))
                return false;
            dispatch("hl.dsp.focus({ workspace = \"" + ws + "\" })", "workspace " + ws);
            return true;
        }
        if (b.action === "toggle_special") {
            const n = String(b.name || "");
            if (!/^[A-Za-z0-9_-]+$/.test(n))
                return false;
            dispatch("hl.dsp.workspace.toggle_special(\"" + n + "\")", "togglespecialworkspace " + n);
            return true;
        }
        return false;
    }

    // Desktop state ---------------------------------------------------------------

    function clean(text) {
        return String(text || "").replace(/[\u0000-\u001f\u007f]/g, "").substr(0, 512);
    }

    function addr(a) {
        return String(a || "").replace(/^0x/, "").toLowerCase();
    }

    property var _mons: []
    property var _wss: []
    property var _clients: []
    property string _focused: ""

    function build() {
        const mons = _mons, wss = _wss, tops = _clients;
        const monName = {};
        for (const m of mons)
            monName[m.id] = m.name;
        const shown = {};
        for (const m of mons) {
            if (m.activeWorkspace)
                shown[m.activeWorkspace.id] = true;
            if (m.specialWorkspace && m.specialWorkspace.id)
                shown[m.specialWorkspace.id] = true;
        }
        const minimized = (wss.find(w => w.name === "special:minimized") || {}).id;
        const focused = _focused;
        const windows = tops.filter(c => c.mapped !== false && c.address).map(c => {
            const fs = c.fullscreen === 2 || c.fullscreen === 3 || c.fullscreen === true ? "fullscreen" : (c.fullscreen === 1 ? "maximized" : "none");
            const a = addr(c.address);
            return {
                address: a,
                stable_id: c.stableId !== undefined ? String(c.stableId) : "",
                app_id: c.class || "",
                initial_app_id: c.initialClass || "",
                title: clean(c.title),
                workspace: c.workspace ? c.workspace.id : 0,
                monitor: monName[c.monitor] !== undefined ? monName[c.monitor] : null,
                focused: a === focused,
                floating: !!c.floating,
                fullscreen: fs,
                hidden: !!c.hidden,
                minimized: minimized !== undefined && !!c.workspace && c.workspace.id === minimized,
                pinned: !!c.pinned,
                urgent: _urgent[a] === true,
                focus_order: c.focusHistoryID !== undefined ? c.focusHistoryID : 0,
                at: c.at || [0, 0],
                size: c.size || [0, 0],
                pid: c.pid || 0
            };
        }).sort((a, b) => a.focus_order - b.focus_order);
        const monitors = mons.map(m => {
            const scale = m.scale > 0 ? m.scale : 1;
            let lw = Math.round(m.width / scale), lh = Math.round(m.height / scale);
            if (m.transform % 2 === 1)
                [lw, lh] = [lh, lw];
            const make = (m.make || "").trim(), model = (m.model || "").trim();
            return {
                name: m.name,
                stable_id: (make === "" && model === "") ? m.name : make + "|" + model + "|" + (m.serial || "").trim(),
                description: m.description || "",
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                logical_width: lw,
                logical_height: lh,
                scale: m.scale,
                transform: m.transform || 0,
                refresh_rate: m.refreshRate || null,
                focused: !!m.focused,
                enabled: !m.disabled,
                virtual: m.name.indexOf("HEADLESS-") === 0,
                active_workspace: m.activeWorkspace ? m.activeWorkspace.id : 0,
                special_workspace: (m.specialWorkspace && m.specialWorkspace.id) ? m.specialWorkspace.id : null,
                reserved: m.reserved || [0, 0, 0, 0]
            };
        });
        const workspaces = wss.map(w => ({
            id: w.id,
            name: w.name || String(w.id),
            monitor: w.monitor || "",
            active: !!shown[w.id],
            special: String(w.name || "").indexOf("special:") === 0,
            user: w.id >= 1 && w.id <= 10,
            has_fullscreen: !!w.hasfullscreen,
            windows: windows.filter(x => x.workspace === w.id).map(x => x.address)
        })).sort((a, b) => a.id - b.id);
        const fm = monitors.find(m => m.focused);
        const active = {
            monitor: fm ? fm.name : (monitors.length ? monitors[0].name : null),
            workspace: fm ? fm.active_workspace : null,
            window: focused || null
        };
        const next = { generation: 0, connected: true, monitors: monitors, workspaces: workspaces, windows: windows, active: active };
        const before = JSON.stringify(Object.assign({}, core.desktop, { generation: 0 }));
        if (JSON.stringify(next) !== before) {
            generation += 1;
            next.generation = generation;
            core.desktop = next;
            reevaluate();
        }
        if (!loaded) {
            loaded = true;
            core._standaloneReady();
        }
    }

    // Urgency comes only from events (`urgent>>ADDRESS`); focusing the window clears it.
    property var _urgent: ({})

    property bool _monitorsDirty: true
    property bool _workspacesDirty: true
    property bool _clientsDirty: true
    property int _pending: 0

    function _parse(text, fallback) {
        try {
            return JSON.parse(text);
        } catch (e) {
            return fallback;
        }
    }

    function refresh() {
        if (_pending > 0) {
            _batch.restart();
            return;
        }
        const jobs = [];
        if (_monitorsDirty)
            jobs.push(["j/monitors", t => local._mons = local._parse(t, local._mons)]);
        if (_workspacesDirty)
            jobs.push(["j/workspaces", t => local._wss = local._parse(t, local._wss)]);
        if (_clientsDirty) {
            jobs.push(["j/clients", t => local._clients = local._parse(t, local._clients)]);
            jobs.push(["j/activewindow", t => local._focused = local.addr((local._parse(t, {}) || {}).address)]);
        }
        _monitorsDirty = _workspacesDirty = _clientsDirty = false;
        if (jobs.length === 0)
            return;
        _pending = jobs.length;
        for (const j of jobs) {
            const take = j[1];
            hypr(j[0], text => {
                take(text);
                local._pending -= 1;
                if (local._pending === 0)
                    local.build();
            });
        }
    }

    property Timer _batch: Timer {
        interval: 30
        onTriggered: local.refresh()
    }

    // Hyprland's event socket, read directly (one line per event, NAME>>DATA).
    property Socket _events: Socket {
        path: Hyprland.eventSocketPath
        parser: SplitParser {
            splitMarker: "\n"
            onRead: line => local._event(line)
        }
        onConnectedChanged: {
            if (!connected)
                local._reconnect.restart();
        }
        onError: local._reconnect.restart()
    }

    property Timer _reconnect: Timer {
        interval: 1000
        onTriggered: {
            local._events.connected = false;
            local._events.connected = true;
            local._monitorsDirty = local._workspacesDirty = local._clientsDirty = true;
            local._batch.start();
        }
    }

    function _event(line) {
        const i = line.indexOf(">>");
        const n = i < 0 ? line : line.substr(0, i);
        const data = i < 0 ? "" : line.substr(i + 2);
        if (n === "urgent") {
            const u = Object.assign({}, _urgent);
            u[addr(data)] = true;
            _urgent = u;
            _clientsDirty = true;
        } else if (n === "activewindowv2") {
            const a = addr(data);
            if (_urgent[a]) {
                const u = Object.assign({}, _urgent);
                delete u[a];
                _urgent = u;
            }
            _clientsDirty = true;
        } else if (n === "windowtitle" || n === "windowtitlev2" || n === "activewindow") {
            _clientsDirty = true;
        } else if (n === "openlayer" || n === "closelayer") {
            return;
        } else if (n.indexOf("monitor") === 0 || n === "custom" || n === "configreloaded" || n.indexOf("workspace") >= 0 || n.indexOf("focusedmon") === 0 || n.indexOf("special") >= 0) {
            _monitorsDirty = _workspacesDirty = _clientsDirty = true;
        } else {
            _workspacesDirty = _clientsDirty = true;
        }
        if (!_batch.running)
            _batch.start();
    }

    Component.onCompleted: {
        _events.connected = true;
        refresh();
    }
}
