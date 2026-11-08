import QtQuick
import Quickshell
import Nyx.Shell

// A passive overlay for the policy tests: never takes the keyboard, closes nothing,
// hides by itself. `nyx-shellctl invoke shell.osd-sample.show '{"level": 40}'`.
Scope {
    id: root

    property int level: 50

    ShellComponent {
        componentId: "nyx.osd-sample"
        actions: ["shell.osd-sample.show"]
        overlays: ["nyx.osd-sample"]
        onInvoked: (action, args) => {
            root.level = Math.max(0, Math.min(100, (args && args.level !== undefined) ? args.level : 50));
            ShellCore.showOverlay("nyx.osd-sample");
            hide.restart();
        }
    }

    Timer {
        id: hide
        interval: 1500
        onTriggered: ShellCore.hideOverlay("nyx.osd-sample")
    }

    OverlaySurface {
        overlayId: "nyx.osd-sample"
        category: "passive"
        keyboard: "none"
        fullscreenRule: "show"
        anchors.bottom: true
        margins.bottom: 80
        implicitWidth: 320
        implicitHeight: 64

        Rectangle {
            anchors.fill: parent
            radius: Theme.radius
            color: Theme.surface
            border.color: Theme.border
            Row {
                anchors.centerIn: parent
                spacing: 14
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.level + "%"
                    color: Theme.text
                    font.pixelSize: Theme.fontSizeTitle
                    width: 48
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 200
                    height: 8
                    radius: 4
                    color: Theme.muted
                    Rectangle {
                        width: parent.width * root.level / 100
                        height: parent.height
                        radius: 4
                        color: Theme.accent
                        Behavior on width {
                            NumberAnimation {
                                duration: Motion.fast
                            }
                        }
                    }
                }
            }
        }
    }
}
