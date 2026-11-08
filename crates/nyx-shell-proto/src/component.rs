//! Shell components: what they declare and the state they're in.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    #[default]
    Surface,
    Service,
    Runtime,
    Bridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub id: String,
    #[serde(default)]
    pub kind: ComponentKind,
    /// The process it lives in (`nyx-shell-qs`, its own binary name, …).
    #[serde(default)]
    pub process: String,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    /// Action ids it handles; invocations arrive as `ActionInvoked` events.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Overlay ids it shows.
    #[serde(default)]
    pub overlays: Vec<String>,
    /// Descriptive: systemd restarts processes.
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl ComponentManifest {
    pub fn validate(&self) -> Result<(), String> {
        if !crate::valid_id(&self.id) {
            return Err(format!("invalid component id {:?}", self.id));
        }
        for id in self.requires.iter().chain(&self.optional).chain(&self.actions).chain(&self.overlays) {
            if !crate::valid_id(id) {
                return Err(format!("invalid id {id:?} in the manifest"));
            }
        }
        if self.requires.contains(&self.id) || self.optional.contains(&self.id) {
            return Err("a component can't depend on itself".into());
        }
        if self.process.len() > 64 || self.capabilities.len() > 16 || self.actions.len() > 64 || self.overlays.len() > 64 {
            return Err("manifest too large".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    Registered,
    Starting,
    Ready,
    Degraded,
    Stopped,
    Failed,
}

impl ComponentState {
    /// Whether `self` may move to `next`.
    pub fn can_become(self, next: ComponentState) -> bool {
        use ComponentState::*;
        match next {
            Registered => false,
            Starting => matches!(self, Registered | Stopped | Failed),
            Ready => matches!(self, Registered | Starting | Degraded | Ready),
            Degraded => matches!(self, Starting | Ready | Degraded),
            Stopped | Failed => true,
        }
    }

    pub fn is_up(self) -> bool {
        matches!(self, ComponentState::Ready | ComponentState::Degraded)
    }
}

/// What a component may report about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthReport {
    Ready,
    Degraded,
    Stopped,
}

impl From<HealthReport> for ComponentState {
    fn from(h: HealthReport) -> Self {
        match h {
            HealthReport::Ready => ComponentState::Ready,
            HealthReport::Degraded => ComponentState::Degraded,
            HealthReport::Stopped => ComponentState::Stopped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentStatus {
    #[serde(flatten)]
    pub manifest: ComponentManifest,
    pub state: ComponentState,
    /// After dependencies: `degraded` when a required one isn't up.
    pub effective: ComponentState,
    pub detail: Option<String>,
    /// `client:NAME`, `panel1:BUSNAME` or `builtin`.
    pub owner: String,
    /// Milliseconds since the state last changed.
    pub since_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ComponentState::*;

    #[test]
    fn transitions() {
        assert!(Registered.can_become(Starting));
        assert!(Starting.can_become(Ready));
        assert!(Ready.can_become(Degraded));
        assert!(Degraded.can_become(Ready));
        assert!(Ready.can_become(Failed));
        assert!(Failed.can_become(Starting));
        assert!(!Stopped.can_become(Ready), "a stopped component starts again first");
        assert!(!Ready.can_become(Registered));
        assert!(!Registered.can_become(Degraded));
    }

    #[test]
    fn manifests() {
        let m: ComponentManifest = serde_json::from_str(r#"{"id":"nyx.inspector","overlays":["nyx.inspector"]}"#).unwrap();
        m.validate().unwrap();
        assert_eq!(m.kind, ComponentKind::Surface);
        let selfish = ComponentManifest { requires: vec!["a".into()], ..ComponentManifest { id: "a".into(), ..Default::default() } };
        assert!(selfish.validate().is_err());
    }

    #[test]
    fn status_is_flat() {
        let s = ComponentStatus {
            manifest: ComponentManifest { id: "x".into(), ..Default::default() },
            state: Ready,
            effective: Ready,
            detail: None,
            owner: "builtin".into(),
            since_ms: 0,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["id"], "x");
        assert_eq!(v["state"], "ready");
        let back: ComponentStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
