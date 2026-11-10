import QtQuick
import Quickshell
import Quickshell.Wayland

// A layer-shell surface whose visibility Shell Core decides. It registers its policy
// on every (re)connection, shows when its overlay state says `visible`, on the monitor
// the state names, and asks Shell Core to hide on Escape (transient overlays).
//
// Tests: NYX_SHELL_SCREEN pins every surface to that output and never falls back to
// another one; NYX_SHELL_NO_KEYBOARD=1 keeps every surface off the keyboard.
PanelWindow {
    id: surface

    required property string overlayId
    property string category: "transient"
    property string group: ""
    property string keyboard: "none"
    property string fullscreenRule: "hide"
    property string placement: "active"
    property var restoreFocus: undefined
    property string layer: category === "persistent" ? "top" : "overlay"
    property bool dismissOnEscape: category === "transient"

    default property alias content: body.data

    // After each successful registration (Shell Core started again included).
    signal registered

    readonly property var overlayState: ShellCore.overlay(overlayId)
    readonly property bool shown: overlayState !== null && overlayState.visible
    readonly property string pinnedScreen: Quickshell.env("NYX_SHELL_SCREEN") || ""
    readonly property bool keyboardOff: Quickshell.env("NYX_SHELL_NO_KEYBOARD") === "1"
    readonly property string wantedScreen: pinnedScreen || (overlayState && overlayState.monitor) || DesktopState.activeMonitorName
    readonly property var targetScreen: Quickshell.screens.find(s => s.name === wantedScreen) || null

    function policy() {
        const p = {
            id: overlayId,
            category: category,
            keyboard: keyboard,
            fullscreen: fullscreenRule,
            placement: placement
        };
        if (group)
            p.group = group;
        if (restoreFocus !== undefined)
            p.restore_focus = restoreFocus;
        return p;
    }

    function register() {
        ShellCore.request("RegisterOverlay", { policy: policy() }, (ok, err) => {
            if (err)
                console.warn("nyx-shell:", overlayId, "not registered:", err.code, err.msg);
            else {
                ShellCore._merge(ok, null);
                surface.registered();
            }
        });
    }

    Connections {
        target: ShellCore
        function onReconnected() {
            surface.register();
        }
    }
    Component.onCompleted: if (ShellCore.ready)
        register()

    visible: shown && targetScreen !== null
    screen: targetScreen
    color: "transparent"
    exclusionMode: ExclusionMode.Ignore
    WlrLayershell.namespace: "nyx-shell-" + overlayId
    WlrLayershell.layer: ({ background: WlrLayer.Background, bottom: WlrLayer.Bottom, top: WlrLayer.Top, overlay: WlrLayer.Overlay })[layer]
    WlrLayershell.keyboardFocus: (keyboardOff || !shown) ? WlrKeyboardFocus.None : ({ none: WlrKeyboardFocus.None, "on-demand": WlrKeyboardFocus.OnDemand, exclusive: WlrKeyboardFocus.Exclusive })[keyboard]

    FocusScope {
        id: body
        anchors.fill: parent
        focus: true
        Keys.onEscapePressed: event => {
            if (surface.dismissOnEscape) {
                ShellCore.hideOverlay(surface.overlayId);
                event.accepted = true;
            }
        }
    }
}
