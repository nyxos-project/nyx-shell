import QtQuick

// One topic of a backend, subscribed only while `active` (a tile on screen, a detail
// view open), as GTK tiles subscribed only while mapped. `value` is its state.
//
//   BackendTopic { id: audio; client: cc; name: "audio"; active: tile.visible }
//   Text { text: audio.value ? audio.value.volume : "" }
QtObject {
    id: t

    required property var client
    required property string name
    property bool active: true
    readonly property var value: (client && client.state[name] !== undefined) ? client.state[name] : null

    property bool _on: false
    function _sync() {
        if (!client)
            return;
        if (active && !_on) {
            client.watch(name);
            _on = true;
        } else if (!active && _on) {
            client.unwatch(name);
            _on = false;
        }
    }
    onActiveChanged: _sync()
    Component.onCompleted: _sync()
    Component.onDestruction: if (_on)
        client.unwatch(name)
}
