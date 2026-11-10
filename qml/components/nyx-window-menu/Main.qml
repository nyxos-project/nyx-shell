import QtQuick
import Quickshell
import Quickshell.Hyprland
import Nyx.Shell
import Nyx.Ui

// The window menu (adaptive window controls): what a window can do now, as Shell
// Core's action table says (`actions` in its desktop state), named for its mode as a
// Nyx app's own window controls name them. NyxOS's decorations (nyx-decor) open it from
// a headerbar's right-click and from the chromeless control on a floating terminal,
// through Hyprland's custom event `nyx-window-menu,0xADDRESS,X,Y` (X, Y: where, in the
// layout's logical coordinates).
//
// One transient overlay over the monitor the point is on: a click outside the menu,
// Escape, the window going away or another window taking focus close it.
//
// NYX_WINDOW_MENU_TEST=ADDRESS opens it for that window once Shell Core is up
// (screenshots).
Scope {
    id: root

    readonly property string overlayId: "nyx.window-menu"
    // As Shell Core names windows: lowercase hex without 0x.
    property string address: ""
    property point at: Qt.point(0, 0)
    readonly property var win: ShellCore.desktop.windows.find(w => w.address === address) || null
    readonly property string activeWindow: (ShellCore.desktop.active && ShellCore.desktop.active.window) || ""
    readonly property bool shown: surface.shown

    ShellComponent {
        componentId: "nyx.window-menu"
        overlays: [root.overlayId]
    }

    function monitorAt(x, y) {
        return ShellCore.desktop.monitors.find(m => m.enabled && x >= m.x && x < m.x + m.logical_width && y >= m.y && y < m.y + m.logical_height) || null;
    }

    // `monitorName` when the caller knows it (the test); else the monitor under the point.
    function open(addr, x, y, monitorName) {
        const m = monitorName ? ShellCore.desktop.monitors.find(m => m.name === monitorName) : monitorAt(x, y);
        const a = addr.replace(/^0x/, "").toLowerCase();
        if (!m || !ShellCore.desktop.windows.some(w => w.address === a))
            return;
        address = a;
        at = Qt.point(x - m.x, y - m.y);
        ShellCore.showOverlay(overlayId, m.name);
    }

    function close() {
        ShellCore.hideOverlay(overlayId);
    }

    Connections {
        target: Hyprland
        function onRawEvent(event) {
            if (event.name !== "custom" || !event.data.startsWith("nyx-window-menu,"))
                return;
            const parts = event.data.split(",");
            if (parts.length === 4)
                root.open(parts[1], Number(parts[2]), Number(parts[3]));
        }
    }

    readonly property bool testing: Quickshell.env("NYX_WINDOW_MENU_TEST") !== ""

    onWinChanged: if (!win && shown)
        close()
    // A test copy's window never has focus, so the test keeps it open.
    onActiveWindowChanged: if (shown && !testing && activeWindow !== "" && activeWindow !== address)
        close()

    // The window's actions, as menu entries.
    function entries() {
        const w = win;
        if (!w)
            return [];
        const has = a => (w.actions || []).indexOf(a) >= 0;
        const maximized = w.fullscreen === "maximized";
        const fullscreen = w.fullscreen === "fullscreen";
        const out = [];
        if (has("minimize"))
            out.push({ id: "minimize", label: "Minimize", icon: "window-minimize-symbolic" });
        if (has("toggle_maximized"))
            out.push({
                id: "toggle_maximized",
                label: w.floating ? (maximized ? "Restore size" : "Maximize") : (maximized ? "Restore to the layout" : "Expand"),
                icon: maximized ? "window-restore-symbolic" : "window-maximize-symbolic"
            });
        if (has("set_floating"))
            out.push({ id: "set_floating", label: "Float", icon: "window-new-symbolic" });
        if (has("set_tiled"))
            out.push({ id: "set_tiled", label: "Tile", icon: "view-grid-symbolic" });
        if (has("toggle_fullscreen"))
            out.push({ id: "toggle_fullscreen", label: fullscreen ? "Leave fullscreen" : "Fullscreen", icon: fullscreen ? "view-restore-symbolic" : "view-fullscreen-symbolic" });
        if (has("toggle_pinned"))
            out.push({ id: "toggle_pinned", label: "Keep on top, on every workspace", checked: !!w.pinned });
        const moves = [];
        if (has("move_to_workspace"))
            moves.push({ id: "workspaces", label: "Move to workspace…", icon: "view-app-grid-symbolic" });
        if (has("move_to_monitor") && otherMonitors().length > 0)
            moves.push({ id: "monitors", label: "Move to monitor…", icon: "video-display-symbolic" });
        if (moves.length > 0)
            out.push({ separator: true }, ...moves);
        out.push({ separator: true }, { id: "close", label: "Close", icon: "window-close-symbolic", danger: true });
        return out;
    }

    function otherMonitors() {
        return ShellCore.desktop.monitors.filter(m => m.enabled && win && m.name !== win.monitor);
    }

    // Workspaces 1 to 10 of the window's monitor (a virtual monitor's own, past its base).
    function workspaceEntries() {
        const w = win;
        if (!w)
            return [];
        const m = ShellCore.desktop.monitors.find(m => m.name === w.monitor);
        const base = m && m.region ? m.region.workspace_base : 0;
        const out = [{ heading: "Move to workspace" }];
        for (let n = 1; n <= 10; n++) {
            if (base + n !== w.workspace)
                out.push({ id: "ws:" + (base + n), label: "Workspace " + n });
        }
        return out;
    }

    function monitorEntries() {
        return [{ heading: "Move to monitor" }].concat(otherMonitors().map(m => ({ id: "mon:" + m.name, label: m.description ? m.description + " (" + m.name + ")" : m.name, icon: "video-display-symbolic" })));
    }

    function pick(id) {
        if (id === "workspaces" || id === "monitors") {
            sub.entries = id === "workspaces" ? workspaceEntries() : monitorEntries();
            sub.openAt(0, 0);
            return;
        }
        if (id.startsWith("ws:"))
            ShellCore.windowAction(address, "move_to_workspace", { workspace: id.slice(3) });
        else if (id.startsWith("mon:"))
            ShellCore.windowAction(address, "move_to_monitor", { monitor: id.slice(4) });
        else
            ShellCore.windowAction(address, id);
        close();
    }

    // A menu closing (picked, Escape, a click outside) ends the overlay, unless the
    // other one just opened.
    function dismissed() {
        Qt.callLater(() => {
            if (!main.visible && !sub.visible)
                root.close();
        });
    }

    Connections {
        target: ShellCore
        function onReconnected() {
            const t = Quickshell.env("NYX_WINDOW_MENU_TEST");
            if (!t)
                return;
            Qt.callLater(() => {
                const w = ShellCore.desktop.windows.find(w => w.address === t.replace(/^0x/, "").toLowerCase());
                const m = w && ShellCore.desktop.monitors.find(m => m.name === w.monitor);
                if (m)
                    root.open(t, m.x + Math.round(m.logical_width / 3), m.y + Math.round(m.logical_height / 4), m.name);
            });
        }
    }

    OverlaySurface {
        id: surface
        overlayId: root.overlayId
        category: "transient"
        keyboard: "exclusive"
        fullscreenRule: "hide"
        placement: "active"
        anchors.top: true
        anchors.bottom: true
        anchors.left: true
        anchors.right: true

        onShownChanged: {
            if (shown)
                Qt.callLater(() => {
                    main.openAt(0, 0);
                    keys.forceActiveFocus();
                });
            else {
                main.close();
                sub.close();
            }
        }

        // Anywhere but the menu closes it.
        MouseArea {
            anchors.fill: parent
            onClicked: root.close()
        }

        // The menus can't grab: a popup's grab needs an input event on this surface,
        // and the click that asked for the menu went to the window. The overlay has
        // the keyboard, so it moves through them.
        Item {
            id: keys
            anchors.fill: parent
            focus: true
            Keys.onPressed: event => {
                const m = sub.visible ? sub : main;
                if (event.key === Qt.Key_Up)
                    m.move(-1);
                else if (event.key === Qt.Key_Down)
                    m.move(1);
                else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                    if (m.current >= 0)
                        m.pick(m.current);
                } else if (event.key === Qt.Key_Escape)
                    root.close();
                else
                    return;
                event.accepted = true;
            }
        }

        Item {
            id: point
            x: root.at.x
            y: root.at.y
            width: 1
            height: 1
        }

        Menu {
            id: main
            anchorItem: point
            grabFocus: false
            minWidth: 240
            entries: root.entries()
            onTriggered: id => root.pick(id)
            onDismissed: root.dismissed()
        }

        Menu {
            id: sub
            anchorItem: point
            grabFocus: false
            minWidth: 240
            onTriggered: id => root.pick(id)
            onDismissed: root.dismissed()
        }
    }
}
