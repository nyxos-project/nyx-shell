pragma Singleton
import QtQuick
import Quickshell

// Views of Shell Core's desktop state, so surfaces don't each search the lists.
Singleton {
    readonly property var desktop: ShellCore.desktop
    readonly property var monitors: desktop.monitors
    readonly property var workspaces: desktop.workspaces
    readonly property var windows: desktop.windows
    readonly property string activeMonitorName: (desktop.active && desktop.active.monitor) || ""
    readonly property var activeMonitor: monitor(activeMonitorName)
    readonly property var activeWindow: window((desktop.active && desktop.active.window) || "")

    function monitor(name) {
        return monitors.find(m => m.name === name) || null;
    }
    function window(address) {
        return windows.find(w => w.address === address) || null;
    }
    function workspace(id) {
        return workspaces.find(w => w.id === id) || null;
    }
    function windowsOn(workspaceId) {
        return windows.filter(w => w.workspace === workspaceId);
    }
    function userWorkspaces() {
        return workspaces.filter(w => w.user);
    }
    // A fullscreen window on screen on this monitor.
    function fullscreenOn(name) {
        const m = monitor(name);
        if (!m)
            return false;
        return windows.some(w => !w.hidden && w.fullscreen === "fullscreen" && (w.workspace === m.active_workspace || w.workspace === m.special_workspace));
    }
}
