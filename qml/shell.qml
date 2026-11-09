import QtQuick
import Qt.labs.folderlistmodel
import Quickshell
import Nyx.Shell
import "examples/inspector" as Inspector
import "examples/osd" as OsdSample

// nyx-shell-qs: the one Quickshell runtime for ordinary shell surfaces.
//
// Components are folders with a Main.qml, found in the folders on
// NYX_SHELL_COMPONENT_PATH (colon-separated), else $XDG_DATA_HOME/nyx-shell/components
// and each $XDG_DATA_DIRS/nyx-shell/components. A component appears and goes as its
// folder (usually a link to the component's own QML) is added or removed, so
// `nyx-dock --ui qml|gtk` switches without restarting the runtime.
// NYX_SHELL_COMPONENTS=a,b loads only those (tests); NYX_SHELL_EXAMPLES=1 adds the
// inspector and the OSD sample.
ShellRoot {
    id: shell

    readonly property var searchPath: {
        const set = Quickshell.env("NYX_SHELL_COMPONENT_PATH");
        if (set)
            return set.split(":").filter(p => p !== "");
        const home = Quickshell.env("HOME");
        const data = Quickshell.env("XDG_DATA_HOME") || (home + "/.local/share");
        const dirs = (Quickshell.env("XDG_DATA_DIRS") || "/usr/local/share:/usr/share").split(":").filter(p => p !== "");
        return [data].concat(dirs).map(d => d + "/nyx-shell/components");
    }
    readonly property var only: (Quickshell.env("NYX_SHELL_COMPONENTS") || "").split(",").filter(s => s !== "")

    // Component name -> folder; the first folder on the path wins.
    property var found: ({})

    function rescan() {
        const out = {};
        for (let i = 0; i < folders.count; i++) {
            const model = folders.objectAt(i);
            if (!model)
                continue;
            // A folder that doesn't exist makes FolderListModel list the working
            // directory instead: take only what is inside the folder asked for.
            const inside = "file://" + model.modelData.replace(/\/$/, "") + "/";
            for (let j = 0; j < model.count; j++) {
                const name = model.get(j, "fileName");
                const url = model.get(j, "fileUrl").toString();
                if (url !== inside + name)
                    continue;
                if (out[name] === undefined && (shell.only.length === 0 || shell.only.indexOf(name) >= 0))
                    out[name] = url;
            }
        }
        if (JSON.stringify(out) !== JSON.stringify(shell.found))
            shell.found = out;
    }

    Instantiator {
        id: folders
        model: shell.searchPath
        delegate: FolderListModel {
            required property string modelData
            folder: "file://" + modelData
            showFiles: false
            showDirs: true
            showDotAndDotDot: false
            showHidden: false
            onCountChanged: Qt.callLater(shell.rescan)
            onStatusChanged: Qt.callLater(shell.rescan)
        }
        onObjectAdded: Qt.callLater(shell.rescan)
    }

    Variants {
        model: Object.keys(shell.found).sort()
        delegate: LazyLoader {
            required property string modelData
            active: true
            source: shell.found[modelData] !== undefined ? shell.found[modelData] + "/Main.qml" : ""
        }
    }

    LazyLoader {
        active: Quickshell.env("NYX_SHELL_EXAMPLES") === "1"
        Scope {
            Inspector.Inspector {}
            OsdSample.OsdSample {}
        }
    }
}
