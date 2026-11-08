//! The normalized desktop: monitors, workspaces, windows, and deltas between states.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DesktopState {
    pub generation: u64,
    /// False while Hyprland is unreachable; the rest is the last known state.
    pub connected: bool,
    pub monitors: Vec<MonitorState>,
    pub workspaces: Vec<WorkspaceState>,
    pub windows: Vec<WindowState>,
    pub active: Active,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Active {
    pub monitor: Option<String>,
    pub workspace: Option<i64>,
    pub window: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MonitorState {
    /// Connector name (`eDP-1`). Not a stable identity: it follows the port.
    pub name: String,
    /// make|model|serial, or the connector name when the screen reports none.
    pub stable_id: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    /// Pixels, before scale.
    pub width: i32,
    pub height: i32,
    /// After scale and rotation.
    pub logical_width: i32,
    pub logical_height: i32,
    pub scale: f64,
    pub transform: i32,
    pub refresh_rate: Option<f64>,
    pub focused: bool,
    pub enabled: bool,
    #[serde(rename = "virtual")]
    pub is_virtual: bool,
    pub active_workspace: i64,
    pub special_workspace: Option<i64>,
    /// Left, top, right, bottom, logical pixels.
    pub reserved: [i32; 4],
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub id: i64,
    pub name: String,
    pub monitor: String,
    /// Shown on its monitor (as the active or the open special workspace).
    pub active: bool,
    pub special: bool,
    /// One of the user's workspaces, 1 to 10.
    pub user: bool,
    pub has_fullscreen: bool,
    /// Window addresses, most recently focused first.
    pub windows: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fullscreen {
    #[default]
    None,
    Maximized,
    Fullscreen,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WindowState {
    /// Lowercase hex without `0x`.
    pub address: String,
    pub stable_id: String,
    pub app_id: String,
    pub initial_app_id: String,
    /// Untrusted text, at most 512 characters, no control characters.
    pub title: String,
    pub workspace: i64,
    pub monitor: Option<String>,
    pub focused: bool,
    pub floating: bool,
    pub fullscreen: Fullscreen,
    pub hidden: bool,
    pub minimized: bool,
    pub pinned: bool,
    pub urgent: bool,
    pub focus_order: i32,
}

/// Upserts and removals for one collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection<T, K> {
    pub upsert: Vec<T>,
    pub remove: Vec<K>,
}

impl<T, K> Default for Collection<T, K> {
    fn default() -> Self {
        Collection { upsert: Vec::new(), remove: Vec::new() }
    }
}

impl<T, K> Collection<T, K> {
    pub fn is_empty(&self) -> bool {
        self.upsert.is_empty() && self.remove.is_empty()
    }
}

/// What changed from generation `base` to `generation`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DesktopDelta {
    pub generation: u64,
    pub base: u64,
    pub connected: Option<bool>,
    pub monitors: Collection<MonitorState, String>,
    pub workspaces: Collection<WorkspaceState, i64>,
    pub windows: Collection<WindowState, String>,
    pub active: Option<Active>,
}

impl DesktopDelta {
    /// Nothing but the generation moved.
    pub fn is_empty(&self) -> bool {
        self.connected.is_none()
            && self.active.is_none()
            && self.monitors.is_empty()
            && self.workspaces.is_empty()
            && self.windows.is_empty()
    }
}

/// A delta whose base isn't the state's generation: fetch a snapshot instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleDelta {
    pub have: u64,
    pub base: u64,
}

impl std::fmt::Display for StaleDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "delta from generation {} doesn't apply to generation {}", self.base, self.have)
    }
}

impl std::error::Error for StaleDelta {}

impl DesktopState {
    /// What changed from `old` to `self`.
    pub fn delta_from(&self, old: &DesktopState) -> DesktopDelta {
        DesktopDelta {
            generation: self.generation,
            base: old.generation,
            connected: (self.connected != old.connected).then_some(self.connected),
            monitors: diff(&old.monitors, &self.monitors, |m| m.name.clone()),
            workspaces: diff(&old.workspaces, &self.workspaces, |w| w.id),
            windows: diff(&old.windows, &self.windows, |w| w.address.clone()),
            active: (self.active != old.active).then(|| self.active.clone()),
        }
    }

    /// Apply a delta. Workspaces come out sorted by id and windows by focus order, as
    /// the server sends them.
    pub fn apply(&mut self, delta: &DesktopDelta) -> Result<(), StaleDelta> {
        if delta.base != self.generation {
            return Err(StaleDelta { have: self.generation, base: delta.base });
        }
        if let Some(c) = delta.connected {
            self.connected = c;
        }
        patch(&mut self.monitors, &delta.monitors, |m| m.name.clone());
        patch(&mut self.workspaces, &delta.workspaces, |w| w.id);
        self.workspaces.sort_by_key(|w| w.id);
        patch(&mut self.windows, &delta.windows, |w| w.address.clone());
        self.windows.sort_by_key(|w| w.focus_order);
        if let Some(a) = &delta.active {
            self.active = a.clone();
        }
        self.generation = delta.generation;
        Ok(())
    }

    pub fn monitor(&self, name: &str) -> Option<&MonitorState> {
        self.monitors.iter().find(|m| m.name == name)
    }

    pub fn window(&self, address: &str) -> Option<&WindowState> {
        self.windows.iter().find(|w| w.address == address)
    }

    pub fn workspace(&self, id: i64) -> Option<&WorkspaceState> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    /// Whether a fullscreen window is on screen on `monitor`.
    pub fn fullscreen_on(&self, monitor: &str) -> bool {
        let Some(m) = self.monitor(monitor) else { return false };
        [Some(m.active_workspace), m.special_workspace]
            .into_iter()
            .flatten()
            .any(|id| self.windows.iter().any(|w| w.workspace == id && w.fullscreen == Fullscreen::Fullscreen && !w.hidden))
    }
}

fn diff<T: Clone + PartialEq, K: Ord + Clone>(old: &[T], new: &[T], key: impl Fn(&T) -> K) -> Collection<T, K> {
    let before: BTreeMap<K, &T> = old.iter().map(|t| (key(t), t)).collect();
    let after: BTreeMap<K, &T> = new.iter().map(|t| (key(t), t)).collect();
    Collection {
        upsert: new.iter().filter(|t| before.get(&key(t)) != Some(t)).cloned().collect(),
        remove: before.keys().filter(|k| !after.contains_key(*k)).cloned().collect(),
    }
}

fn patch<T: Clone, K: PartialEq>(items: &mut Vec<T>, c: &Collection<T, K>, key: impl Fn(&T) -> K) {
    items.retain(|t| !c.remove.contains(&key(t)));
    for new in &c.upsert {
        match items.iter_mut().find(|t| key(t) == key(new)) {
            Some(slot) => *slot = new.clone(),
            None => items.push(new.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(addr: &str, ws: i64, order: i32) -> WindowState {
        WindowState { address: addr.into(), app_id: "kitty".into(), workspace: ws, focus_order: order, ..Default::default() }
    }

    fn state() -> DesktopState {
        DesktopState {
            generation: 1,
            connected: true,
            monitors: vec![MonitorState { name: "eDP-1".into(), active_workspace: 1, focused: true, ..Default::default() }],
            workspaces: vec![WorkspaceState { id: 1, name: "1".into(), monitor: "eDP-1".into(), active: true, user: true, windows: vec!["a".into()], ..Default::default() }],
            windows: vec![win("a", 1, 0)],
            active: Active { monitor: Some("eDP-1".into()), workspace: Some(1), window: Some("a".into()) },
        }
    }

    #[test]
    fn delta_round_trip() {
        let old = state();
        let mut new = old.clone();
        new.generation = 2;
        new.windows.push(win("b", 1, 0));
        new.windows[0].focus_order = 1;
        new.workspaces[0].windows = vec!["b".into(), "a".into()];
        new.active.window = Some("b".into());
        let d = new.delta_from(&old);
        assert_eq!(d.base, 1);
        assert_eq!(d.windows.upsert.len(), 2);
        assert!(d.monitors.is_empty());
        assert!(d.connected.is_none());
        let mut client = old.clone();
        client.apply(&d).unwrap();
        assert_eq!(client.windows.iter().map(|w| w.address.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(client, DesktopState { windows: client.windows.clone(), ..new.clone() });
    }

    #[test]
    fn removals_and_stale_deltas() {
        let old = state();
        let mut new = old.clone();
        new.generation = 2;
        new.windows.clear();
        new.workspaces[0].windows.clear();
        new.active.window = None;
        let d = new.delta_from(&old);
        assert_eq!(d.windows.remove, ["a"]);
        let mut behind = old.clone();
        behind.generation = 0;
        assert_eq!(behind.apply(&d), Err(StaleDelta { have: 0, base: 1 }));
        let mut client = old;
        client.apply(&d).unwrap();
        assert!(client.windows.is_empty());
        assert_eq!(client.generation, 2);
    }

    #[test]
    fn unchanged_is_empty() {
        let old = state();
        let mut new = old.clone();
        new.generation = 2;
        assert!(new.delta_from(&old).is_empty());
    }

    #[test]
    fn fullscreen_on_a_monitor() {
        let mut s = state();
        assert!(!s.fullscreen_on("eDP-1"));
        s.windows[0].fullscreen = Fullscreen::Maximized;
        assert!(!s.fullscreen_on("eDP-1"), "maximized isn't fullscreen");
        s.windows[0].fullscreen = Fullscreen::Fullscreen;
        assert!(s.fullscreen_on("eDP-1"));
        assert!(!s.fullscreen_on("DP-9"));
    }

    #[test]
    fn wire_names() {
        let m = MonitorState { is_virtual: true, ..Default::default() };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["virtual"], true);
        assert_eq!(serde_json::to_value(Fullscreen::Maximized).unwrap(), "maximized");
    }
}
