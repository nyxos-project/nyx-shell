pragma Singleton
import QtQuick
import Quickshell

// One set of durations and curves for every shell surface.
Singleton {
    readonly property int fast: 120
    readonly property int normal: 200
    readonly property int slow: 320
    readonly property int easing: Easing.OutCubic
    readonly property int easingIn: Easing.InCubic
}
