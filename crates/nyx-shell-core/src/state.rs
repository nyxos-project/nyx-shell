//! From `nyx_desktop`'s model to the wire's normalized desktop.

use nyx_desktop::Desktop;
use nyx_shell_proto::{Active, DesktopState, Fullscreen, MonitorState, WindowState, WorkspaceState};
use std::collections::HashMap;

/// Omarchy's workspace for minimized windows.
const MINIMIZED: &str = "special:minimized";
/// Longest title sent to clients, in characters.
pub const MAX_TITLE: usize = 512;

/// The normalized state of `d` as generation `generation`. `refresh` holds each
/// monitor's refresh rate by connector, read separately.
pub fn normalize(d: &Desktop, generation: u64, refresh: &HashMap<String, f64>) -> DesktopState {
    let monitor_name = |id: i64| d.monitors.iter().find(|m| m.id == id).map(|m| m.name.clone());
    let monitors = d
        .monitors
        .iter()
        .map(|m| {
            let (logical_width, logical_height) = m.logical_size();
            MonitorState {
                name: m.name.clone(),
                stable_id: m.stable_id(),
                description: clean(&m.description, 256),
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                logical_width,
                logical_height,
                scale: m.scale,
                transform: m.transform,
                refresh_rate: refresh.get(&m.name).copied().filter(|r| *r > 0.0),
                focused: m.focused,
                enabled: !m.disabled,
                is_virtual: m.is_virtual(),
                active_workspace: m.active_workspace,
                special_workspace: m.special_workspace,
                reserved: m.reserved,
            }
        })
        .collect();
    let shown = |id: i64| d.monitors.iter().any(|m| !m.disabled && (m.active_workspace == id || m.special_workspace == Some(id)));
    let workspaces = d
        .workspaces
        .iter()
        .map(|w| WorkspaceState {
            id: w.id,
            name: clean(&w.name, 128),
            monitor: w.monitor.clone(),
            active: shown(w.id),
            special: w.is_special(),
            user: w.is_user(),
            has_fullscreen: w.has_fullscreen,
            windows: d.windows.iter().filter(|x| x.workspace == w.id).map(|x| x.address.clone()).collect(),
        })
        .collect();
    let minimized_id = d.workspaces.iter().find(|w| w.name == MINIMIZED).map(|w| w.id);
    let windows = d
        .windows
        .iter()
        .map(|w| WindowState {
            address: w.address.clone(),
            stable_id: w.stable_id.clone(),
            app_id: clean(&w.class, 256),
            initial_app_id: clean(&w.initial_class, 256),
            title: clean(&w.title, MAX_TITLE),
            workspace: w.workspace,
            monitor: monitor_name(w.monitor),
            focused: d.active_window.as_deref() == Some(w.address.as_str()),
            floating: w.floating,
            fullscreen: match w.fullscreen {
                0 => Fullscreen::None,
                1 => Fullscreen::Maximized,
                _ => Fullscreen::Fullscreen,
            },
            hidden: w.hidden,
            minimized: Some(w.workspace) == minimized_id,
            pinned: w.pinned,
            urgent: w.urgent,
            focus_order: w.focus_order,
        })
        .collect();
    let focused = d.focused_monitor();
    DesktopState {
        generation,
        connected: d.connected,
        monitors,
        workspaces,
        windows,
        active: Active {
            monitor: focused.map(|m| m.name.clone()),
            workspace: focused.map(|m| m.active_workspace),
            window: d.active_window.clone(),
        },
    }
}

/// Untrusted text from Hyprland: no control characters, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nyx_desktop::event::Event;

    /// The fixture `nyx-desktop` tests with, built through its public API: two monitors
    /// (a laptop and a headless output), three windows.
    pub fn desktop() -> Desktop {
        let mut d = Desktop { connected: true, ..Default::default() };
        d.monitors = vec![
            nyx_desktop::Monitor {
                id: 0,
                name: "eDP-2".into(),
                description: "Samsung Display Corp. ATNA60DL01-0".into(),
                make: "Samsung Display Corp.".into(),
                model: "ATNA60DL01-0".into(),
                serial: String::new(),
                width: 2560,
                height: 1600,
                x: 0,
                y: 0,
                scale: 1.6,
                transform: 0,
                focused: true,
                disabled: false,
                active_workspace: 1,
                special_workspace: None,
                reserved: [0, 26, 0, 0],
            },
            nyx_desktop::Monitor {
                id: 1,
                name: "HEADLESS-20".into(),
                description: "Headless".into(),
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                width: 1280,
                height: 800,
                x: -20000,
                y: -20000,
                scale: 1.0,
                transform: 0,
                focused: false,
                disabled: false,
                active_workspace: 201,
                special_workspace: None,
                reserved: [0; 4],
            },
        ];
        let ws = |id: i64, name: &str, monitor: &str| nyx_desktop::Workspace {
            id,
            name: name.into(),
            monitor: monitor.into(),
            windows: 0,
            has_fullscreen: false,
            tiled_layout: "dwindle".into(),
        };
        d.workspaces = vec![ws(-99, "special:minimized", "eDP-2"), ws(1, "1", "eDP-2"), ws(2, "2", "eDP-2"), ws(201, "201", "HEADLESS-20")];
        let win = |addr: &str, class: &str, ws: i64, monitor: i64, order: i32| nyx_desktop::Window {
            address: addr.into(),
            stable_id: format!("{order}"),
            class: class.into(),
            initial_class: class.into(),
            title: format!("{class} window"),
            workspace: ws,
            monitor,
            floating: false,
            fullscreen: 0,
            at: [10, 36],
            size: [1580, 954],
            pid: 100 + order,
            focus_order: order,
            pinned: false,
            hidden: false,
            urgent: false,
            grouped: Vec::new(),
        };
        d.windows = vec![win("a1", "firefox", 1, 0, 0), win("b2", "kitty", 2, 0, 1), win("c3", "nautilus", -99, 0, 2)];
        d.active_window = Some("a1".into());
        d
    }

    #[test]
    fn normalizes() {
        let mut refresh = HashMap::new();
        refresh.insert("eDP-2".to_string(), 120.0);
        let s = normalize(&desktop(), 7, &refresh);
        assert_eq!(s.generation, 7);
        let laptop = s.monitor("eDP-2").unwrap();
        assert_eq!((laptop.logical_width, laptop.logical_height), (1600, 1000));
        assert_eq!(laptop.stable_id, "Samsung Display Corp.|ATNA60DL01-0|");
        assert_eq!(laptop.refresh_rate, Some(120.0));
        assert!(s.monitor("HEADLESS-20").unwrap().is_virtual);
        assert_eq!(s.monitor("HEADLESS-20").unwrap().refresh_rate, None);
        assert_eq!(s.active.monitor.as_deref(), Some("eDP-2"));
        assert_eq!(s.active.workspace, Some(1));
        assert_eq!(s.active.window.as_deref(), Some("a1"));
        let one = s.workspace(1).unwrap();
        assert!(one.active && one.user && !one.special);
        assert_eq!(one.windows, ["a1"]);
        assert!(!s.workspace(2).unwrap().active);
        assert!(s.workspace(201).unwrap().active, "shown on the headless output");
        assert!(s.window("c3").unwrap().minimized);
        assert!(s.window("a1").unwrap().focused);
        assert_eq!(s.window("b2").unwrap().monitor.as_deref(), Some("eDP-2"));
    }

    #[test]
    fn events_become_small_deltas() {
        let mut d = desktop();
        let old = normalize(&d, 1, &HashMap::new());
        d.apply(&Event::parse("activewindowv2>>b2"));
        let new = normalize(&d, 2, &HashMap::new());
        let delta = new.delta_from(&old);
        assert!(delta.monitors.is_empty() && delta.workspaces.is_empty());
        assert_eq!(delta.windows.upsert.len(), 2, "the old and the new focused window");
        assert_eq!(delta.active.unwrap().window.as_deref(), Some("b2"));
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean("a\u{1b}[31mb\nc", 512), "a[31mbc");
        assert_eq!(clean(&"x".repeat(600), MAX_TITLE).chars().count(), 512);
        let mut d = desktop();
        d.windows[0].title = "evil\u{0}title".into();
        assert_eq!(normalize(&d, 1, &HashMap::new()).windows[0].title, "eviltitle");
    }
}
