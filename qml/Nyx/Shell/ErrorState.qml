import QtQuick

// What a surface shows when it can't get its data.
Item {
    id: root

    property string title: "Something went wrong"
    property string detail: ""
    property string actionText: "Try again"
    signal retry

    implicitWidth: Math.max(titleText.implicitWidth, detailText.implicitWidth) + 48
    implicitHeight: column.implicitHeight + 24

    Column {
        id: column
        anchors.centerIn: parent
        spacing: 8
        Text {
            id: titleText
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.title
            color: Theme.text
            font.pixelSize: Theme.fontSizeTitle
            font.weight: Font.DemiBold
        }
        Text {
            id: detailText
            anchors.horizontalCenter: parent.horizontalCenter
            visible: root.detail !== ""
            text: root.detail
            textFormat: Text.PlainText
            color: Theme.dimText
            font.pixelSize: Theme.fontSize
        }
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: retryText.implicitWidth + 24
            height: 30
            radius: Theme.radiusSmall
            color: retryArea.containsMouse ? Theme.highlight : Theme.muted
            Text {
                id: retryText
                anchors.centerIn: parent
                text: root.actionText
                color: Theme.text
                font.pixelSize: Theme.fontSize
            }
            MouseArea {
                id: retryArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: root.retry()
            }
        }
    }
}
