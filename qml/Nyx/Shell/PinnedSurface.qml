import QtQuick
import Quickshell

// A persistent overlay pinned through Shell Core (`PinOverlay`): shown until unpinned,
// in the box Shell Core lays out on its monitor (`pin.at`, `pin.size`, logical pixels),
// on every workspace of that monitor, hidden over fullscreen windows and while locked.
// It takes the keyboard only when clicked, so it never comes between the person and
// their windows or app switching. It pins itself again after every registration, so a
// restarted Shell Core gets it back, where it was put by hand too (`pinAt`).
//
// Moving it: `dragBy(dx, dy)` while dragging (it follows at once), `drop()` asks Shell
// Core to keep it there (out of the stack, inside the usable area), `restack()` puts it
// back in the stack; `placed(at)` says where it ended up (null: in the stack).
OverlaySurface {
    id: pinned

    // The size to ask for, and the monitor (connector name) it was on last time.
    property var pinSize: [360, 280]
    property string pinMonitor: ""
    // Where it was put by hand last time, [x, y]; null in the stack.
    property var pinAt: null
    readonly property var box: overlayState && overlayState.pin ? overlayState.pin : null
    readonly property bool manual: box !== null && box.manual === true
    // While dragged: where it is now.
    property var dragAt: null

    // Shell Core said no (no room on the monitor, locked): the message.
    signal refused(string message)
    signal placed(var at)

    function pin() {
        // A surface held to one output (NYX_SHELL_SCREEN) is laid out for that one.
        ShellCore.pinOverlay(overlayId, pinSize, pinnedScreen || pinMonitor, (ok, err) => {
            if (err)
                pinned.refused(err.msg);
            else if (pinAt && !(ok.overlay.pin && ok.overlay.pin.manual))
                move(pinAt);
        });
    }
    function move(at) {
        ShellCore.request("MovePinnedOverlay", at ? { id: overlayId, at: [Math.round(at[0]), Math.round(at[1])] } : { id: overlayId }, (ok, err) => {
            pinned.dragAt = null;
            ShellCore._merge(ok, err);
            if (ok && ok.overlay.pin)
                pinned.placed(ok.overlay.pin.manual ? ok.overlay.pin.at : null);
        });
    }
    function dragBy(dx, dy) {
        const from = dragAt || (box ? box.at : [0, 0]);
        dragAt = [from[0] + dx, from[1] + dy];
    }
    function drop() {
        if (dragAt)
            move(dragAt);
    }
    function restack() {
        move(null);
    }
    function unpin() {
        ShellCore.unpinOverlay(overlayId);
    }

    category: "persistent"
    keyboard: "on-demand"
    fullscreenRule: "hide"
    layer: "top"
    dismissOnEscape: false
    anchors.top: true
    anchors.left: true
    margins.top: dragAt ? dragAt[1] : box ? box.at[1] : 0
    margins.left: dragAt ? dragAt[0] : box ? box.at[0] : 0
    implicitWidth: box ? box.size[0] : pinSize[0]
    implicitHeight: box ? box.size[1] : pinSize[1]

    // The id it registered under: what it was made from may be gone when it goes.
    property string _registered: ""
    onRegistered: {
        _registered = overlayId;
        pin();
    }
    // Gone (its panel unpinned, its component unloaded): the others move up.
    Component.onDestruction: if (_registered !== "")
        ShellCore.request("UnregisterOverlay", { id: _registered }, null)
}
