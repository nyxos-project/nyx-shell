//! Overlay policies and the state of each overlay.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// OSDs, status indicators: never take the keyboard, never close others.
    Passive,
    /// Control Center, quick panels, Glimpse: open, take input, close.
    #[default]
    Transient,
    /// Dock, desktop widgets: there all the time.
    Persistent,
    /// Assistant Aura: drawn over everything, no input.
    Presentation,
    /// The lock screen and authentication UI: outside Shell Core, listed only.
    SecureExternal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Keyboard {
    #[default]
    None,
    OnDemand,
    Exclusive,
}

/// What happens over a fullscreen window on the overlay's monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FullscreenRule {
    /// Draw anyway.
    Show,
    /// Hidden until the fullscreen window is gone; the request is kept.
    #[default]
    Hide,
    /// Refuse to open.
    Block,
}

/// Where an overlay opens.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Placement {
    /// The focused monitor.
    #[default]
    Active,
    /// The monitor under the pointer.
    Pointer,
    /// Every monitor.
    All,
    /// A monitor by connector name; the focused one when it isn't connected.
    Monitor(String),
}

impl std::fmt::Display for Placement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Placement::Active => f.write_str("active"),
            Placement::Pointer => f.write_str("pointer"),
            Placement::All => f.write_str("all"),
            Placement::Monitor(m) => write!(f, "monitor:{m}"),
        }
    }
}

impl std::str::FromStr for Placement {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "active" => Placement::Active,
            "pointer" => Placement::Pointer,
            "all" => Placement::All,
            _ => match s.strip_prefix("monitor:") {
                Some(m) if !m.is_empty() && m.len() <= 64 && m.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) => {
                    Placement::Monitor(m.into())
                }
                _ => return Err(format!("unknown placement {s:?}: active, pointer, all or monitor:NAME")),
            },
        })
    }
}

impl Serialize for Placement {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Placement {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayPolicy {
    pub id: String,
    #[serde(default)]
    pub category: Category,
    /// Showing one overlay hides the others in its group. Transient overlays default
    /// to `panels`, the Control Center's and Notification Center's group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default)]
    pub keyboard: Keyboard,
    #[serde(default)]
    pub fullscreen: FullscreenRule,
    #[serde(default)]
    pub placement: Placement,
    /// Give focus back to the previous window on close; transient overlays default to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_focus: Option<bool>,
}

/// The group the GTK panels share.
pub const PANELS: &str = "panels";

impl OverlayPolicy {
    pub fn new(id: &str, category: Category) -> OverlayPolicy {
        OverlayPolicy {
            id: id.into(),
            category,
            group: None,
            keyboard: Keyboard::None,
            fullscreen: FullscreenRule::Hide,
            placement: Placement::Active,
            restore_focus: None,
        }
    }

    /// Check the rules that don't depend on the registry.
    pub fn validate(&self) -> Result<(), String> {
        if !crate::valid_id(&self.id) {
            return Err(format!("invalid overlay id {:?}", self.id));
        }
        if let Some(g) = &self.group
            && !crate::valid_id(g)
        {
            return Err(format!("invalid group {g:?}"));
        }
        if matches!(self.category, Category::Passive | Category::Presentation) && self.keyboard != Keyboard::None {
            return Err(format!("{} overlays never take the keyboard", category_name(self.category)));
        }
        if self.category == Category::Passive && self.restore_focus == Some(true) {
            return Err("passive overlays never take focus, so there is none to restore".into());
        }
        Ok(())
    }

    /// The group after defaults.
    pub fn effective_group(&self) -> Option<&str> {
        match (&self.group, self.category) {
            (Some(g), _) => Some(g),
            (None, Category::Transient) => Some(PANELS),
            _ => None,
        }
    }

    /// Whether focus goes back on close, after defaults.
    pub fn effective_restore_focus(&self) -> bool {
        self.restore_focus.unwrap_or(self.category == Category::Transient)
    }
}

pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Passive => "passive",
        Category::Transient => "transient",
        Category::Persistent => "persistent",
        Category::Presentation => "presentation",
        Category::SecureExternal => "secure-external",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenReason {
    Locked,
    Fullscreen,
}

/// A pinned overlay's box: its size and its top-left corner, in logical pixels from
/// its monitor's top-left. Shell Core lays pinned overlays out; surfaces follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub size: [u32; 2],
    pub at: [i32; 2],
}

/// The smallest and largest pinned overlay, logical pixels.
pub const PIN_MIN: [u32; 2] = [160, 100];
pub const PIN_MAX: [u32; 2] = [800, 900];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayState {
    pub id: String,
    /// `client:NAME`, `panel1:BUSNAME` or `builtin`.
    pub owner: String,
    pub category: Category,
    pub group: Option<String>,
    pub keyboard: Keyboard,
    pub fullscreen: FullscreenRule,
    pub placement: Placement,
    /// What was asked.
    pub requested: bool,
    /// What the surface should do.
    pub visible: bool,
    /// Where it shows; `None` for every monitor (or not placed yet).
    pub monitor: Option<String>,
    pub reason: Option<HiddenReason>,
    /// Bumped on every change, so a surface can ignore stale instructions.
    pub revision: u64,
    /// Set while pinned (`PinOverlay`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<Pin>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_strings() {
        for s in ["active", "pointer", "all", "monitor:DP-2"] {
            let p: Placement = s.parse().unwrap();
            assert_eq!(p.to_string(), s);
        }
        assert!("monitor:".parse::<Placement>().is_err());
        assert!("monitor:a\"b".parse::<Placement>().is_err());
        assert!("elsewhere".parse::<Placement>().is_err());
    }

    #[test]
    fn policy_defaults_and_rules() {
        let p: OverlayPolicy = serde_json::from_str(r#"{"id":"nyx.inspector"}"#).unwrap();
        assert_eq!(p.category, Category::Transient);
        assert_eq!(p.effective_group(), Some(PANELS));
        assert!(p.effective_restore_focus());
        assert_eq!(p.placement, Placement::Active);
        p.validate().unwrap();

        let mut osd = OverlayPolicy::new("nyx.osd", Category::Passive);
        assert_eq!(osd.effective_group(), None);
        assert!(!osd.effective_restore_focus());
        osd.keyboard = Keyboard::Exclusive;
        assert!(osd.validate().is_err(), "passive overlays can't take the keyboard");

        let bad = OverlayPolicy::new("Bad Id", Category::Transient);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn wire_form() {
        let p = OverlayPolicy { group: Some("x".into()), ..OverlayPolicy::new("a", Category::SecureExternal) };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["category"], "secure-external");
        assert_eq!(v["keyboard"], "none");
        assert_eq!(v["fullscreen"], "hide");
        assert_eq!(v["placement"], "active");
    }
}
