import QtQuick
import Quickshell
import Nyx.Shell
import Nyx.Theme
import Nyx.Ui

// System OSDs (NyxOS): volume, microphone, screen brightness and keyboard light, shown
// briefly when they change. State comes from the Control Center backend's providers
// (event-driven for sound, read back at once after a change made through it), so no
// second audio or brightness service runs. One passive surface: it never takes the
// keyboard, shows over fullscreen windows, and repeated changes update it in place
// instead of stacking. Changes made in the open Control Center (its sliders already
// show them) don't pop it up.
//
// NYX_OSD_TEST=volume:40 (or mic:off, brightness:70, keyboard:2/3) shows a sample.
Scope {
    id: osd

    BackendClient {
        id: cc
        app: "nyx-control-center"
        topics: ["panel"]
    }
    BackendTopic {
        id: audio
        client: cc
        name: "audio"
    }
    BackendTopic {
        id: bright
        client: cc
        name: "brightness"
    }

    property string kind: ""
    property real level: 0
    property bool muted: false
    property string text: ""
    property var last: ({})
    readonly property bool panelOpen: !!(cc.state.panel && cc.state.panel.open)

    ShellComponent {
        componentId: "nyx.osd"
        overlays: ["nyx.osd"]
    }

    function show(k, level, muted, text) {
        kind = k;
        osd.level = level;
        osd.muted = muted;
        osd.text = text;
        ShellCore.showOverlay("nyx.osd");
        hide.restart();
    }

    // Something changed since the last state seen: show what. The first state after a
    // (re)connection is only remembered.
    function compare(key, value, onChange) {
        const before = last[key];
        const l = Object.assign({}, last);
        l[key] = value;
        last = l;
        if (before !== undefined && JSON.stringify(before) !== JSON.stringify(value) && !panelOpen)
            onChange();
    }

    Connections {
        target: audio
        function onValueChanged() {
            const a = audio.value;
            if (!a || !a.available)
                return;
            osd.compare("volume", [Math.round(a.volume * 100), a.muted], () => osd.show("volume", a.volume, a.muted, a.muted ? "Muted" : Math.round(a.volume * 100) + "%"));
            osd.compare("mic", a.mic_muted, () => osd.show("mic", a.mic_muted ? 0 : a.mic_volume, a.mic_muted, a.mic_muted ? "Microphone off" : "Microphone on"));
        }
    }
    Connections {
        target: bright
        function onValueChanged() {
            const b = bright.value;
            if (!b)
                return;
            if (b.screen)
                osd.compare("screen", Math.round(b.screen.value * 100), () => osd.show("brightness", b.screen.value, false, Math.round(b.screen.value * 100) + "%"));
            if (b.keyboard) {
                const step = Math.round(b.keyboard.value * b.keyboard.max);
                osd.compare("keyboard", step, () => osd.show("keyboard", b.keyboard.value, step === 0, step === 0 ? "Off" : step + " of " + b.keyboard.max));
            }
        }
    }
    Connections {
        target: cc
        function onReconnected() {
            osd.last = {};
        }
    }

    Timer {
        id: hide
        interval: 1500
        onTriggered: ShellCore.hideOverlay("nyx.osd")
    }

    // The sample waits for the overlay's registration.
    Connections {
        target: ShellCore
        function onReconnected() {
            Qt.callLater(osd.sample);
        }
    }
    function sample() {
        const t = Quickshell.env("NYX_OSD_TEST");
        if (!t || !ShellCore.overlay("nyx.osd"))
            return;
        const [k, v] = t.split(":");
        {
            if (k === "mic")
                show("mic", v === "off" ? 0 : 1, v === "off", v === "off" ? "Microphone off" : "Microphone on");
            else if (k === "keyboard") {
                const [s, m] = v.split("/").map(Number);
                show("keyboard", s / m, s === 0, s + " of " + m);
            } else
                show(k, Number(v) / 100, false, v + "%");
            hide.interval = 60000;
        }
    }

    OverlaySurface {
        id: surface
        overlayId: "nyx.osd"
        category: "passive"
        keyboard: "none"
        fullscreenRule: "show"
        placement: "active"
        anchors.bottom: true
        margins.bottom: 96
        implicitWidth: 300
        implicitHeight: 56

        Rectangle {
            anchors.fill: parent
            radius: Style.radiusCard
            color: Style.popover
            border.width: 1
            border.color: Style.alpha(Style.border, 0.55)
            opacity: surface.shown ? 1 : 0
            Behavior on opacity {
                NumberAnimation {
                    duration: Style.fast
                }
            }

            Icon {
                id: icon
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                size: 22
                recolor: true
                color: osd.muted ? Style.dimText : Style.text
                name: {
                    switch (osd.kind) {
                    case "volume":
                        return osd.muted ? "audio-volume-muted-symbolic" : osd.level < 0.34 ? "audio-volume-low-symbolic" : osd.level < 0.67 ? "audio-volume-medium-symbolic" : "audio-volume-high-symbolic";
                    case "mic":
                        return osd.muted ? "microphone-sensitivity-muted-symbolic" : "audio-input-microphone-symbolic";
                    case "brightness":
                        return "display-brightness-symbolic";
                    case "keyboard":
                        return "keyboard-brightness-symbolic";
                    }
                    return "";
                }
            }
            ProgressBar {
                anchors.left: icon.right
                anchors.leftMargin: 14
                anchors.right: value.left
                anchors.rightMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                height: 6
                visible: osd.kind !== "mic"
                value: Math.min(1, osd.level)
                fill: osd.muted ? Style.dimText : osd.level > 1 ? Style.warning : Style.accent
            }
            Label {
                id: value
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                width: osd.kind === "mic" ? parent.width - 70 : 52
                horizontalAlignment: osd.kind === "mic" ? Text.AlignLeft : Text.AlignRight
                text: osd.text
                font.features: { "tnum": 1 }
                font.weight: Font.Medium
            }
        }
    }
}
