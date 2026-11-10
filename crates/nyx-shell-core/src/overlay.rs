//! The overlay registry: policies, who owns what, and what should be on screen.
//!
//! Shell Core decides whether each overlay is visible; surfaces follow. Showing one
//! hides the rest of its group, locking hides everything, fullscreen windows hide or
//! block overlays as their policy says, and an owner that goes away takes its overlays
//! with it. Pinned overlays stay until unpinned, outside their group, laid out down the
//! right edge of their monitor.

use crate::{ClientId, Owner};
use nyx_shell_proto::config::OverlayOverride;
use nyx_shell_proto::overlay::{HiddenReason, PANELS, PIN_MAX, PIN_MIN};
use nyx_shell_proto::{Category, DesktopState, ErrorBody, ErrorCode, FullscreenRule, Keyboard, MonitorState, OverlayPolicy, OverlayState, Pin, Placement};
use std::collections::BTreeMap;

/// The GTK panels that speak `Panel1`: overlay id, bus name.
pub const PANEL1: &[(&str, &str)] = &[
    ("control-center", "io.github.design_nexus.nyx.ControlCenter"),
    ("notification-center", "io.github.design_nexus.nyx.NotificationCenter"),
    ("switcher", "io.github.design_nexus.nyx.Switcher"),
];

/// The lock screen's entry: listed, never driven.
pub const LOCK_SCREEN: &str = "lock-screen";

#[derive(Debug, Clone)]
struct Entry {
    /// As registered.
    base: OverlayPolicy,
    /// After `[overlays.ID]` overrides.
    policy: OverlayPolicy,
    owner: Owner,
    requested: bool,
    visible: bool,
    monitor: Option<String>,
    reason: Option<HiddenReason>,
    revision: u64,
    /// While pinned: the box, and when it was pinned (the layout's order).
    pin: Option<(Pin, u64)>,
}

impl Entry {
    fn state(&self) -> OverlayState {
        OverlayState {
            id: self.policy.id.clone(),
            owner: self.owner.label(),
            category: self.policy.category,
            group: self.policy.effective_group().map(str::to_string),
            keyboard: self.policy.keyboard,
            fullscreen: self.policy.fullscreen,
            placement: self.policy.placement.clone(),
            requested: self.requested,
            visible: self.visible,
            monitor: self.monitor.clone(),
            reason: self.reason,
            revision: self.revision,
            pin: self.pin.map(|(p, _)| p),
        }
    }

    fn is_panel1(&self) -> bool {
        matches!(self.owner, Owner::Panel1 { .. })
    }
}

/// What a change did, for events.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Changed(OverlayState),
    Removed(String),
}

#[derive(Debug, Default)]
pub struct Overlays {
    entries: BTreeMap<String, Entry>,
    overrides: BTreeMap<String, OverlayOverride>,
    pins: u64,
}

/// Pinned overlays keep this far from the usable area's edge, and from each other.
pub const PIN_MARGIN: i32 = 12;
pub const PIN_GAP: i32 = 10;

fn err(code: ErrorCode, msg: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code, msg)
}

fn apply_override(base: &OverlayPolicy, o: Option<&OverlayOverride>) -> OverlayPolicy {
    let mut p = base.clone();
    if let Some(o) = o {
        if let Some(g) = &o.group {
            p.group = Some(g.clone());
        }
        if let Some(f) = o.fullscreen {
            p.fullscreen = f;
        }
        if let Some(pl) = &o.placement {
            p.placement = pl.clone();
        }
        if let Some(r) = o.restore_focus {
            p.restore_focus = Some(r);
        }
    }
    p
}

impl Overlays {
    /// The registry with the built-in entries: the `Panel1` panels and the lock screen.
    pub fn new() -> Overlays {
        let mut o = Overlays::default();
        for (id, bus) in PANEL1 {
            let policy = OverlayPolicy {
                group: Some(PANELS.into()),
                keyboard: if *id == "switcher" { Keyboard::Exclusive } else { Keyboard::OnDemand },
                fullscreen: FullscreenRule::Show,
                restore_focus: Some(false),
                ..OverlayPolicy::new(id, Category::Transient)
            };
            o.insert(policy, Owner::Panel1 { bus: (*bus).into() });
        }
        let lock = OverlayPolicy { fullscreen: FullscreenRule::Show, placement: Placement::All, ..OverlayPolicy::new(LOCK_SCREEN, Category::SecureExternal) };
        o.insert(lock, Owner::Builtin);
        o
    }

    fn insert(&mut self, base: OverlayPolicy, owner: Owner) -> &Entry {
        let policy = apply_override(&base, self.overrides.get(&base.id));
        let id = base.id.clone();
        self.entries.insert(id.clone(), Entry { base, policy, owner, requested: false, visible: false, monitor: None, reason: None, revision: 0, pin: None });
        &self.entries[&id]
    }

    pub fn get(&self, id: &str) -> Option<OverlayState> {
        self.entries.get(id).map(Entry::state)
    }

    pub fn all(&self) -> Vec<OverlayState> {
        self.entries.values().map(Entry::state).collect()
    }

    pub fn policy(&self, id: &str) -> Option<&OverlayPolicy> {
        self.entries.get(id).map(|e| &e.policy)
    }

    /// Ids of overlays registered by clients.
    pub fn client_overlays(&self) -> impl Iterator<Item = &str> {
        self.entries.values().filter(|e| matches!(e.owner, Owner::Client { .. })).map(|e| e.policy.id.as_str())
    }

    pub fn register(&mut self, policy: OverlayPolicy, owner: Owner) -> Result<OverlayState, ErrorBody> {
        policy.validate().map_err(|e| err(ErrorCode::Invalid, e))?;
        if policy.category == Category::SecureExternal {
            return Err(err(ErrorCode::Invalid, "secure-external overlays live outside Shell Core and can't be registered"));
        }
        if let Some(existing) = self.entries.get(&policy.id) {
            let same_client = existing.owner.client().is_some() && existing.owner.client() == owner.client();
            if !same_client && !matches!(existing.owner, Owner::Gone { .. }) {
                return Err(err(ErrorCode::Conflict, format!("{} is owned by {}", policy.id, existing.owner.label())));
            }
        }
        Ok(self.insert(policy, owner).state())
    }

    pub fn unregister(&mut self, id: &str, client: ClientId) -> Result<(), ErrorBody> {
        let e = self.entries.get(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no overlay {id}")))?;
        if e.owner.client() != Some(client) {
            return Err(err(ErrorCode::Forbidden, format!("{id} isn't yours")));
        }
        self.entries.remove(id);
        Ok(())
    }

    /// Ask for `id` to show. `monitor` is where to put it when its placement needs a
    /// choice made outside (the pointer's monitor), or a client's explicit wish.
    /// Returns every overlay that changed, the shown one first.
    pub fn show(&mut self, id: &str, monitor: Option<String>, desktop: &DesktopState, locked: bool) -> Result<Vec<Change>, ErrorBody> {
        let e = self.entries.get(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no overlay {id}")))?;
        if e.policy.category == Category::SecureExternal {
            return Err(err(ErrorCode::Invalid, "the lock screen isn't driven over IPC"));
        }
        if e.is_panel1() {
            return Err(err(ErrorCode::Invalid, format!("{id} is a GTK panel: use its action")));
        }
        if locked {
            return Err(err(ErrorCode::Locked, "the session is locked"));
        }
        let target = place(&e.policy.placement, monitor, desktop);
        if e.policy.fullscreen == FullscreenRule::Block && fullscreen_at(target.as_deref(), desktop) {
            return Err(err(ErrorCode::Invalid, format!("{id} doesn't open over a fullscreen window")));
        }
        let group = e.policy.effective_group().map(str::to_string);
        let passive = e.policy.category == Category::Passive;
        let mut changes = Vec::new();
        {
            let e = self.entries.get_mut(id).expect("checked above");
            e.requested = true;
            e.monitor = target;
            if let Some(c) = refresh(e, desktop, locked, true) {
                changes.push(c);
            }
        }
        if let Some(g) = group.filter(|_| !passive) {
            let others: Vec<String> = self
                .entries
                .values()
                .filter(|o| o.policy.id != id && o.requested && o.pin.is_none() && o.policy.effective_group() == Some(g.as_str()))
                .map(|o| o.policy.id.clone())
                .collect();
            for other in others {
                changes.extend(self.set_requested(&other, false, desktop, locked));
            }
        }
        Ok(changes)
    }

    pub fn hide(&mut self, id: &str, desktop: &DesktopState, locked: bool) -> Result<Vec<Change>, ErrorBody> {
        let e = self.entries.get(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no overlay {id}")))?;
        if e.policy.category == Category::SecureExternal {
            return Err(err(ErrorCode::Invalid, "the lock screen isn't driven over IPC"));
        }
        if e.is_panel1() {
            return Err(err(ErrorCode::Invalid, format!("{id} is a GTK panel: use its action")));
        }
        Ok(self.set_requested(id, false, desktop, locked))
    }

    pub fn toggle(&mut self, id: &str, desktop: &DesktopState, locked: bool) -> Result<Vec<Change>, ErrorBody> {
        let requested = self.entries.get(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no overlay {id}")))?.requested;
        if requested { self.hide(id, desktop, locked) } else { self.show(id, None, desktop, locked) }
    }

    /// Pin `id` at `size` on `monitor` (or where its placement says), or unpin it.
    /// Only persistent overlays are pinned. Returns every overlay that changed: the
    /// pinned one, and others on its monitor that moved.
    pub fn pin(&mut self, id: &str, pinned: bool, size: Option<[u32; 2]>, monitor: Option<String>, desktop: &DesktopState, locked: bool) -> Result<Vec<Change>, ErrorBody> {
        let e = self.entries.get(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no overlay {id}")))?;
        if e.policy.category != Category::Persistent {
            return Err(err(ErrorCode::Invalid, format!("only persistent overlays are pinned; {id} is {}", nyx_shell_proto::overlay::category_name(e.policy.category))));
        }
        let was = e.monitor.clone().filter(|_| e.pin.is_some());
        if !pinned {
            let e = self.entries.get_mut(id).expect("checked above");
            e.pin = None;
            e.requested = false;
            let mut changes: Vec<Change> = refresh(e, desktop, locked, true).into_iter().collect();
            if let Some(m) = was {
                changes.extend(self.lay_out(&m, desktop, locked, true).unwrap_or_default());
            }
            return Ok(changes);
        }
        if locked {
            return Err(err(ErrorCode::Locked, "the session is locked"));
        }
        let size = size.ok_or_else(|| err(ErrorCode::Invalid, "a pinned overlay needs its size"))?;
        if size[0] < PIN_MIN[0] || size[1] < PIN_MIN[1] || size[0] > PIN_MAX[0] || size[1] > PIN_MAX[1] {
            return Err(err(ErrorCode::Invalid, format!("a pinned overlay is {}×{} to {}×{}", PIN_MIN[0], PIN_MIN[1], PIN_MAX[0], PIN_MAX[1])));
        }
        let target = place(&e.policy.placement, monitor, desktop).ok_or_else(|| err(ErrorCode::Invalid, format!("{id} pins on one monitor, not all")))?;
        let order = match e.pin {
            Some((_, order)) if was.as_deref() == Some(target.as_str()) => order,
            _ => {
                self.pins += 1;
                self.pins
            }
        };
        let saved = self.entries[id].clone();
        {
            let e = self.entries.get_mut(id).expect("checked above");
            e.pin = Some((Pin { size, at: [0, 0] }, order));
            e.requested = true;
            e.monitor = Some(target.clone());
        }
        let Some(mut changes) = self.lay_out(&target, desktop, locked, false) else {
            self.entries.insert(id.to_string(), saved);
            return Err(err(ErrorCode::Invalid, format!("there's no room for another pinned panel on {target}")));
        };
        if !changes.iter().any(|c| matches!(c, Change::Changed(s) if s.id == id)) {
            let e = self.entries.get_mut(id).expect("checked above");
            changes.extend(refresh(e, desktop, locked, true));
        }
        if let Some(m) = was.filter(|m| *m != target) {
            changes.extend(self.lay_out(&m, desktop, locked, true).unwrap_or_default());
        }
        Ok(changes)
    }

    /// Place the pinned overlays on `monitor` again: top to bottom down the right edge
    /// of its usable area, in the order they were pinned, a new column to the left when
    /// one is full. `None` when they don't fit, unless `crowd` (a monitor that shrank),
    /// which lets the last ones overlap. Returns the ones that moved.
    fn lay_out(&mut self, monitor: &str, desktop: &DesktopState, locked: bool, crowd: bool) -> Option<Vec<Change>> {
        let Some(m) = desktop.monitor(monitor) else { return Some(Vec::new()) };
        let mut pinned: Vec<(u64, String, [u32; 2])> = self
            .entries
            .values()
            .filter(|e| e.requested && e.monitor.as_deref() == Some(monitor))
            .filter_map(|e| e.pin.map(|(p, order)| (order, e.policy.id.clone(), p.size)))
            .collect();
        pinned.sort();
        let spots = spots(m, pinned.iter().map(|p| p.2), crowd)?;
        let mut changes = Vec::new();
        for ((_, id, _), at) in pinned.iter().zip(spots) {
            let e = self.entries.get_mut(id).expect("listed above");
            let moved = e.pin.is_some_and(|(p, _)| p.at != at);
            if let Some((p, _)) = &mut e.pin {
                p.at = at;
            }
            if let Some(c) = refresh(e, desktop, locked, moved) {
                changes.push(c);
            }
        }
        Some(changes)
    }

    fn set_requested(&mut self, id: &str, on: bool, desktop: &DesktopState, locked: bool) -> Vec<Change> {
        let Some(e) = self.entries.get_mut(id) else { return Vec::new() };
        let was = e.requested;
        e.requested = on;
        refresh(e, desktop, locked, was != on).into_iter().collect()
    }

    /// A GTK panel announced itself (`Panel1.Opened`). The others in its group close.
    pub fn panel_opened(&mut self, id: &str, desktop: &DesktopState, locked: bool) -> Vec<Change> {
        let Some(e) = self.entries.get(id).filter(|e| e.is_panel1()) else { return Vec::new() };
        let group = e.policy.effective_group().map(str::to_string);
        let mut changes = self.set_requested(id, true, desktop, locked);
        if let Some(g) = group {
            let others: Vec<String> = self
                .entries
                .values()
                .filter(|o| o.policy.id != id && o.requested && o.policy.effective_group() == Some(g.as_str()))
                .map(|o| o.policy.id.clone())
                .collect();
            for other in others {
                changes.extend(self.set_requested(&other, false, desktop, locked));
            }
        }
        changes
    }

    /// A GTK panel's layer surface mapped or unmapped, or the panel went away.
    pub fn panel_state(&mut self, id: &str, open: bool, desktop: &DesktopState, locked: bool) -> Vec<Change> {
        if !self.entries.get(id).is_some_and(Entry::is_panel1) {
            return Vec::new();
        }
        self.set_requested(id, open, desktop, locked)
    }

    /// Ids of `Panel1` panels currently thought open, in `group`.
    pub fn open_panels(&self, group: &str) -> Vec<String> {
        self.entries
            .values()
            .filter(|e| e.is_panel1() && e.requested && e.policy.effective_group() == Some(group))
            .map(|e| e.policy.id.clone())
            .collect()
    }

    /// The lock screen's entry follows logind.
    pub fn set_locked_entry(&mut self, locked: bool, desktop: &DesktopState) -> Vec<Change> {
        self.set_requested(LOCK_SCREEN, locked, desktop, locked)
    }

    /// Everything after the desktop or the lock changed: fullscreen rules, monitors
    /// that went away.
    pub fn recompute(&mut self, desktop: &DesktopState, locked: bool) -> Vec<Change> {
        let mut changes = Vec::new();
        for e in self.entries.values_mut() {
            let mut moved = false;
            if let Some(m) = &e.monitor
                && desktop.connected
                && desktop.monitor(m).is_none()
            {
                e.monitor = desktop.active.monitor.clone();
                moved = true;
            }
            if let Some(c) = refresh(e, desktop, locked, moved) {
                changes.push(c);
            }
        }
        if desktop.connected {
            let monitors: std::collections::BTreeSet<String> =
                self.entries.values().filter(|e| e.pin.is_some() && e.requested).filter_map(|e| e.monitor.clone()).collect();
            for m in monitors {
                for c in self.lay_out(&m, desktop, locked, true).unwrap_or_default() {
                    let Change::Changed(s) = &c else { continue };
                    changes.retain(|o| !matches!(o, Change::Changed(x) if x.id == s.id));
                    changes.push(c);
                }
            }
        }
        changes
    }

    /// A client went away: its overlays go too.
    pub fn client_gone(&mut self, client: ClientId) -> Vec<Change> {
        let gone: Vec<String> = self.entries.values().filter(|e| e.owner.client() == Some(client)).map(|e| e.policy.id.clone()).collect();
        gone.into_iter()
            .map(|id| {
                self.entries.remove(&id);
                Change::Removed(id)
            })
            .collect()
    }

    /// New `[overlays.ID]` overrides from the settings.
    pub fn set_overrides(&mut self, overrides: BTreeMap<String, OverlayOverride>, desktop: &DesktopState, locked: bool) -> Vec<Change> {
        self.overrides = overrides;
        let mut changes = Vec::new();
        for e in self.entries.values_mut() {
            let policy = apply_override(&e.base, self.overrides.get(&e.base.id));
            if policy.validate().is_err() || policy == e.policy {
                continue;
            }
            e.policy = policy;
            if let Some(c) = refresh(e, desktop, locked, true) {
                changes.push(c);
            }
        }
        changes
    }

    /// Whether any visible overlay restores focus when it closes.
    pub fn any_restoring_visible(&self) -> bool {
        self.entries.values().any(|e| e.visible && !e.is_panel1() && e.policy.effective_restore_focus())
    }
}

/// Work out `visible` and `reason` again; returns the state when anything changed
/// (`force` also counts as a change, for request and placement changes).
fn refresh(e: &mut Entry, desktop: &DesktopState, locked: bool, force: bool) -> Option<Change> {
    let (visible, reason) = if e.is_panel1() || e.policy.category == Category::SecureExternal {
        (e.requested, None)
    } else if !e.requested {
        (false, None)
    } else if locked {
        (false, Some(HiddenReason::Locked))
    } else if e.policy.fullscreen == FullscreenRule::Hide && fullscreen_at(e.monitor.as_deref(), desktop) {
        (false, Some(HiddenReason::Fullscreen))
    } else {
        (true, None)
    };
    if !force && visible == e.visible && reason == e.reason {
        return None;
    }
    e.visible = visible;
    e.reason = reason;
    e.revision += 1;
    Some(Change::Changed(e.state()))
}

/// The monitor an overlay opens on.
fn place(placement: &Placement, wish: Option<String>, desktop: &DesktopState) -> Option<String> {
    let active = desktop.active.monitor.clone().or_else(|| desktop.monitors.first().map(|m| m.name.clone()));
    let exists = |m: &String| desktop.monitor(m).is_some();
    match placement {
        Placement::All => None,
        Placement::Monitor(m) => Some(m.clone()).filter(exists).or_else(|| wish.filter(exists)).or(active),
        Placement::Active | Placement::Pointer => wish.filter(exists).or(active),
    }
}

/// Top-left corners for boxes of `sizes`, in order, inside `m`'s usable area.
fn spots(m: &MonitorState, sizes: impl Iterator<Item = [u32; 2]>, crowd: bool) -> Option<Vec<[i32; 2]>> {
    let [l, t, r, b] = m.reserved;
    let (left, top) = (l + PIN_MARGIN, t + PIN_MARGIN);
    let (right, bottom) = (m.logical_width - r - PIN_MARGIN, m.logical_height - b - PIN_MARGIN);
    let (mut edge, mut y, mut column) = (right, top, 0);
    let mut out = Vec::new();
    for [w, h] in sizes {
        let (w, h) = (w as i32, h as i32);
        if y > top && y + h > bottom {
            edge -= column + PIN_GAP;
            y = top;
            column = 0;
        }
        let x = edge - w;
        if !crowd && (x < left || y + h > bottom) {
            return None;
        }
        out.push([x.max(left), y]);
        y += h + PIN_GAP;
        column = column.max(w);
    }
    Some(out)
}

/// A fullscreen window on `monitor`, or on any monitor for overlays on all of them.
fn fullscreen_at(monitor: Option<&str>, desktop: &DesktopState) -> bool {
    match monitor {
        Some(m) => desktop.fullscreen_on(m),
        None => desktop.monitors.iter().any(|m| desktop.fullscreen_on(&m.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nyx_shell_proto::{Fullscreen, MonitorState, WindowState};

    fn desktop() -> DesktopState {
        DesktopState {
            generation: 1,
            connected: true,
            monitors: vec![
                MonitorState { name: "eDP-1".into(), active_workspace: 1, focused: true, ..Default::default() },
                MonitorState { name: "DP-2".into(), active_workspace: 2, ..Default::default() },
            ],
            windows: vec![WindowState { address: "a".into(), workspace: 2, ..Default::default() }],
            active: nyx_shell_proto::Active { monitor: Some("eDP-1".into()), workspace: Some(1), window: None },
            ..Default::default()
        }
    }

    fn client(id: ClientId) -> Owner {
        Owner::Client { id, name: format!("c{id}") }
    }

    fn transient(id: &str) -> OverlayPolicy {
        OverlayPolicy { keyboard: Keyboard::Exclusive, ..OverlayPolicy::new(id, Category::Transient) }
    }

    fn visible(o: &Overlays, id: &str) -> bool {
        o.get(id).unwrap().visible
    }

    #[test]
    fn showing_one_hides_its_group() {
        let d = desktop();
        let mut o = Overlays::new();
        o.register(transient("a"), client(1)).unwrap();
        o.register(transient("b"), client(1)).unwrap();
        o.register(OverlayPolicy::new("osd", Category::Passive), client(2)).unwrap();
        o.show("a", None, &d, false).unwrap();
        o.show("osd", None, &d, false).unwrap();
        assert!(visible(&o, "a") && visible(&o, "osd"));
        let changes = o.show("b", None, &d, false).unwrap();
        assert!(visible(&o, "b"));
        assert!(!visible(&o, "a"), "same group");
        assert!(visible(&o, "osd"), "passive overlays are in no group");
        assert_eq!(changes.len(), 2);
        assert_eq!(o.get("b").unwrap().monitor.as_deref(), Some("eDP-1"));
    }

    #[test]
    fn rapid_toggles_end_where_they_should() {
        let d = desktop();
        let mut o = Overlays::new();
        o.register(transient("a"), client(1)).unwrap();
        let mut last = 0;
        for i in 0..21 {
            for c in o.toggle("a", &d, false).unwrap() {
                if let Change::Changed(s) = c {
                    assert!(s.revision > last, "revisions only grow");
                    last = s.revision;
                }
            }
            assert_eq!(visible(&o, "a"), i % 2 == 0);
        }
        assert!(visible(&o, "a"));
    }

    #[test]
    fn locked_hides_everything_and_refuses() {
        let d = desktop();
        let mut o = Overlays::new();
        o.register(transient("a"), client(1)).unwrap();
        o.show("a", None, &d, false).unwrap();
        o.set_locked_entry(true, &d);
        let changes = o.recompute(&d, true);
        assert!(changes.iter().any(|c| matches!(c, Change::Changed(s) if s.id == "a" && !s.visible && s.reason == Some(HiddenReason::Locked))));
        assert!(visible(&o, LOCK_SCREEN));
        assert_eq!(o.show("a", None, &d, true).unwrap_err().code, ErrorCode::Locked);
        o.set_locked_entry(false, &d);
        o.recompute(&d, false);
        assert!(visible(&o, "a"), "the request survives the lock");
        assert_eq!(o.show(LOCK_SCREEN, None, &d, false).unwrap_err().code, ErrorCode::Invalid);
    }

    #[test]
    fn fullscreen_rules() {
        let mut d = desktop();
        d.windows[0].fullscreen = Fullscreen::Fullscreen;
        let mut o = Overlays::new();
        o.register(OverlayPolicy { fullscreen: FullscreenRule::Hide, ..transient("hide") }, client(1)).unwrap();
        o.register(OverlayPolicy { fullscreen: FullscreenRule::Block, group: Some("x".into()), ..transient("block") }, client(1)).unwrap();
        o.register(OverlayPolicy { fullscreen: FullscreenRule::Show, group: Some("y".into()), ..transient("show") }, client(1)).unwrap();
        o.show("hide", Some("DP-2".into()), &d, false).unwrap();
        assert!(!visible(&o, "hide"));
        assert_eq!(o.get("hide").unwrap().reason, Some(HiddenReason::Fullscreen));
        assert_eq!(o.show("block", Some("DP-2".into()), &d, false).unwrap_err().code, ErrorCode::Invalid);
        o.show("block", None, &d, false).unwrap();
        assert!(visible(&o, "block"), "no fullscreen window on eDP-1");
        o.show("show", Some("DP-2".into()), &d, false).unwrap();
        assert!(visible(&o, "show"));
        d.windows[0].fullscreen = Fullscreen::None;
        o.recompute(&d, false);
        assert!(visible(&o, "hide"), "back once the fullscreen window is gone");
    }

    #[test]
    fn owners_and_conflicts() {
        let d = desktop();
        let mut o = Overlays::new();
        o.register(transient("a"), client(1)).unwrap();
        assert_eq!(o.register(transient("a"), client(2)).unwrap_err().code, ErrorCode::Conflict);
        o.register(transient("a"), client(1)).unwrap();
        assert_eq!(o.register(transient("control-center"), client(1)).unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(o.unregister("a", 2).unwrap_err().code, ErrorCode::Forbidden);
        o.show("a", None, &d, false).unwrap();
        assert_eq!(o.client_gone(1), vec![Change::Removed("a".into())]);
        assert!(o.get("a").is_none());
        let mut secure = OverlayPolicy::new("fake-lock", Category::SecureExternal);
        secure.keyboard = Keyboard::None;
        assert_eq!(o.register(secure, client(1)).unwrap_err().code, ErrorCode::Invalid);
    }

    #[test]
    fn panels_and_client_overlays_share_a_group() {
        let d = desktop();
        let mut o = Overlays::new();
        o.register(transient("inspector"), client(1)).unwrap();
        o.show("inspector", None, &d, false).unwrap();
        o.panel_opened("control-center", &d, false);
        assert!(visible(&o, "control-center"));
        assert!(!visible(&o, "inspector"), "a GTK panel opening closes it");
        o.show("inspector", None, &d, false).unwrap();
        assert!(!visible(&o, "control-center"), "and the other way round");
        assert!(o.open_panels(PANELS).is_empty());
        assert_eq!(o.show("control-center", None, &d, false).unwrap_err().code, ErrorCode::Invalid);
    }

    #[test]
    fn monitors_that_go_away() {
        let mut d = desktop();
        let mut o = Overlays::new();
        o.register(transient("a"), client(1)).unwrap();
        o.show("a", Some("DP-2".into()), &d, false).unwrap();
        assert_eq!(o.get("a").unwrap().monitor.as_deref(), Some("DP-2"));
        d.monitors.retain(|m| m.name != "DP-2");
        let changes = o.recompute(&d, false);
        assert_eq!(changes.len(), 1);
        assert_eq!(o.get("a").unwrap().monitor.as_deref(), Some("eDP-1"));
        o.register(OverlayPolicy { placement: Placement::All, group: Some("z".into()), ..transient("everywhere") }, client(1)).unwrap();
        o.show("everywhere", None, &d, false).unwrap();
        assert_eq!(o.get("everywhere").unwrap().monitor, None);
    }

    fn pinnable(id: &str) -> OverlayPolicy {
        OverlayPolicy { keyboard: Keyboard::OnDemand, group: Some("pins".into()), ..OverlayPolicy::new(id, Category::Persistent) }
    }

    fn at(o: &Overlays, id: &str) -> [i32; 2] {
        o.get(id).unwrap().pin.unwrap().at
    }

    #[test]
    fn pinned_overlays_stack_down_the_right_edge() {
        let mut d = desktop();
        d.monitors[0].logical_width = 1280;
        d.monitors[0].logical_height = 800;
        d.monitors[0].reserved = [0, 30, 0, 0];
        let mut o = Overlays::new();
        for id in ["a", "b", "c"] {
            o.register(pinnable(id), client(1)).unwrap();
        }
        o.pin("a", true, Some([400, 300]), None, &d, false).unwrap();
        o.pin("b", true, Some([300, 300]), None, &d, false).unwrap();
        assert_eq!(at(&o, "a"), [1280 - 12 - 400, 30 + 12]);
        assert_eq!(at(&o, "b"), [1280 - 12 - 300, 30 + 12 + 300 + 10]);
        // A third doesn't fit under them: a column to the left of the widest.
        o.pin("c", true, Some([300, 300]), None, &d, false).unwrap();
        assert_eq!(at(&o, "c"), [1280 - 12 - 400 - 10 - 300, 42]);
        assert!(visible(&o, "a") && visible(&o, "b") && visible(&o, "c"));
        assert_eq!(o.get("a").unwrap().monitor.as_deref(), Some("eDP-1"));

        // Unpinning one moves the later ones up.
        let changes = o.pin("a", false, None, None, &d, false).unwrap();
        assert!(!visible(&o, "a") && o.get("a").unwrap().pin.is_none());
        assert_eq!(at(&o, "b"), [1280 - 12 - 300, 42]);
        assert_eq!(at(&o, "c"), [1280 - 12 - 300, 42 + 310]);
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn pins_are_checked() {
        let mut d = desktop();
        d.monitors[0].logical_width = 800;
        d.monitors[0].logical_height = 600;
        let mut o = Overlays::new();
        o.register(transient("t"), client(1)).unwrap();
        assert_eq!(o.pin("t", true, Some([300, 200]), None, &d, false).unwrap_err().code, ErrorCode::Invalid, "persistent only");
        o.register(pinnable("p"), client(1)).unwrap();
        assert_eq!(o.pin("p", true, None, None, &d, false).unwrap_err().code, ErrorCode::Invalid, "needs a size");
        assert_eq!(o.pin("p", true, Some([10, 10]), None, &d, false).unwrap_err().code, ErrorCode::Invalid, "too small");
        assert_eq!(o.pin("p", true, Some([300, 200]), None, &d, true).unwrap_err().code, ErrorCode::Locked);
        o.pin("p", true, Some([700, 500]), None, &d, false).unwrap();
        o.register(pinnable("q"), client(1)).unwrap();
        let e = o.pin("q", true, Some([700, 500]), None, &d, false).unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid, "no room left");
        assert!(o.get("q").unwrap().pin.is_none() && !visible(&o, "q"), "a refused pin changes nothing");
        // On the other monitor, named.
        d.monitors[1].logical_width = 800;
        d.monitors[1].logical_height = 600;
        o.pin("q", true, Some([700, 500]), Some("DP-2".into()), &d, false).unwrap();
        assert_eq!(o.get("q").unwrap().monitor.as_deref(), Some("DP-2"));
    }

    #[test]
    fn pinned_overlays_stay_when_their_group_opens() {
        let mut d = desktop();
        d.monitors[0].logical_width = 1280;
        d.monitors[0].logical_height = 800;
        let mut o = Overlays::new();
        o.register(pinnable("pinned"), client(1)).unwrap();
        o.register(pinnable("other"), client(1)).unwrap();
        o.pin("pinned", true, Some([300, 200]), None, &d, false).unwrap();
        o.show("other", None, &d, false).unwrap();
        assert!(visible(&o, "pinned") && visible(&o, "other"));
        o.set_locked_entry(true, &d);
        o.recompute(&d, true);
        assert!(!visible(&o, "pinned"), "hidden while locked");
        o.set_locked_entry(false, &d);
        o.recompute(&d, false);
        assert!(visible(&o, "pinned"), "and back after");
    }

    #[test]
    fn pinned_overlays_follow_their_monitor() {
        let mut d = desktop();
        for m in &mut d.monitors {
            m.logical_width = 1280;
            m.logical_height = 800;
        }
        let mut o = Overlays::new();
        o.register(pinnable("p"), client(1)).unwrap();
        o.pin("p", true, Some([300, 200]), Some("DP-2".into()), &d, false).unwrap();
        d.monitors[1].reserved = [0, 40, 0, 0];
        let changes = o.recompute(&d, false);
        assert_eq!(changes.len(), 1, "moved below the new bar");
        assert_eq!(at(&o, "p"), [1280 - 12 - 300, 52]);
        d.monitors.retain(|m| m.name != "DP-2");
        o.recompute(&d, false);
        assert_eq!(o.get("p").unwrap().monitor.as_deref(), Some("eDP-1"));
        assert_eq!(at(&o, "p"), [1280 - 12 - 300, 12]);
    }

    #[test]
    fn overrides_from_settings() {
        let mut d = desktop();
        d.windows[0].fullscreen = Fullscreen::Fullscreen;
        let mut o = Overlays::new();
        o.register(OverlayPolicy { fullscreen: FullscreenRule::Show, ..transient("a") }, client(1)).unwrap();
        o.show("a", Some("DP-2".into()), &d, false).unwrap();
        assert!(visible(&o, "a"));
        let mut ov = BTreeMap::new();
        ov.insert("a".to_string(), OverlayOverride { fullscreen: Some(FullscreenRule::Hide), ..Default::default() });
        o.set_overrides(ov, &d, false);
        assert!(!visible(&o, "a"));
        assert_eq!(o.policy("a").unwrap().fullscreen, FullscreenRule::Hide);
    }
}
