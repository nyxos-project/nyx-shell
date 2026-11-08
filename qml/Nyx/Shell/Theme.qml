pragma Singleton
import QtQuick
import Quickshell

// Colours from Shell Core's theme events (the suite theme; Theme Engine 2.0 replaces
// the source in Handoff 02), with Nyx Dark as the fallback until they arrive.
Singleton {
    readonly property var palette: (ShellCore.theme && ShellCore.theme.palette) || ({})
    readonly property int generation: ShellCore.theme ? ShellCore.theme.generation : 0
    readonly property bool light: ShellCore.theme ? ShellCore.theme.light : false

    readonly property color bg: palette.bg || "#0e0f13"
    readonly property color surface: palette.surface || "#16171d"
    readonly property color text: palette.text || "#e6e7ee"
    readonly property color dimText: palette.dim_text || "#9a9cab"
    readonly property color accent: palette.accent || "#8b7cf6"
    readonly property color border: palette.border || "#2a2c36"
    readonly property color muted: palette.muted || "#20222b"
    readonly property color highlight: palette.highlight || "#2b2d3a"
    readonly property color danger: palette.danger || "#e06c75"
    readonly property color shadow: palette.shadow || "#000000"

    readonly property int radius: 14
    readonly property int radiusSmall: 8
    readonly property int spacing: 8
    readonly property int padding: 14
    readonly property int fontSize: 13
    readonly property int fontSizeSmall: 11
    readonly property int fontSizeTitle: 16
}
