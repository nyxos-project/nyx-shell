//! Window controls policy: which actions a window can take now, how its controls are
//! presented, and the Hyprland rules that carry `[window_controls]` to the compositor's
//! decorations.
//!
//! Every presentation (a Nyx app's popover, the compositor's headerbar, the chromeless
//! control) shows what [`available`] lists and asks for it with `RequestWindowAction`,
//! so they can't disagree about what a window can do.

use crate::config::WindowControlsConfig;
use crate::desktop::{Controls, Fullscreen, WindowState};
use crate::frame::WindowActionKind as A;
use std::collections::BTreeSet;

/// Nyx apps' ids start with this; they draw their own header with the native control.
pub const NYX_APP_PREFIX: &str = "io.github.design_nexus.nyx.";

/// Apps that have no chrome on purpose. A value in `[window_controls.apps]` wins.
pub const BUILTIN: &[(&str, Controls)] = &[
    ("foot", Controls::ChromelessFloating),
    ("footclient", Controls::ChromelessFloating),
    ("kitty", Controls::ChromelessFloating),
    ("Alacritty", Controls::ChromelessFloating),
    ("com.mitchellh.ghostty", Controls::ChromelessFloating),
    ("org.wezfurlong.wezterm", Controls::ChromelessFloating),
];

/// The tag a window's presentation is carried to Hyprland with: `nyx-controls-KIND`.
pub fn tag(c: Controls) -> String {
    format!("nyx-controls-{}", c.name())
}

/// How `w`'s controls are presented: a rule for its app id or class, the built-in
/// rules, a Nyx app's own header, or nothing known (the compositor decides).
pub fn controls(w: &WindowState, cfg: &WindowControlsConfig) -> Option<Controls> {
    if !cfg.enabled {
        return None;
    }
    let ids = [w.app_id.as_str(), w.initial_app_id.as_str()];
    if let Some(c) = ids.iter().find_map(|id| cfg.apps.get(*id).and_then(|r| r.controls)) {
        return Some(c);
    }
    if let Some(c) = ids.iter().find_map(|id| BUILTIN.iter().find(|(a, _)| a == id).map(|(_, c)| *c)) {
        return Some(c);
    }
    ids.iter().any(|id| id.starts_with(NYX_APP_PREFIX)).then_some(Controls::NativeNyx)
}

/// The actions that make sense for `w` now, in the table's order. `minimize_workspaces`
/// are the workspaces a minimized window can be brought back from. A minimized window
/// can only come back or close; true fullscreen first has to be left; pinning and
/// minimizing are for floating windows.
pub fn available(w: &WindowState, minimize_workspaces: &BTreeSet<i64>) -> Vec<A> {
    let mut out = vec![A::Focus, A::Close];
    if w.minimized {
        return out;
    }
    out.push(A::ToggleFullscreen);
    out.push(A::MoveToMonitor);
    if !w.pinned {
        out.push(A::MoveToWorkspace);
    }
    if w.fullscreen != Fullscreen::Fullscreen {
        out.extend([A::ToggleFloating, if w.floating { A::SetTiled } else { A::SetFloating }, A::ToggleMaximized]);
        if w.floating {
            out.push(A::TogglePinned);
            if !w.pinned && minimize_workspaces.contains(&w.workspace) {
                out.push(A::Minimize);
            }
        }
    }
    out.sort();
    out
}

/// `[window_controls]` as Hyprland rules: a `nyx-controls-KIND` tag per app with a
/// presentation (the compositor's decorations read it) and `float` or `tile` per app
/// with a preference. Built-in rules come first, so an app's own rule replaces them.
pub fn rules_lua(cfg: &WindowControlsConfig) -> String {
    let mut out = String::from(
        "-- Written by `nyx-shellctl window-controls --apply` from [window_controls] in\n\
         -- ~/.config/nyx/shell.toml (and NyxOS's defaults). Change it there; this file is replaced.\n",
    );
    let mut tagged: Vec<(&str, Controls)> = Vec::new();
    if cfg.enabled {
        for (id, c) in BUILTIN {
            if !cfg.apps.get(*id).is_some_and(|r| r.controls.is_some()) {
                tagged.push((id, *c));
            }
        }
        for (id, r) in &cfg.apps {
            if let Some(c) = r.controls {
                tagged.push((id, c));
            }
        }
    }
    for (id, c) in tagged {
        out.push_str(&format!("hl.window_rule({{ match = {{ class = \"{}\" }}, tag = \"+{}\" }})\n", class_regex(id), tag(c)));
    }
    for (id, r) in &cfg.apps {
        if let Some(float) = r.float {
            let effect = if float { "float" } else { "tile" };
            out.push_str(&format!("hl.window_rule({{ match = {{ class = \"{}\" }}, {effect} = true }})\n", class_regex(id)));
        }
    }
    out
}

/// An app id as an anchored Hyprland regex, written inside a Lua string. Ids are
/// [`crate::config::valid_app_id`], so only `.` and `+` need escaping.
fn class_regex(id: &str) -> String {
    let mut s = String::from("^");
    for ch in id.chars() {
        if ch == '.' || ch == '+' {
            s.push_str("\\\\");
        }
        s.push(ch);
    }
    s.push('$');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppWindowRule;

    fn win(app: &str) -> WindowState {
        WindowState { address: "a1".into(), app_id: app.into(), initial_app_id: app.into(), workspace: 1, ..Default::default() }
    }

    #[test]
    fn presentation_follows_rules_then_builtins_then_nyx() {
        let mut cfg = WindowControlsConfig::default();
        assert_eq!(controls(&win("foot"), &cfg), Some(Controls::ChromelessFloating));
        assert_eq!(controls(&win("io.github.design_nexus.nyx.Files"), &cfg), Some(Controls::NativeNyx));
        assert_eq!(controls(&win("firefox"), &cfg), None, "unknown apps are the compositor's call");
        cfg.apps.insert("foot".into(), AppWindowRule { controls: Some(Controls::CustomHeaderbar), float: None });
        cfg.apps.insert("firefox".into(), AppWindowRule { controls: Some(Controls::PreserveNative), float: None });
        assert_eq!(controls(&win("foot"), &cfg), Some(Controls::CustomHeaderbar), "a rule beats the built-in one");
        assert_eq!(controls(&win("firefox"), &cfg), Some(Controls::PreserveNative));
        let mut renamed = win("Something");
        renamed.initial_app_id = "kitty".into();
        assert_eq!(controls(&renamed, &cfg), Some(Controls::ChromelessFloating), "the initial class counts too");
        cfg.enabled = false;
        assert_eq!(controls(&win("foot"), &cfg), None);
    }

    #[test]
    fn actions_depend_on_the_mode() {
        let here: BTreeSet<i64> = [1].into();
        let tiled = available(&win("kitty"), &here);
        assert!(tiled.contains(&A::SetFloating) && tiled.contains(&A::ToggleMaximized), "a tiled window floats or expands");
        assert!(!tiled.contains(&A::SetTiled) && !tiled.contains(&A::TogglePinned) && !tiled.contains(&A::Minimize));

        let floating = WindowState { floating: true, ..win("kitty") };
        let f = available(&floating, &here);
        assert!(f.contains(&A::SetTiled) && f.contains(&A::TogglePinned) && f.contains(&A::Minimize));
        assert!(!available(&floating, &BTreeSet::new()).contains(&A::Minimize), "no minimize where nothing brings it back");
        assert!(!available(&WindowState { workspace: 2, ..floating.clone() }, &here).contains(&A::Minimize));

        let pinned = WindowState { pinned: true, ..floating.clone() };
        let p = available(&pinned, &here);
        assert!(p.contains(&A::TogglePinned) && !p.contains(&A::MoveToWorkspace) && !p.contains(&A::Minimize));

        let full = WindowState { fullscreen: Fullscreen::Fullscreen, ..floating.clone() };
        assert_eq!(available(&full, &here), [A::Focus, A::Close, A::ToggleFullscreen, A::MoveToWorkspace, A::MoveToMonitor]);
        let maximized = WindowState { fullscreen: Fullscreen::Maximized, ..win("kitty") };
        assert!(available(&maximized, &here).contains(&A::ToggleMaximized), "an expanded window can be restored");

        let minimized = WindowState { minimized: true, ..floating };
        assert_eq!(available(&minimized, &here), [A::Focus, A::Close]);
        let sorted = available(&win("kitty"), &here);
        assert!(sorted.windows(2).all(|p| p[0] < p[1]));
    }

    #[test]
    fn rules() {
        let mut cfg = WindowControlsConfig::default();
        cfg.apps.insert("kitty".into(), AppWindowRule { controls: Some(Controls::PreserveNative), float: Some(true) });
        cfg.apps.insert("org.kde.dolphin".into(), AppWindowRule { controls: Some(Controls::CustomHeaderbar), float: Some(false) });
        cfg.apps.insert("g++".into(), AppWindowRule { controls: None, float: None });
        let lua = rules_lua(&cfg);
        assert!(lua.contains("hl.window_rule({ match = { class = \"^foot$\" }, tag = \"+nyx-controls-chromeless-floating\" })"));
        assert!(!lua.contains("^kitty$\" }, tag = \"+nyx-controls-chromeless"), "the app's own rule replaces the built-in one");
        assert!(lua.contains("class = \"^kitty$\" }, tag = \"+nyx-controls-preserve-native\""));
        assert!(lua.contains("class = \"^org\\\\.kde\\\\.dolphin$\" }, tag = \"+nyx-controls-custom-headerbar\""));
        assert!(lua.contains("class = \"^kitty$\" }, float = true"));
        assert!(lua.contains("dolphin$\" }, tile = true"));
        assert!(!lua.contains("g\\\\+\\\\+"), "no rule without a value");
        cfg.enabled = false;
        let off = rules_lua(&cfg);
        assert!(!off.contains("tag ="), "off: no presentations, preferences stay");
        assert!(off.contains("float = true"));
        assert_eq!(class_regex("a+b.c"), "^a\\\\+b\\\\.c$");
    }
}
