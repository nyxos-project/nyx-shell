//! Giving focus back when a transient overlay closes.
//!
//! The window focused when the first focus-restoring overlay opened is remembered.
//! When the last one closes, focus goes back to it only if it still exists and is on
//! screen, the session isn't locked, nothing else was focused in the meantime (the
//! user's choice wins), and no restore happened within the debounce time. That last
//! rule means a restore can never start a loop.

use nyx_shell_proto::DesktopState;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct Focus {
    saved: Option<String>,
    /// Another window was focused while an overlay was open.
    moved: bool,
    open: bool,
    last_restore: Option<Instant>,
}

impl Focus {
    /// A focus-restoring overlay became visible.
    pub fn opened(&mut self, desktop: &DesktopState) {
        if !self.open {
            self.open = true;
            self.moved = false;
            self.saved = desktop.active.window.clone();
        }
    }

    /// The desktop changed while an overlay is open: note a window the user focused.
    pub fn desktop_changed(&mut self, desktop: &DesktopState) {
        if self.open
            && let Some(now) = &desktop.active.window
            && Some(now) != self.saved.as_ref()
        {
            self.moved = true;
        }
    }

    /// The last focus-restoring overlay closed. Returns the window to focus, if any.
    pub fn closed(&mut self, desktop: &DesktopState, locked: bool, enabled: bool, debounce: Duration, now: Instant) -> Option<String> {
        if !self.open {
            return None;
        }
        self.open = false;
        let saved = self.saved.take()?;
        if !enabled || locked || self.moved {
            return None;
        }
        if desktop.active.window.as_deref() == Some(saved.as_str()) {
            return None;
        }
        if desktop.active.window.is_some() {
            return None;
        }
        let window = desktop.window(&saved)?;
        let on_screen = desktop.workspace(window.workspace).is_some_and(|w| w.active) && !window.hidden;
        if !on_screen {
            return None;
        }
        if self.last_restore.is_some_and(|t| now.duration_since(t) < debounce) {
            return None;
        }
        self.last_restore = Some(now);
        Some(saved)
    }

    /// The session locked: forget everything.
    pub fn locked(&mut self) {
        self.saved = None;
        self.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_shell_proto::{Active, WindowState, WorkspaceState};

    fn desktop(active: Option<&str>) -> DesktopState {
        DesktopState {
            connected: true,
            workspaces: vec![
                WorkspaceState { id: 1, active: true, ..Default::default() },
                WorkspaceState { id: 2, active: false, ..Default::default() },
            ],
            windows: vec![
                WindowState { address: "a".into(), workspace: 1, ..Default::default() },
                WindowState { address: "b".into(), workspace: 1, ..Default::default() },
                WindowState { address: "z".into(), workspace: 2, ..Default::default() },
            ],
            active: Active { monitor: Some("m".into()), workspace: Some(1), window: active.map(str::to_string) },
            ..Default::default()
        }
    }

    const D: Duration = Duration::from_millis(500);

    #[test]
    fn restores_after_an_overlay_took_focus() {
        let mut f = Focus::default();
        let t = Instant::now();
        f.opened(&desktop(Some("a")));
        f.desktop_changed(&desktop(None));
        assert_eq!(f.closed(&desktop(None), false, true, D, t).as_deref(), Some("a"));
    }

    #[test]
    fn the_users_choice_wins() {
        let mut f = Focus::default();
        f.opened(&desktop(Some("a")));
        f.desktop_changed(&desktop(Some("b")));
        assert_eq!(f.closed(&desktop(Some("b")), false, true, D, Instant::now()), None);
    }

    #[test]
    fn nothing_when_locked_disabled_gone_or_hidden() {
        let t = Instant::now();
        let mut f = Focus::default();
        f.opened(&desktop(Some("a")));
        assert_eq!(f.closed(&desktop(None), true, true, D, t), None, "locked");
        f.opened(&desktop(Some("a")));
        assert_eq!(f.closed(&desktop(None), false, false, D, t), None, "disabled");
        f.opened(&desktop(Some("gone")));
        assert_eq!(f.closed(&desktop(None), false, true, D, t), None, "window closed");
        f.opened(&desktop(Some("z")));
        assert_eq!(f.closed(&desktop(None), false, true, D, t), None, "on a hidden workspace");
        f.opened(&desktop(Some("a")));
        assert_eq!(f.closed(&desktop(Some("a")), false, true, D, t), None, "already focused");
    }

    #[test]
    fn debounce_prevents_loops() {
        let mut f = Focus::default();
        let t = Instant::now();
        f.opened(&desktop(Some("a")));
        assert!(f.closed(&desktop(None), false, true, D, t).is_some());
        f.opened(&desktop(Some("a")));
        assert_eq!(f.closed(&desktop(None), false, true, D, t + Duration::from_millis(100)), None);
        f.opened(&desktop(Some("a")));
        assert!(f.closed(&desktop(None), false, true, D, t + Duration::from_millis(700)).is_some());
    }

    #[test]
    fn closing_without_opening_does_nothing() {
        let mut f = Focus::default();
        assert_eq!(f.closed(&desktop(None), false, true, D, Instant::now()), None);
    }
}
