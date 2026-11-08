pragma ComponentBehavior: Bound
import QtQuick

// What a surface shows before its data arrives.
Item {
    id: root

    property string text: "Connecting to Nyx Shell Core…"
    implicitWidth: label.implicitWidth + 48
    implicitHeight: 72

    Row {
        id: dots
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: 12
        spacing: 6
        Repeater {
            model: 3
            Rectangle {
                id: dot
                required property int index
                width: 7
                height: 7
                radius: 3.5
                color: Theme.accent
                opacity: 0.3
                SequentialAnimation on opacity {
                    running: root.visible
                    loops: Animation.Infinite
                    PauseAnimation {
                        duration: dot.index * 150
                    }
                    NumberAnimation {
                        to: 1
                        duration: Motion.slow
                    }
                    NumberAnimation {
                        to: 0.3
                        duration: Motion.slow
                    }
                    PauseAnimation {
                        duration: (2 - dot.index) * 150
                    }
                }
            }
        }
    }
    Text {
        id: label
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: dots.bottom
        anchors.topMargin: 12
        text: root.text
        color: Theme.dimText
        font.pixelSize: Theme.fontSize
    }
}
