//! Hyprland: the watcher that feeds the state, dispatching, and turning client requests
//! into dispatchers only after checking them against the state.

use crate::window;
use nyx_desktop::actions::{self, Dispatch};
use nyx_desktop::hypr::Instance;
use nyx_desktop::Desktop;
use nyx_shell_proto::{DesktopState, ErrorBody, ErrorCode, WindowAction, WindowActionKind, WorkspaceAction};
use serde::Deserialize;
use std::sync::Arc;

/// What Shell Core asks of the compositor. A trait so tests can record dispatches.
pub trait Compositor: Send + Sync + 'static {
    /// Run a dispatcher: the Lua form, else the classic one.
    fn dispatch(&self, d: &Dispatch) -> Result<(), String>;
    /// The monitor under the pointer (reads its position; never moves it).
    fn pointer_monitor(&self) -> Option<String>;
}

pub struct Hyprland;

impl Compositor for Hyprland {
    fn dispatch(&self, d: &Dispatch) -> Result<(), String> {
        let hypr = Instance::find().map_err(|e| format!("{e:#}"))?;
        hypr.dispatch(&d.lua).or_else(|_| hypr.dispatch(&d.classic)).map_err(|e| format!("{e:#}"))
    }

    fn pointer_monitor(&self) -> Option<String> {
        #[derive(Deserialize)]
        struct Pos {
            x: i32,
            y: i32,
        }
        #[derive(Deserialize)]
        struct Mon {
            name: String,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            #[serde(default)]
            scale: f64,
            #[serde(default)]
            transform: i32,
        }
        let hypr = Instance::find().ok()?;
        let p: Pos = hypr.json("cursorpos").ok()?;
        let monitors: Vec<Mon> = hypr.json("monitors").ok()?;
        monitors
            .into_iter()
            .find(|m| {
                let s = if m.scale > 0.0 { m.scale } else { 1.0 };
                let (mut w, mut h) = ((m.width as f64 / s) as i32, (m.height as f64 / s) as i32);
                if m.transform % 2 == 1 {
                    std::mem::swap(&mut w, &mut h);
                }
                p.x >= m.x && p.x < m.x + w && p.y >= m.y && p.y < m.y + h
            })
            .map(|m| m.name)
    }
}

/// One update from the watcher thread.
pub struct Update {
    pub desktop: Arc<Desktop>,
    pub context: crate::window::Context,
}

/// Start the watcher; `send` runs on its thread for every update. Omarchy's workspace
/// modes and nyx-decor's headerbars are read there too, as reading them asks Hyprland.
pub fn watch(send: impl Fn(Update) + Send + 'static) -> nyx_desktop::watch::Watcher {
    let omarchy_minimize = crate::actions::which(crate::window::OMARCHY_MINIMIZE).is_some();
    nyx_desktop::watch::watch(move |u| {
        let context = crate::window::Context {
            minimize_workspaces: if omarchy_minimize { floating_workspaces(&u.desktop) } else { Default::default() },
            default_headerbars: if u.desktop.connected { default_headerbars() } else { Default::default() },
        };
        send(Update { desktop: u.desktop, context })
    })
}

/// The windows nyx-decor gave a headerbar no rule asked for (`hyprctl -j nyxdecor`).
/// Without the plugin Hyprland doesn't know the request: none.
fn default_headerbars() -> std::collections::BTreeSet<String> {
    #[derive(serde::Deserialize)]
    struct Reply {
        default_headerbars: Vec<String>,
    }
    Instance::find()
        .and_then(|i| i.json::<Reply>("nyxdecor"))
        .map(|r| parse_addresses(r.default_headerbars))
        .unwrap_or_default()
}

/// Hyprland's `0x…` addresses as Shell Core writes them: lowercase hex without `0x`.
fn parse_addresses(list: Vec<String>) -> std::collections::BTreeSet<String> {
    list.iter().map(|a| a.trim_start_matches("0x").to_ascii_lowercase()).collect()
}

/// Omarchy's floating workspaces, where its minimize works.
fn floating_workspaces(d: &Desktop) -> std::collections::BTreeSet<i64> {
    use nyx_desktop::omarchy::{Mode, workspace_mode};
    d.workspaces.iter().filter(|w| w.id >= 1 && workspace_mode(w.id, &w.tiled_layout) == Mode::Floating).map(|w| w.id).collect()
}

/// What a window action becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowRoute {
    Dispatch(Vec<Dispatch>),
    /// A fixed program (Omarchy's minimize), waited for.
    Run(Vec<String>),
}

fn invalid(msg: impl Into<String>) -> ErrorBody {
    ErrorBody::new(ErrorCode::Invalid, msg)
}

fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A workspace as a client may name it: an id, `name:NAME` or `special:NAME`.
pub fn valid_workspace(s: &str) -> bool {
    if let Some(n) = s.strip_prefix("name:").or_else(|| s.strip_prefix("special:")) {
        return plain_name(n);
    }
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.len() <= 10 && digits.bytes().all(|b| b.is_ascii_digit())
}

pub fn workspace_dispatch(action: &WorkspaceAction) -> Result<Dispatch, ErrorBody> {
    match action {
        WorkspaceAction::Focus { workspace } => {
            if !valid_workspace(workspace) {
                return Err(invalid(format!("invalid workspace {workspace:?}")));
            }
            Ok(Dispatch { lua: format!("hl.dsp.focus({{ workspace = \"{workspace}\" }})"), classic: format!("workspace {workspace}") })
        }
        WorkspaceAction::ToggleSpecial { name } => {
            if !plain_name(name) {
                return Err(invalid(format!("invalid special workspace {name:?}")));
            }
            Ok(Dispatch { lua: format!("hl.dsp.workspace.toggle_special(\"{name}\")"), classic: format!("togglespecialworkspace {name}") })
        }
    }
}

/// The dispatchers for `a`, after checking that the window exists and can take the
/// action now (`WindowState::actions`).
pub fn window_dispatch(a: &WindowAction, desktop: &DesktopState) -> Result<WindowRoute, ErrorBody> {
    if !a.window.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("window addresses are hex"));
    }
    let w = desktop.window(&a.window).ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, format!("no window {}", a.window)))?;
    if !w.actions.is_empty() && !w.actions.contains(&a.action) {
        return Err(invalid(format!("{} isn't available for this window now", serde_json::to_value(a.action).unwrap_or_default())));
    }
    let addr = &w.address;
    Ok(WindowRoute::Dispatch(match a.action {
        WindowActionKind::Focus => actions::activate(addr, w.minimized, desktop.active.workspace.unwrap_or(0)),
        WindowActionKind::Close => vec![actions::close_window(addr)],
        WindowActionKind::ToggleFloating => vec![actions::toggle_floating(addr)],
        WindowActionKind::SetFloating => vec![window::set_floating(addr, true)],
        WindowActionKind::SetTiled => vec![window::set_floating(addr, false)],
        WindowActionKind::ToggleMaximized => vec![actions::toggle_maximized(addr)],
        WindowActionKind::ToggleFullscreen => vec![window::toggle_fullscreen(addr)],
        WindowActionKind::TogglePinned => vec![window::toggle_pinned(addr)],
        WindowActionKind::Minimize => return Ok(WindowRoute::Run(vec![window::OMARCHY_MINIMIZE.into(), format!("0x{addr}")])),
        WindowActionKind::MoveToWorkspace => {
            let ws = a.workspace.as_deref().ok_or_else(|| invalid("move_to_workspace needs a workspace"))?;
            if !valid_workspace(ws) {
                return Err(invalid(format!("invalid workspace {ws:?}")));
            }
            vec![actions::move_to_workspace(addr, ws, a.follow)]
        }
        WindowActionKind::MoveToMonitor => {
            let m = a.monitor.as_deref().ok_or_else(|| invalid("move_to_monitor needs a monitor"))?;
            let m = desktop.monitor(m).ok_or_else(|| ErrorBody::new(ErrorCode::NotFound, format!("no monitor {m}")))?;
            vec![actions::move_to_monitor(addr, &m.name)]
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_shell_proto::{MonitorState, WindowState};

    #[test]
    fn plugin_addresses() {
        let got = parse_addresses(vec!["0x5586DC248020".into(), "618bd598c160".into()]);
        assert_eq!(got, ["5586dc248020".to_string(), "618bd598c160".to_string()].into());
    }

    fn desktop() -> DesktopState {
        DesktopState {
            connected: true,
            monitors: vec![MonitorState { name: "DP-1".into(), ..Default::default() }],
            windows: vec![WindowState { address: "abc".into(), workspace: 1, ..Default::default() }],
            ..Default::default()
        }
    }

    fn act(window: &str, action: WindowActionKind) -> WindowAction {
        WindowAction { window: window.into(), action, workspace: None, monitor: None, follow: false }
    }

    #[test]
    fn workspaces() {
        let d = workspace_dispatch(&WorkspaceAction::Focus { workspace: "3".into() }).unwrap();
        assert_eq!(d.lua, "hl.dsp.focus({ workspace = \"3\" })");
        assert_eq!(d.classic, "workspace 3");
        let s = workspace_dispatch(&WorkspaceAction::ToggleSpecial { name: "scratch".into() }).unwrap();
        assert_eq!(s.lua, "hl.dsp.workspace.toggle_special(\"scratch\")");
        for bad in ["", "3\"})", "name:", "name:a b", "special:x;y", "1e9", "--1"] {
            assert!(workspace_dispatch(&WorkspaceAction::Focus { workspace: bad.into() }).is_err(), "{bad}");
        }
        assert!(valid_workspace("-98"));
        assert!(valid_workspace("special:minimized"));
    }

    fn steps(a: &WindowAction, d: &DesktopState) -> Vec<Dispatch> {
        match window_dispatch(a, d).unwrap() {
            WindowRoute::Dispatch(s) => s,
            WindowRoute::Run(argv) => panic!("ran {argv:?}"),
        }
    }

    #[test]
    fn windows_must_exist() {
        let d = desktop();
        assert_eq!(steps(&act("abc", WindowActionKind::Close), &d)[0].classic, "closewindow address:0xabc");
        assert_eq!(window_dispatch(&act("def", WindowActionKind::Close), &d).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(window_dispatch(&act("ab\"c", WindowActionKind::Close), &d).unwrap_err().code, ErrorCode::Invalid);
        let mut m = act("abc", WindowActionKind::MoveToMonitor);
        m.monitor = Some("HDMI-9".into());
        assert_eq!(window_dispatch(&m, &d).unwrap_err().code, ErrorCode::NotFound);
        m.monitor = Some("DP-1".into());
        assert_eq!(steps(&m, &d)[0].classic, "movewindow mon:DP-1,address:0xabc");
        let mut w = act("abc", WindowActionKind::MoveToWorkspace);
        assert!(window_dispatch(&w, &d).is_err());
        w.workspace = Some("special:x\"".into());
        assert!(window_dispatch(&w, &d).is_err());
        w.workspace = Some("4".into());
        w.follow = true;
        assert_eq!(steps(&w, &d)[0].classic, "movetoworkspace 4,address:0xabc");
        let f = steps(&act("abc", WindowActionKind::Focus), &d);
        assert_eq!(f.len(), 2, "focus and raise");
    }

    #[test]
    fn only_what_the_window_can_do_now() {
        let mut d = desktop();
        let here = [1].into();
        d.windows[0].actions = nyx_shell_proto::window::available(&d.windows[0], &here);
        assert_eq!(steps(&act("abc", WindowActionKind::SetFloating), &d)[0].classic, "setfloating address:0xabc");
        for refused in [WindowActionKind::SetTiled, WindowActionKind::TogglePinned, WindowActionKind::Minimize] {
            assert_eq!(window_dispatch(&act("abc", refused), &d).unwrap_err().code, ErrorCode::Invalid, "{refused:?} on a tiled window");
        }
        d.windows[0].floating = true;
        d.windows[0].actions = nyx_shell_proto::window::available(&d.windows[0], &here);
        assert_eq!(
            window_dispatch(&act("abc", WindowActionKind::Minimize), &d).unwrap(),
            WindowRoute::Run(vec!["omarchy-hyprland-window-minimize".into(), "0xabc".into()])
        );
        assert_eq!(steps(&act("abc", WindowActionKind::TogglePinned), &d)[0].classic, "pin address:0xabc");
        assert!(window_dispatch(&act("abc", WindowActionKind::SetFloating), &d).is_err(), "it floats already");
    }
}
