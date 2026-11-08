import QtQuick
import Quickshell
import Nyx.Shell
import "examples/inspector" as Inspector
import "examples/osd" as OsdSample

// nyx-shell-qs: the one Quickshell runtime for ordinary shell surfaces. Components
// move in here in Handoff 03; until then it holds the examples (NYX_SHELL_EXAMPLES=1).
ShellRoot {
    LazyLoader {
        active: Quickshell.env("NYX_SHELL_EXAMPLES") === "1"
        Scope {
            Inspector.Inspector {}
            OsdSample.OsdSample {}
        }
    }
}
