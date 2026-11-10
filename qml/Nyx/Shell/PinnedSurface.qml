import QtQuick
import Quickshell

// A persistent overlay pinned through Shell Core (`PinOverlay`): shown until unpinned,
// in the box Shell Core lays out on its monitor (`pin.at`, `pin.size`, logical pixels),
// on every workspace of that monitor, hidden over fullscreen windows and while locked.
// It takes the keyboard only when clicked, so it never comes between the person and
// their windows or app switching. It pins itself again after every registration, so a
// restarted Shell Core gets it back.
OverlaySurface {
    id: pinned

    // The size to ask for, and the monitor (connector name) it was on last time.
    property var pinSize: [360, 280]
    property string pinMonitor: ""
    readonly property var box: overlayState && overlayState.pin ? overlayState.pin : null

    // Shell Core said no (no room on the monitor, locked): the message.
    signal refused(string message)

    function pin() {
        // A surface held to one output (NYX_SHELL_SCREEN) is laid out for that one.
        ShellCore.pinOverlay(overlayId, pinSize, pinnedScreen || pinMonitor, (ok, err) => {
            if (err)
                pinned.refused(err.msg);
        });
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
    margins.top: box ? box.at[1] : 0
    margins.left: box ? box.at[0] : 0
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
