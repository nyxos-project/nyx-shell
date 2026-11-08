import QtQuick

// Keyboard conventions for shell surfaces: Escape dismisses, Tab and Shift+Tab move
// between focusable children in order, arrow keys are left to the children.
FocusScope {
    id: scope

    signal dismissed

    activeFocusOnTab: false
    Keys.onEscapePressed: event => {
        scope.dismissed();
        event.accepted = true;
    }
    Keys.onTabPressed: event => {
        const next = scope.nextItemInFocusChain(true);
        if (next)
            next.forceActiveFocus(Qt.TabFocusReason);
        event.accepted = true;
    }
    Keys.onBacktabPressed: event => {
        const prev = scope.nextItemInFocusChain(false);
        if (prev)
            prev.forceActiveFocus(Qt.BacktabFocusReason);
        event.accepted = true;
    }
}
