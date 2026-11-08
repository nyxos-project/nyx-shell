import QtQuick

// A component's registration: its manifest, sent on every (re)connection, then ready.
// Actions it lists arrive as `invoked`.
QtObject {
    id: component

    required property string componentId
    property string kind: "surface"
    property string process: "nyx-shell-qs"
    property var requires: []
    property var optional: []
    property var actions: []
    property var overlays: []
    // Report ready right after registering; set false and call reportReady() yourself
    // when there's loading to do first.
    property bool autoReady: true
    property string state: "unregistered"

    signal invoked(string action, var args)

    function register() {
        const manifest = { id: componentId, kind: kind, process: process, requires: requires, optional: optional, actions: actions, overlays: overlays };
        ShellCore.request("RegisterComponent", { manifest: manifest }, (ok, err) => {
            if (err) {
                component.state = "failed";
                console.warn("nyx-shell:", componentId, "not registered:", err.code, err.msg);
                return;
            }
            component.state = ok.component.state;
            if (autoReady)
                reportReady();
        });
    }
    function reportReady() {
        ShellCore.request("ReportReady", { id: componentId }, ok => { if (ok) component.state = ok.component.state; });
    }
    function reportHealth(health, detail) {
        ShellCore.request("ReportHealth", { id: componentId, state: health, detail: detail || null }, ok => { if (ok) component.state = ok.component.state; });
    }

    property Connections _core: Connections {
        target: ShellCore
        function onReconnected() {
            component.register();
        }
        function onActionInvoked(action, args, invocation) {
            if (component.actions.indexOf(action) >= 0)
                component.invoked(action, args);
        }
    }
    Component.onCompleted: if (ShellCore.ready)
        register()
}
