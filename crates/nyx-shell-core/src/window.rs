//! Window controls on Shell Core's side: each window's presentation and actions from
//! the policy in `nyx_shell_proto::window`, and the dispatchers behind the actions
//! `nyx-desktop` doesn't have.

use nyx_desktop::actions::Dispatch;
use nyx_shell_proto::window::{available, controls};
use nyx_shell_proto::{Controls, DesktopState, WindowControlsConfig};
use std::collections::BTreeSet;

/// Omarchy's minimize, which keeps the window on `special:minimized` for the dock.
pub const OMARCHY_MINIMIZE: &str = "omarchy-hyprland-window-minimize";

/// What the compositor side knows beyond the window list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// Workspaces where a window can be minimized and brought back: Omarchy's floating
    /// workspaces, when its minimize is installed. Empty on NyxOS, which has no such place.
    pub minimize_workspaces: BTreeSet<i64>,
    /// Windows NyxOS's decorations (nyx-decor) gave a headerbar though no rule asked:
    /// apps that left their decorations to the compositor. Addresses as in
    /// `WindowState`.
    pub default_headerbars: BTreeSet<String>,
}

/// Fill in each window's presentation and actions.
pub fn annotate(state: &mut DesktopState, cfg: &WindowControlsConfig, ctx: &Context) {
    for w in &mut state.windows {
        w.controls = controls(w, cfg).or_else(|| ctx.default_headerbars.contains(&w.address).then_some(Controls::CustomHeaderbar));
        w.actions = available(w, &ctx.minimize_workspaces);
    }
}

fn target(address: &str) -> String {
    format!("window = \"address:0x{address}\"")
}

/// Float (`on`) or tile a window, whatever it is now. Hyprland's float takes `enable`
/// and `disable` (fullscreen's `set` and `unset` would toggle).
pub fn set_floating(address: &str, on: bool) -> Dispatch {
    let (action, classic) = if on { ("enable", "setfloating") } else { ("disable", "settiled") };
    Dispatch {
        lua: format!("hl.dsp.window.float({{ action = \"{action}\", {} }})", target(address)),
        classic: format!("{classic} address:0x{address}"),
    }
}

/// True fullscreen on or off. The classic form acts on the focused window.
pub fn toggle_fullscreen(address: &str) -> Dispatch {
    Dispatch {
        lua: format!("hl.dsp.window.fullscreen({{ mode = \"fullscreen\", action = \"toggle\", {} }})", target(address)),
        classic: "fullscreen 0".into(),
    }
}

pub fn toggle_pinned(address: &str) -> Dispatch {
    Dispatch {
        lua: format!("hl.dsp.window.pin({{ action = \"toggle\", {} }})", target(address)),
        classic: format!("pin address:0x{address}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatchers() {
        assert_eq!(set_floating("ab", true).lua, "hl.dsp.window.float({ action = \"enable\", window = \"address:0xab\" })");
        assert!(set_floating("ab", false).lua.contains("action = \"disable\""));
        assert_eq!(set_floating("ab", false).classic, "settiled address:0xab");
        assert_eq!(
            toggle_fullscreen("ab").lua,
            "hl.dsp.window.fullscreen({ mode = \"fullscreen\", action = \"toggle\", window = \"address:0xab\" })"
        );
        assert_eq!(toggle_pinned("ab").classic, "pin address:0xab");
    }

    #[test]
    fn default_headerbars_unless_a_rule_decides() {
        let win = |address: &str, app_id: &str| nyx_shell_proto::WindowState { address: address.into(), app_id: app_id.into(), ..Default::default() };
        let mut state = DesktopState { windows: vec![win("a1", "org.qt.app"), win("b2", "foot"), win("c3", "mpv")], ..Default::default() };
        let ctx = Context { default_headerbars: ["a1".to_string(), "b2".to_string()].into(), ..Default::default() };
        annotate(&mut state, &WindowControlsConfig::default(), &ctx);
        assert_eq!(state.windows[0].controls, Some(Controls::CustomHeaderbar));
        assert_eq!(state.windows[1].controls, Some(Controls::ChromelessFloating));
        assert_eq!(state.windows[2].controls, None);
    }
}
