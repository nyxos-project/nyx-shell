//! The `shell` settings domain (`shell.toml` in the nyx-config layers).
//!
//! Each section is read on its own: one that doesn't parse or validate falls back to
//! its defaults and is reported, so one mistake never takes the others down.

use crate::overlay::{FullscreenRule, Placement};
use crate::shortcut::Shortcut;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub focus: FocusConfig,
    pub ipc: IpcConfig,
    pub log: LogConfig,
    pub monitors: MonitorsConfig,
    /// Overrides for registered overlays, by id.
    pub overlays: BTreeMap<String, OverlayOverride>,
    /// Keys for actions: `"shell.control-center.toggle" = "SUPER + D"`. Empty by default,
    /// so the keys NyxOS's Hyprland config binds stay owned there.
    pub shortcuts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusConfig {
    /// Give focus back to the previous window when a transient overlay closes.
    pub restore: bool,
    /// At most one restore per this many milliseconds.
    pub debounce_ms: u64,
}

impl Default for FocusConfig {
    fn default() -> Self {
        FocusConfig { restore: true, debounce_ms: 500 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpcConfig {
    /// Events queued per client before it is sent a `Resync` instead.
    pub max_queue: usize,
    /// Quickshell executables trusted as the shell's UI runtime.
    pub ui_runtimes: Vec<String>,
}

impl Default for IpcConfig {
    fn default() -> Self {
        IpcConfig { max_queue: 256, ui_runtimes: vec!["/usr/bin/quickshell".into()] }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// Allow window titles in debug logs (local troubleshooting only).
    pub sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorsConfig {
    /// Put persistent surfaces on headless outputs too.
    pub include_virtual: bool,
    /// Per screen, by stable id (make|model|serial, or the connector name).
    pub screens: BTreeMap<String, ScreenConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenConfig {
    /// Shell surfaces on this screen at all.
    pub enabled: bool,
    /// Per component id: on or off on this screen.
    pub components: BTreeMap<String, bool>,
}

impl Default for ScreenConfig {
    fn default() -> Self {
        ScreenConfig { enabled: true, components: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayOverride {
    pub group: Option<String>,
    pub fullscreen: Option<FullscreenRule>,
    pub placement: Option<Placement>,
    pub restore_focus: Option<bool>,
}

/// The merged settings as clients see them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConfigState {
    pub generation: u64,
    pub config: ShellConfig,
    /// Sections that fell back to their defaults, and why.
    pub errors: Vec<String>,
}

const SECTIONS: [&str; 6] = ["focus", "ipc", "log", "monitors", "overlays", "shortcuts"];

impl ShellConfig {
    /// Read merged settings. Returns the config and the problems found.
    pub fn from_table(table: &toml::Table) -> (ShellConfig, Vec<String>) {
        let mut c = ShellConfig::default();
        let mut errors = Vec::new();
        for key in table.keys() {
            if !SECTIONS.contains(&key.as_str()) {
                errors.push(format!("{key}: unknown section, ignored"));
            }
        }
        section(table, "focus", &mut c.focus, &mut errors, |f| {
            if f.debounce_ms > 10_000 {
                return Err("debounce_ms is at most 10000".into());
            }
            Ok(())
        });
        section(table, "ipc", &mut c.ipc, &mut errors, |i| {
            if !(16..=65_536).contains(&i.max_queue) {
                return Err("max_queue is between 16 and 65536".into());
            }
            if i.ui_runtimes.len() > 8 || i.ui_runtimes.iter().any(|p| !p.starts_with('/') || p.contains("..")) {
                return Err("ui_runtimes are at most 8 absolute paths".into());
            }
            Ok(())
        });
        section(table, "log", &mut c.log, &mut errors, |_| Ok(()));
        section(table, "monitors", &mut c.monitors, &mut errors, |m| {
            if m.screens.len() > 32 || m.screens.keys().any(|k| k.is_empty() || k.len() > 256) {
                return Err("at most 32 screens, ids of 1 to 256 characters".into());
            }
            for s in m.screens.values() {
                if s.components.keys().any(|k| !crate::valid_id(k)) {
                    return Err("component ids are [a-z0-9._:-]".into());
                }
            }
            Ok(())
        });
        section(table, "overlays", &mut c.overlays, &mut errors, |o| {
            for (id, ov) in o {
                if !crate::valid_id(id) {
                    return Err(format!("invalid overlay id {id:?}"));
                }
                if let Some(g) = &ov.group
                    && !crate::valid_id(g)
                {
                    return Err(format!("{id}: invalid group {g:?}"));
                }
            }
            Ok(())
        });
        section(table, "shortcuts", &mut c.shortcuts, &mut errors, |s| {
            for (action, keys) in s {
                if !crate::valid_id(action) {
                    return Err(format!("invalid action id {action:?}"));
                }
                Shortcut::parse(keys).map_err(|e| format!("{action}: {e}"))?;
            }
            Ok(())
        });
        (c, errors)
    }
}

fn section<T: DeserializeOwned>(
    table: &toml::Table,
    name: &str,
    slot: &mut T,
    errors: &mut Vec<String>,
    check: impl Fn(&T) -> Result<(), String>,
) {
    let Some(value) = table.get(name) else { return };
    match value.clone().try_into::<T>() {
        Ok(v) => match check(&v) {
            Ok(()) => *slot = v,
            Err(e) => errors.push(format!("{name}: {e}; using the defaults")),
        },
        Err(e) => errors.push(format!("{name}: {}; using the defaults", e.to_string().trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (ShellConfig, Vec<String>) {
        ShellConfig::from_table(&text.parse::<toml::Table>().unwrap())
    }

    #[test]
    fn empty_is_defaults() {
        let (c, e) = read("");
        assert!(e.is_empty());
        assert_eq!(c, ShellConfig::default());
        assert!(c.focus.restore);
        assert!(c.shortcuts.is_empty());
    }

    #[test]
    fn a_broken_section_falls_back_alone() {
        let (c, e) = read(
            r#"
            [focus]
            restore = "yes"
            [ipc]
            max_queue = 64
            [shortcuts]
            "shell.control-center.toggle" = "SUPER + D"
            "#,
        );
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].starts_with("focus:"));
        assert!(c.focus.restore, "default kept");
        assert_eq!(c.ipc.max_queue, 64);
        assert_eq!(c.shortcuts["shell.control-center.toggle"], "SUPER + D");
    }

    #[test]
    fn semantic_checks() {
        let (c, e) = read("[ipc]\nmax_queue = 1\n[shortcuts]\n\"x\" = \"SUPER + \"\n[nonsense]\na = 1\n");
        assert_eq!(c.ipc.max_queue, 256);
        assert!(c.shortcuts.is_empty());
        assert_eq!(e.len(), 3, "{e:?}");
    }

    #[test]
    fn the_shipped_defaults_are_valid() {
        let (c, e) = read(include_str!("../../../data/shell.toml"));
        assert!(e.is_empty(), "{e:?}");
        assert!(c.shortcuts.is_empty(), "NyxOS's Hyprland config owns the keys");
        assert_eq!(c.ipc, IpcConfig::default());
        assert_eq!(c.focus, FocusConfig::default());
    }

    #[test]
    fn overlays_and_screens() {
        let (c, e) = read(
            r#"
            [overlays."nyx.inspector"]
            fullscreen = "block"
            placement = "monitor:DP-2"
            [monitors.screens."Dell Inc.|U2723QE|ABC"]
            enabled = false
            "#,
        );
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(c.overlays["nyx.inspector"].fullscreen, Some(FullscreenRule::Block));
        assert_eq!(c.overlays["nyx.inspector"].placement, Some(Placement::Monitor("DP-2".into())));
        assert!(!c.monitors.screens["Dell Inc.|U2723QE|ABC"].enabled);
    }
}
