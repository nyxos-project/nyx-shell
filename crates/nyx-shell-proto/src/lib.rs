//! Nyx Shell Core's IPC contract, version 1 (`docs/PROTOCOL.md`).
//!
//! One JSON object per line on `$XDG_RUNTIME_DIR/nyx-shell/core.sock`: requests
//! (`{v, id, op, body}`), replies (`{id, ok}` or `{id, err}`) and events
//! (`{ev, seq, ts, body}`). Clients may depend on this crate alone: it has no runtime.

pub mod action;
pub mod component;
pub mod config;
pub mod desktop;
pub mod frame;
pub mod overlay;
pub mod session;
pub mod shortcut;
pub mod theme;

pub use action::{ActionInfo, Routed};
pub use component::{ComponentKind, ComponentManifest, ComponentState, ComponentStatus, HealthReport, RestartPolicy};
pub use config::{ConfigState, ShellConfig};
pub use desktop::{
    Active, Collection, DesktopDelta, DesktopState, Fullscreen, MonitorState, PhysicalState, RegionState, WindowState, WorkspaceState,
    StaleDelta,
};
pub use frame::{
    Capability, ErrorBody, ErrorCode, Event, EventFrame, Hello, HelloReply, Op, RawRequest, Reply, Topic, MAX_LINE,
    MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, WindowAction, WindowActionKind, WorkspaceAction,
};
pub use overlay::{Category, FullscreenRule, Keyboard, OverlayPolicy, OverlayState, Placement};
pub use session::{CompositorStatus, ConfigSummary, IpcStatus, OverlaySummary, SessionState, SessionStatus, ThemeSummary};
pub use theme::ThemeState;

/// The socket path: `NYX_SHELL_SOCKET`, else `$XDG_RUNTIME_DIR/nyx-shell/core.sock`.
pub fn socket_path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("NYX_SHELL_SOCKET").filter(|p| !p.is_empty()) {
        return Some(p.into());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty())?;
    Some(std::path::PathBuf::from(runtime).join("nyx-shell").join("core.sock"))
}

/// Ids of overlays, components and actions: `[a-z0-9._:-]`, 1 to 64 characters.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b))
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids() {
        assert!(super::valid_id("nyx.inspector"));
        assert!(super::valid_id("shell.overlay.toggle:nyx.osd-sample"));
        assert!(!super::valid_id(""));
        assert!(!super::valid_id("Upper"));
        assert!(!super::valid_id("a b"));
        assert!(!super::valid_id(&"a".repeat(65)));
    }
}
