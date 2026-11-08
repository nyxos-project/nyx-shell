import QtQuick
import QtQuick.Layouts
import Quickshell
import Nyx.Shell

// A representative transient overlay: everything Shell Core knows, from the shared
// state, with no compositor watcher or overlay logic of its own.
Scope {
    ShellComponent {
        componentId: "nyx.inspector"
        actions: ["shell.inspector.toggle"]
        overlays: ["nyx.inspector"]
        onInvoked: ShellCore.toggleOverlay("nyx.inspector")
    }

    OverlaySurface {
        id: panel
        overlayId: "nyx.inspector"
        category: "transient"
        keyboard: "exclusive"
        fullscreenRule: "show"
        anchors {
            top: true
            right: true
        }
        margins {
            top: 10
            right: 10
        }
        implicitWidth: 620
        implicitHeight: 700

        Rectangle {
            anchors.fill: parent
            radius: Theme.radius
            color: Theme.surface
            border.color: Theme.border
            border.width: 1
            opacity: panel.shown ? 1 : 0
            Behavior on opacity {
                NumberAnimation {
                    duration: Motion.normal
                    easing.type: Motion.easing
                }
            }

            LoadingState {
                anchors.centerIn: parent
                visible: !ShellCore.ready
            }

            // Built only while shown, so a hidden inspector costs nothing per update.
            Loader {
                anchors.fill: parent
                anchors.margins: Theme.padding
                active: panel.shown && ShellCore.ready
                sourceComponent: details
            }
        }
    }

    Component {
        id: details

        Flickable {
            contentHeight: column.implicitHeight
            clip: true

            ColumnLayout {
                id: column
                width: parent.width
                spacing: Theme.spacing

                Text {
                    text: "Shell Core"
                    color: Theme.text
                    font.pixelSize: Theme.fontSizeTitle
                    font.weight: Font.DemiBold
                }
                Row {
                    spacing: 6
                    Repeater {
                        model: {
                            const s = ShellCore.session;
                            if (!s)
                                return [];
                            const out = [s.state, "generation " + ShellCore.desktop.generation, "theme " + Theme.generation];
                            if (s.read_only)
                                out.push("read-only");
                            if (s.locked)
                                out.push("locked");
                            return out;
                        }
                        Chip {
                            required property string modelData
                            label: modelData
                        }
                    }
                }

                Heading {
                    text: "Monitors"
                }
                Repeater {
                    model: DesktopState.monitors
                    Line {
                        required property var modelData
                        strong: modelData.focused
                        text: modelData.name + "  " + modelData.logical_width + "×" + modelData.logical_height + " @" + modelData.scale + (modelData.refresh_rate ? "  " + Math.round(modelData.refresh_rate) + " Hz" : "") + "  workspace " + modelData.active_workspace + (modelData.virtual ? "  (headless)" : "")
                    }
                }

                Heading {
                    text: "Workspaces"
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 6
                    Repeater {
                        model: DesktopState.workspaces.filter(w => !w.special)
                        Chip {
                            required property var modelData
                            label: modelData.name + " · " + modelData.windows.length
                            active: modelData.active
                        }
                    }
                }

                Heading {
                    text: "Windows"
                }
                Repeater {
                    model: DesktopState.windows.slice(0, 8)
                    Line {
                        required property var modelData
                        strong: modelData.focused
                        text: modelData.app_id + " — " + modelData.title
                    }
                }

                Heading {
                    text: "Overlays"
                }
                Repeater {
                    model: Object.values(ShellCore.overlays)
                    Line {
                        required property var modelData
                        strong: modelData.visible
                        text: modelData.id + "  " + (modelData.visible ? "visible" : "hidden") + "  " + modelData.category + "  " + modelData.owner
                    }
                }

                Heading {
                    text: "Components"
                }
                Repeater {
                    model: (ShellCore.session && ShellCore.session.components) || []
                    Line {
                        required property var modelData
                        strong: modelData.effective === "ready"
                        text: modelData.id + "  " + modelData.effective + (modelData.detail ? "  (" + modelData.detail + ")" : "")
                    }
                }
            }
        }
    }

    component Heading: Text {
        Layout.topMargin: 6
        color: Theme.dimText
        font.pixelSize: Theme.fontSizeSmall
        font.weight: Font.DemiBold
        font.capitalization: Font.AllUppercase
    }

    component Line: Text {
        property bool strong: false
        Layout.fillWidth: true
        textFormat: Text.PlainText
        elide: Text.ElideRight
        color: strong ? Theme.text : Theme.dimText
        font.pixelSize: Theme.fontSize
    }

    component Chip: Rectangle {
        property string label
        property bool active: false
        implicitWidth: chipText.implicitWidth + 16
        implicitHeight: 24
        radius: 12
        color: active ? Theme.accent : Theme.muted
        Text {
            id: chipText
            anchors.centerIn: parent
            text: parent.label
            textFormat: Text.PlainText
            color: parent.active ? Theme.bg : Theme.text
            font.pixelSize: Theme.fontSizeSmall
        }
    }
}
