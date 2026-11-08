import QtQuick
import Quickshell

// One delegate per screen that should carry this component's persistent surface:
// headless outputs only with `monitors.include_virtual`, screens and components
// switched off in `[monitors.screens."STABLE ID"]` left out. NYX_SHELL_SCREEN keeps it
// to that one output. Delegates get `modelData` (the screen) as with Variants.
Variants {
    id: root

    property string componentId: ""

    readonly property var monitorsConfig: (ShellCore.config && ShellCore.config.config && ShellCore.config.config.monitors) || ({ include_virtual: false, screens: {} })
    readonly property string pinnedScreen: Quickshell.env("NYX_SHELL_SCREEN") || ""

    function allowed(screen) {
        if (pinnedScreen)
            return screen.name === pinnedScreen;
        const m = DesktopState.monitor(screen.name);
        if (!m || !m.enabled)
            return false;
        if (m.virtual && !monitorsConfig.include_virtual)
            return false;
        const s = (monitorsConfig.screens || {})[m.stable_id];
        if (s && s.enabled === false)
            return false;
        if (s && componentId && s.components && s.components[componentId] === false)
            return false;
        return true;
    }

    model: Quickshell.screens.filter(s => root.allowed(s))
}
