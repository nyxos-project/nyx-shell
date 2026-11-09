import QtQuick
import Quickshell
import Quickshell.Io

// The connection to a component's Rust backend (nyx-core's nyx-ui-link): one socket,
// the listed topics subscribed, each kept current from patches, reconnecting with
// backoff. Bind to `state.TOPIC`; send requests with `request()`; other events
// arrive as `event(name, body)`.
//
//   BackendClient { id: dock; app: "nyx-dock"; topics: ["dock"] }
//   Repeater { model: dock.state.dock ? dock.state.dock.items : [] }
//
// The socket is $NYX_DOCK_UI_SOCKET for app "nyx-dock", else
// $XDG_RUNTIME_DIR/nyx-dock/ui.sock.
QtObject {
    id: root

    required property string app
    property var topics: []
    property string socketPath: {
        const v = Quickshell.env(app.toUpperCase().replace(/-/g, "_") + "_UI_SOCKET");
        if (v)
            return v;
        const r = Quickshell.env("XDG_RUNTIME_DIR");
        return r ? r + "/" + app + "/ui.sock" : "";
    }

    // Topic name -> state. Replaced as a whole on every change, so bindings update.
    property var state: ({})
    // True once connected and every topic's first state has arrived.
    property bool ready: false
    readonly property bool connected: _sock !== null && _sock.connected
    property string lastError: ""

    signal event(string name, var body)
    // After every (re)connection, once the topics are in.
    signal reconnected

    // Topics subscribed on demand (BackendTopic), counted.
    property var _watch: ({})
    property var _sock: null
    property int _next: 0
    property var _pending: ({})
    property int _seq: 0
    property var _revs: ({})
    property int _backoff: 250

    function request(op, body, callback) {
        const s = root._sock;
        if (!s || !s.connected) {
            if (callback)
                callback(null, { code: "unavailable", msg: root.app + " isn't running" });
            return;
        }
        root._next += 1;
        if (callback)
            root._pending[root._next] = callback;
        const req = { v: 1, id: root._next, op: op };
        if (body !== undefined && body !== null)
            req.body = body;
        s.write(JSON.stringify(req) + "\n");
        s.flush();
    }

    // Subscribe to a topic while something shows it; every watch needs an unwatch.
    function watch(topic) {
        const n = (root._watch[topic] || 0) + 1;
        root._watch[topic] = n;
        if (n === 1 && root.topics.indexOf(topic) < 0 && root.connected)
            _subscribe(topic);
    }
    function unwatch(topic) {
        const n = (root._watch[topic] || 0) - 1;
        if (n > 0) {
            root._watch[topic] = n;
            return;
        }
        delete root._watch[topic];
        if (root.topics.indexOf(topic) >= 0)
            return;
        // Later: an unwatch comes from a binding update (a tile going away), and
        // changing `state` inside it would loop back into that update.
        Qt.callLater(() => {
            if (root._watch[topic])
                return;
            if (root.connected)
                request("Unsubscribe", { topic: topic }, null);
            const r = Object.assign({}, root._revs);
            delete r[topic];
            root._revs = r;
            const st = Object.assign({}, root.state);
            delete st[topic];
            root.state = st;
        });
    }

    function _set(topic, value, rev) {
        const r = Object.assign({}, root._revs);
        r[topic] = rev;
        root._revs = r;
        const st = Object.assign({}, root.state);
        st[topic] = value;
        root.state = st;
    }

    function _merge(target, patch) {
        if (patch === null || typeof patch !== "object" || Array.isArray(patch))
            return patch;
        const out = (target !== null && typeof target === "object" && !Array.isArray(target)) ? Object.assign({}, target) : {};
        for (const k in patch) {
            if (patch[k] === null)
                delete out[k];
            else
                out[k] = _merge(out[k], patch[k]);
        }
        return out;
    }

    function _subscribe(topic, done) {
        request("Subscribe", { topic: topic }, ok => {
            if (ok)
                _set(topic, ok.state, ok.rev);
            if (done)
                done();
        });
    }

    function _subscribeAll() {
        for (const t in root._watch) {
            if (root.topics.indexOf(t) < 0)
                _subscribe(t);
        }
        let waiting = root.topics.length;
        if (waiting === 0) {
            root.ready = true;
            root.reconnected();
            return;
        }
        for (const t of root.topics)
            _subscribe(t, () => {
                waiting -= 1;
                if (waiting === 0) {
                    root.ready = true;
                    root.reconnected();
                }
            });
    }

    function _event(m) {
        if (root._seq !== 0 && m.seq !== root._seq + 1) {
            root._seq = m.seq;
            for (const t of root.topics)
                _subscribe(t);
            return;
        }
        root._seq = m.seq;
        const b = m.body;
        switch (m.ev) {
        case "Patch":
            if (root._revs[b.topic] === b.base)
                _set(b.topic, _merge(root.state[b.topic], b.patch), b.rev);
            else if (root._revs[b.topic] === undefined || root._revs[b.topic] < b.rev)
                _subscribe(b.topic);
            break;
        case "State":
            _set(b.topic, b.state, b.rev);
            break;
        case "Resync":
            for (const t of b.topics)
                _subscribe(t);
            break;
        default:
            root.event(m.ev, b);
        }
    }

    function _line(data) {
        let m;
        try {
            m = JSON.parse(data);
        } catch (e) {
            root.lastError = "unreadable line from " + root.app;
            return;
        }
        if (m.ev !== undefined) {
            _event(m);
            return;
        }
        const cb = root._pending[m.id];
        delete root._pending[m.id];
        if (m.err) {
            root.lastError = m.err.code + ": " + m.err.msg;
            if (!cb)
                console.warn(root.app + ":", root.lastError);
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
        root._revs = {};
    }

    function _connect() {
        if (root._sock) {
            const old = root._sock;
            root._sock = null;
            old.destroy();
        }
        if (root.socketPath === "")
            return;
        const s = _socketComponent.createObject(root);
        root._sock = s;
        s.connected = true;
    }

    property Component _socketComponent: Component {
        Socket {
            id: socket
            path: root.socketPath
            connected: false
            parser: SplitParser {
                splitMarker: "\n"
                onRead: data => {
                    if (socket === root._sock)
                        root._line(data);
                }
            }
            onConnectionStateChanged: {
                if (socket !== root._sock)
                    return;
                if (connected) {
                    root._backoff = 250;
                    root.lastError = "";
                    root._subscribeAll();
                } else {
                    root._reset();
                    root._retry.restart();
                }
            }
            onError: error => {
                if (socket !== root._sock)
                    return;
                root.lastError = "can't reach " + root.app + " at " + root.socketPath;
                root._retry.restart();
            }
        }
    }

    property Timer _retry: Timer {
        interval: root._backoff
        onTriggered: {
            root._backoff = Math.min(root._backoff * 2, 5000);
            root._connect();
        }
    }

    Component.onCompleted: _connect()
}
