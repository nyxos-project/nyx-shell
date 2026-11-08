//! Shell components: manifests, states, dependencies, health.
//!
//! systemd restarts processes; the registry only knows and reports. A component whose
//! required dependency isn't up is reported `degraded`; one whose connection drops
//! without saying it stopped is `failed`.

use crate::{ClientId, Owner};
use nyx_shell_proto::{ComponentKind, ComponentManifest, ComponentState, ComponentStatus, ErrorBody, ErrorCode, HealthReport, RestartPolicy};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone)]
struct Entry {
    manifest: ComponentManifest,
    state: ComponentState,
    detail: Option<String>,
    owner: Owner,
    since: Instant,
}

#[derive(Debug, Default)]
pub struct Components {
    entries: BTreeMap<String, Entry>,
}

/// Built-in components: Shell Core itself, Hyprland, D-Bus, and the GTK panels.
pub const CORE: &str = "core";
pub const HYPRLAND: &str = "hyprland";
pub const DBUS: &str = "dbus";

fn err(code: ErrorCode, msg: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code, msg)
}

impl Components {
    pub fn new(now: Instant) -> Components {
        let mut c = Components::default();
        let builtin = |id: &str, kind: ComponentKind, process: &str, requires: &[&str]| ComponentManifest {
            id: id.into(),
            kind,
            process: process.into(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            restart: RestartPolicy::OnFailure,
            ..Default::default()
        };
        c.put(builtin(CORE, ComponentKind::Runtime, "nyx-shell-core", &[]), ComponentState::Ready, Owner::Builtin, now);
        c.put(builtin(HYPRLAND, ComponentKind::Bridge, "Hyprland", &[]), ComponentState::Starting, Owner::Builtin, now);
        c.put(builtin(DBUS, ComponentKind::Bridge, "nyx-shell-core", &[]), ComponentState::Starting, Owner::Builtin, now);
        for (id, bus) in crate::overlay::PANEL1 {
            let mut m = builtin(id, ComponentKind::Service, bus, &[DBUS]);
            m.overlays = vec![(*id).into()];
            c.put(m, ComponentState::Registered, Owner::Panel1 { bus: (*bus).into() }, now);
        }
        c
    }

    fn put(&mut self, manifest: ComponentManifest, state: ComponentState, owner: Owner, now: Instant) {
        self.entries.insert(manifest.id.clone(), Entry { manifest, state, detail: None, owner, since: now });
    }

    pub fn register(&mut self, manifest: ComponentManifest, owner: Owner, now: Instant) -> Result<ComponentStatus, ErrorBody> {
        manifest.validate().map_err(|e| err(ErrorCode::Invalid, e))?;
        if let Some(e) = self.entries.get(&manifest.id) {
            let replaceable = matches!(e.owner, Owner::Gone { .. }) || (e.owner.client().is_some() && e.owner.client() == owner.client());
            if !replaceable {
                return Err(err(ErrorCode::Conflict, format!("{} is already registered by {}", manifest.id, e.owner.label())));
            }
        }
        let id = manifest.id.clone();
        self.put(manifest, ComponentState::Starting, owner, now);
        Ok(self.status(&id, now).expect("just inserted"))
    }

    /// A component reported itself.
    pub fn report(&mut self, id: &str, client: ClientId, report: HealthReport, detail: Option<String>, now: Instant) -> Result<ComponentStatus, ErrorBody> {
        let e = self.entries.get_mut(id).ok_or_else(|| err(ErrorCode::NotFound, format!("no component {id}")))?;
        if e.owner.client() != Some(client) {
            return Err(err(ErrorCode::Forbidden, format!("{id} isn't yours")));
        }
        let next = ComponentState::from(report);
        if !e.state.can_become(next) {
            return Err(err(ErrorCode::Invalid, format!("{id} can't go from {:?} to {:?}", e.state, next).to_lowercase()));
        }
        if e.state != next {
            e.since = now;
        }
        e.state = next;
        e.detail = detail.map(|d| crate::state::clean(&d, 256));
        Ok(self.status(id, now).expect("exists"))
    }

    /// Set a built-in's state (Hyprland connected, a panel's bus name appeared).
    pub fn set_builtin(&mut self, id: &str, state: ComponentState, detail: Option<String>, now: Instant) -> bool {
        let Some(e) = self.entries.get_mut(id) else { return false };
        if e.state == state && e.detail == detail {
            return false;
        }
        if e.state != state {
            e.since = now;
        }
        e.state = state;
        e.detail = detail;
        true
    }

    /// A connection dropped: its components that didn't say they stopped failed.
    /// Returns the ids that changed.
    pub fn client_gone(&mut self, client: ClientId, now: Instant) -> Vec<String> {
        let mut changed = Vec::new();
        for (id, e) in &mut self.entries {
            if let Owner::Client { id: owner, name } = &e.owner
                && *owner == client
            {
                e.owner = Owner::Gone { name: name.clone() };
                if e.state != ComponentState::Stopped {
                    e.state = ComponentState::Failed;
                    e.detail = Some("its connection closed".into());
                    e.since = now;
                }
                changed.push(id.clone());
            }
        }
        changed
    }

    pub fn status(&self, id: &str, now: Instant) -> Option<ComponentStatus> {
        let e = self.entries.get(id)?;
        let deps_up = e.manifest.requires.iter().all(|r| self.entries.get(r).is_some_and(|d| d.state.is_up()));
        let effective = if e.state == ComponentState::Ready && !deps_up { ComponentState::Degraded } else { e.state };
        Some(ComponentStatus {
            manifest: e.manifest.clone(),
            state: e.state,
            effective,
            detail: e.detail.clone(),
            owner: e.owner.label(),
            since_ms: now.duration_since(e.since).as_millis() as u64,
        })
    }

    pub fn all(&self, now: Instant) -> Vec<ComponentStatus> {
        self.entries.keys().filter_map(|id| self.status(id, now)).collect()
    }

    /// Components whose dependencies include `id` (their effective state may change).
    pub fn dependents(&self, id: &str) -> Vec<String> {
        self.entries.values().filter(|e| e.manifest.requires.iter().any(|r| r == id)).map(|e| e.manifest.id.clone()).collect()
    }

    /// The ready component that handles `action`, if any.
    pub fn owner_of_action(&self, action: &str) -> Option<(String, Option<ClientId>, bool)> {
        self.entries
            .values()
            .find(|e| e.manifest.actions.iter().any(|a| a == action) && e.owner.client().is_some())
            .map(|e| (e.manifest.id.clone(), e.owner.client(), e.state.is_up()))
    }

    /// Actions declared by client components: (action, component id, up).
    pub fn declared_actions(&self) -> Vec<(String, String, bool)> {
        let mut out = Vec::new();
        for e in self.entries.values() {
            if matches!(e.owner, Owner::Client { .. } | Owner::Gone { .. }) {
                for a in &e.manifest.actions {
                    out.push((a.clone(), e.manifest.id.clone(), e.owner.client().is_some() && e.state.is_up()));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: ClientId) -> Owner {
        Owner::Client { id, name: "qs".into() }
    }

    fn manifest(id: &str, requires: &[&str]) -> ComponentManifest {
        ComponentManifest { id: id.into(), requires: requires.iter().map(|s| s.to_string()).collect(), actions: vec![format!("{id}.toggle")], ..Default::default() }
    }

    #[test]
    fn lifecycle() {
        let t = Instant::now();
        let mut c = Components::new(t);
        let s = c.register(manifest("inspector", &[]), client(1), t).unwrap();
        assert_eq!(s.state, ComponentState::Starting);
        assert_eq!(c.report("inspector", 1, HealthReport::Ready, None, t).unwrap().state, ComponentState::Ready);
        assert_eq!(c.report("inspector", 2, HealthReport::Ready, None, t).unwrap_err().code, ErrorCode::Forbidden);
        c.report("inspector", 1, HealthReport::Degraded, Some("slow".into()), t).unwrap();
        assert_eq!(c.client_gone(1, t), vec!["inspector".to_string()]);
        let s = c.status("inspector", t).unwrap();
        assert_eq!(s.state, ComponentState::Failed);
        assert_eq!(s.owner, "gone:qs");
        // Restarted runtime: registers again.
        assert_eq!(c.register(manifest("inspector", &[]), client(3), t).unwrap().state, ComponentState::Starting);
    }

    #[test]
    fn one_live_owner_per_id() {
        let t = Instant::now();
        let mut c = Components::new(t);
        c.register(manifest("x", &[]), client(1), t).unwrap();
        assert_eq!(c.register(manifest("x", &[]), client(2), t).unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(c.register(manifest(CORE, &[]), client(2), t).unwrap_err().code, ErrorCode::Conflict);
    }

    #[test]
    fn stopped_stays_stopped() {
        let t = Instant::now();
        let mut c = Components::new(t);
        c.register(manifest("x", &[]), client(1), t).unwrap();
        c.report("x", 1, HealthReport::Stopped, None, t).unwrap();
        c.client_gone(1, t);
        assert_eq!(c.status("x", t).unwrap().state, ComponentState::Stopped);
    }

    #[test]
    fn dependencies_degrade() {
        let t = Instant::now();
        let mut c = Components::new(t);
        c.register(manifest("needs-hypr", &[HYPRLAND]), client(1), t).unwrap();
        c.report("needs-hypr", 1, HealthReport::Ready, None, t).unwrap();
        assert_eq!(c.status("needs-hypr", t).unwrap().effective, ComponentState::Degraded, "Hyprland is still starting");
        c.set_builtin(HYPRLAND, ComponentState::Ready, None, t);
        assert_eq!(c.status("needs-hypr", t).unwrap().effective, ComponentState::Ready);
        assert_eq!(c.dependents(HYPRLAND), vec!["needs-hypr".to_string()]);
    }

    #[test]
    fn actions_of_components() {
        let t = Instant::now();
        let mut c = Components::new(t);
        c.register(manifest("x", &[]), client(1), t).unwrap();
        assert_eq!(c.owner_of_action("x.toggle"), Some(("x".into(), Some(1), false)));
        c.report("x", 1, HealthReport::Ready, None, t).unwrap();
        assert_eq!(c.declared_actions(), vec![("x.toggle".into(), "x".into(), true)]);
        assert!(c.report("x", 1, HealthReport::Ready, None, t).is_ok(), "ready again is fine");
    }

    #[test]
    fn builtins() {
        let t = Instant::now();
        let mut c = Components::new(t);
        assert_eq!(c.status(CORE, t).unwrap().state, ComponentState::Ready);
        assert_eq!(c.status("control-center", t).unwrap().owner, "panel1:io.github.design_nexus.nyx.ControlCenter");
        assert!(c.set_builtin("control-center", ComponentState::Ready, None, t));
        assert!(!c.set_builtin("control-center", ComponentState::Ready, None, t), "no change");
    }
}
