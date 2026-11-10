//! Frames on the wire: requests, replies and events.

use crate::action::ActionInfo;
use crate::component::{ComponentManifest, ComponentStatus, HealthReport};
use crate::config::ConfigState;
use crate::desktop::DesktopDelta;
use crate::overlay::{OverlayPolicy, OverlayState};
use crate::session::SessionStatus;
use crate::theme::ThemeState;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The version this crate speaks.
pub const PROTOCOL_VERSION: u32 = 1;
/// The oldest version the server still answers.
pub const MIN_PROTOCOL_VERSION: u32 = 1;
/// Longest line accepted, in bytes.
pub const MAX_LINE: usize = 1 << 20;

/// A request as it arrives, before its operation is decoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRequest {
    pub v: u32,
    pub id: u64,
    pub op: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

impl RawRequest {
    pub fn new(id: u64, op: &Op) -> RawRequest {
        let mut v = serde_json::to_value(op).expect("operations serialize");
        let obj = v.as_object_mut().expect("operations are objects");
        let name = obj.remove("op").and_then(|n| n.as_str().map(str::to_string)).unwrap_or_default();
        let body = obj.remove("body").unwrap_or(Value::Null);
        RawRequest { v: PROTOCOL_VERSION, id, op: name, body }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub client: String,
    #[serde(default = "one")]
    pub min: u32,
    #[serde(default = "one")]
    pub max: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloReply {
    pub protocol: u32,
    pub server: String,
    pub session: String,
    pub capabilities: Vec<Capability>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WorkspaceAction {
    Focus { workspace: String },
    ToggleSpecial { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowActionKind {
    Focus,
    Close,
    ToggleFloating,
    /// Float a tiled window; nothing for one that floats already.
    SetFloating,
    /// Tile a floating window; nothing for a tiled one.
    SetTiled,
    /// Maximize a floating window, or expand a tiled one over its layout (it stays
    /// tiled); again restores it.
    ToggleMaximized,
    /// True fullscreen, separate from maximizing.
    ToggleFullscreen,
    /// Show a floating window on every workspace of its monitor.
    TogglePinned,
    /// Hide the window where it can be brought back (Omarchy's floating workspaces).
    Minimize,
    MoveToWorkspace,
    MoveToMonitor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowAction {
    pub window: String,
    pub action: WindowActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    #[serde(default)]
    pub follow: bool,
}

/// Every operation. Names and bodies are the wire format (`docs/PROTOCOL.md`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "body")]
pub enum Op {
    Hello(Hello),
    GetSessionStatus,
    SubscribeSessionEvents,
    GetDesktopSnapshot,
    SubscribeDesktopChanges,
    RequestWorkspaceAction(WorkspaceAction),
    RequestWindowAction(WindowAction),
    RegisterOverlay {
        policy: OverlayPolicy,
    },
    UnregisterOverlay {
        id: String,
    },
    ShowOverlay {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        monitor: Option<String>,
    },
    HideOverlay {
        id: String,
    },
    ToggleOverlay {
        id: String,
    },
    GetOverlayState,
    SubscribeOverlayState,
    RegisterComponent {
        manifest: ComponentManifest,
    },
    ReportReady {
        id: String,
    },
    ReportHealth {
        id: String,
        state: HealthReport,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    GetComponentStatus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    ListActions,
    InvokeAction {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<Value>,
    },
    SubscribeActionAvailability,
    GetShellConfig,
    UpdateShellConfig {
        key: String,
        value: Value,
    },
    ResetShellConfig {
        key: String,
    },
    SubscribeConfigChanges,
    GetThemeGeneration,
    SubscribeThemeChanges,
    Unsubscribe {
        topic: Topic,
    },
}

/// Why a request couldn't be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    UnknownOp,
    BadBody(String),
}

impl Op {
    pub const NAMES: &'static [&'static str] = &[
        "Hello",
        "GetSessionStatus",
        "SubscribeSessionEvents",
        "GetDesktopSnapshot",
        "SubscribeDesktopChanges",
        "RequestWorkspaceAction",
        "RequestWindowAction",
        "RegisterOverlay",
        "UnregisterOverlay",
        "ShowOverlay",
        "HideOverlay",
        "ToggleOverlay",
        "GetOverlayState",
        "SubscribeOverlayState",
        "RegisterComponent",
        "ReportReady",
        "ReportHealth",
        "GetComponentStatus",
        "ListActions",
        "InvokeAction",
        "SubscribeActionAvailability",
        "GetShellConfig",
        "UpdateShellConfig",
        "ResetShellConfig",
        "SubscribeConfigChanges",
        "GetThemeGeneration",
        "SubscribeThemeChanges",
        "Unsubscribe",
    ];

    /// Decode `op` with its `body`. A body may be left out (or be `{}`) for operations
    /// without required fields.
    pub fn decode(op: &str, body: Value) -> Result<Op, DecodeError> {
        if !Self::NAMES.contains(&op) {
            return Err(DecodeError::UnknownOp);
        }
        let attempt = |b: Option<Value>| {
            let mut m = Map::new();
            m.insert("op".into(), Value::String(op.into()));
            if let Some(b) = b {
                m.insert("body".into(), b);
            }
            serde_json::from_value::<Op>(Value::Object(m))
        };
        let empty = matches!(&body, Value::Object(m) if m.is_empty());
        let first = match body {
            Value::Null => attempt(None),
            b => attempt(Some(b)),
        };
        first
            .or_else(|e| {
                if empty {
                    attempt(None).map_err(|_| e)
                } else {
                    attempt(Some(Value::Object(Map::new()))).map_err(|_| e)
                }
            })
            .map_err(|e| DecodeError::BadBody(e.to_string()))
    }

    /// What a client needs to run this, if anything.
    pub fn capability(&self) -> Option<Capability> {
        use Op::*;
        Some(match self {
            Hello(_) | Unsubscribe { .. } => return None,
            GetSessionStatus
            | SubscribeSessionEvents
            | GetDesktopSnapshot
            | SubscribeDesktopChanges
            | GetOverlayState
            | SubscribeOverlayState
            | GetComponentStatus { .. }
            | ListActions
            | SubscribeActionAvailability
            | GetShellConfig
            | SubscribeConfigChanges
            | GetThemeGeneration
            | SubscribeThemeChanges => Capability::DesktopRead,
            RequestWorkspaceAction(_) | RequestWindowAction(_) => Capability::DesktopAct,
            RegisterOverlay { .. } | UnregisterOverlay { .. } | ShowOverlay { .. } | HideOverlay { .. } | ToggleOverlay { .. } => {
                Capability::OverlayManage
            }
            RegisterComponent { .. } | ReportReady { .. } | ReportHealth { .. } => Capability::ComponentRegister,
            InvokeAction { .. } => Capability::ActionInvoke,
            UpdateShellConfig { .. } | ResetShellConfig { .. } => Capability::ConfigWrite,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "desktop.read")]
    DesktopRead,
    #[serde(rename = "desktop.act")]
    DesktopAct,
    #[serde(rename = "overlay.manage")]
    OverlayManage,
    #[serde(rename = "component.register")]
    ComponentRegister,
    #[serde(rename = "action.invoke")]
    ActionInvoke,
    #[serde(rename = "config.write")]
    ConfigWrite,
    /// `RequestWindowAction` on the client's own windows (the same process id), except
    /// `focus`.
    #[serde(rename = "window.own")]
    WindowOwn,
}

impl Capability {
    pub const ALL: [Capability; 7] = [
        Capability::DesktopRead,
        Capability::DesktopAct,
        Capability::OverlayManage,
        Capability::ComponentRegister,
        Capability::ActionInvoke,
        Capability::ConfigWrite,
        Capability::WindowOwn,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Capability::DesktopRead => "desktop.read",
            Capability::DesktopAct => "desktop.act",
            Capability::OverlayManage => "overlay.manage",
            Capability::ComponentRegister => "component.register",
            Capability::ActionInvoke => "action.invoke",
            Capability::ConfigWrite => "config.write",
            Capability::WindowOwn => "window.own",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Session,
    Desktop,
    Overlays,
    Actions,
    Config,
    Theme,
}

impl Topic {
    pub const ALL: [Topic; 6] = [Topic::Session, Topic::Desktop, Topic::Overlays, Topic::Actions, Topic::Config, Topic::Theme];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    UnsupportedVersion,
    UnknownOp,
    Forbidden,
    NotFound,
    Conflict,
    Invalid,
    Unavailable,
    ReadOnly,
    Locked,
    TooLarge,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub msg: String,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, msg: impl Into<String>) -> ErrorBody {
        ErrorBody { code, msg: msg.into() }
    }
}

impl std::fmt::Display for ErrorBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = serde_json::to_value(self.code).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        write!(f, "{code}: {}", self.msg)
    }
}

impl std::error::Error for ErrorBody {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<ErrorBody>,
}

impl Reply {
    pub fn ok(id: u64, body: Value) -> Reply {
        Reply { id, ok: Some(body), err: None }
    }

    pub fn err(id: u64, err: ErrorBody) -> Reply {
        Reply { id, ok: None, err: Some(err) }
    }
}

/// Events, with their bodies. `seq` and `ts` are added per connection ([`EventFrame`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", content = "body")]
pub enum Event {
    SessionChanged { status: SessionStatus },
    ComponentChanged { component: ComponentStatus },
    DesktopChanged(DesktopDelta),
    OverlayChanged { overlay: OverlayState },
    OverlayRemoved { id: String },
    ActionsChanged { actions: Vec<ActionInfo> },
    ConfigChanged { config: ConfigState },
    ThemeChanged { theme: ThemeState },
    Resync { topics: Vec<Topic> },
    ActionInvoked { action: String, args: Value, invocation: u64 },
    Shutdown {},
}

impl Event {
    /// The topic a client must subscribe to for this event; `None` for events every
    /// client gets (or only the one it's addressed to).
    pub fn topic(&self) -> Option<Topic> {
        match self {
            Event::SessionChanged { .. } | Event::ComponentChanged { .. } => Some(Topic::Session),
            Event::DesktopChanged(_) => Some(Topic::Desktop),
            Event::OverlayChanged { .. } | Event::OverlayRemoved { .. } => Some(Topic::Overlays),
            Event::ActionsChanged { .. } => Some(Topic::Actions),
            Event::ConfigChanged { .. } => Some(Topic::Config),
            Event::ThemeChanged { .. } => Some(Topic::Theme),
            Event::Resync { .. } | Event::ActionInvoked { .. } | Event::Shutdown {} => None,
        }
    }
}

/// An event as sent on one connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventFrame {
    pub ev: String,
    pub seq: u64,
    pub ts: u64,
    #[serde(default)]
    pub body: Value,
}

impl EventFrame {
    pub fn new(event: &Event, seq: u64, ts: u64) -> EventFrame {
        let mut v = serde_json::to_value(event).expect("events serialize");
        let obj = v.as_object_mut().expect("events are objects");
        let ev = obj.remove("ev").and_then(|n| n.as_str().map(str::to_string)).unwrap_or_default();
        let body = obj.remove("body").unwrap_or(Value::Object(Map::new()));
        EventFrame { ev, seq, ts, body }
    }

    /// Decode back into an [`Event`] (clients).
    pub fn event(&self) -> Result<Event, serde_json::Error> {
        let mut m = Map::new();
        m.insert("ev".into(), Value::String(self.ev.clone()));
        m.insert("body".into(), self.body.clone());
        serde_json::from_value(Value::Object(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bodies_may_be_left_out_or_empty() {
        assert_eq!(Op::decode("GetDesktopSnapshot", Value::Null), Ok(Op::GetDesktopSnapshot));
        assert_eq!(Op::decode("GetDesktopSnapshot", json!({})), Ok(Op::GetDesktopSnapshot));
        assert_eq!(Op::decode("GetComponentStatus", Value::Null), Ok(Op::GetComponentStatus { id: None }));
        assert_eq!(Op::decode("GetComponentStatus", json!({"id": "x"})), Ok(Op::GetComponentStatus { id: Some("x".into()) }));
    }

    #[test]
    fn unknown_and_malformed() {
        assert_eq!(Op::decode("RunCommand", json!({"cmd": "rm"})), Err(DecodeError::UnknownOp));
        assert!(matches!(Op::decode("ShowOverlay", json!({"id": 5})), Err(DecodeError::BadBody(_))));
        assert!(matches!(Op::decode("RequestWindowAction", json!({"window": "a", "action": "explode"})), Err(DecodeError::BadBody(_))));
        assert!(matches!(Op::decode("ShowOverlay", Value::Null), Err(DecodeError::BadBody(_))));
    }

    #[test]
    fn names_cover_every_operation() {
        for name in Op::NAMES {
            // Every name is a variant: decoding fails on the body, never on the name.
            match Op::decode(name, json!({"__": 1})) {
                Ok(_) | Err(DecodeError::BadBody(_)) => {}
                Err(DecodeError::UnknownOp) => panic!("{name}"),
            }
        }
    }

    #[test]
    fn request_round_trip() {
        let op = Op::ShowOverlay { id: "nyx.inspector".into(), monitor: None };
        let raw = RawRequest::new(3, &op);
        assert_eq!(raw.op, "ShowOverlay");
        let text = serde_json::to_string(&raw).unwrap();
        assert_eq!(text, r#"{"v":1,"id":3,"op":"ShowOverlay","body":{"id":"nyx.inspector"}}"#);
        let back: RawRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(Op::decode(&back.op, back.body).unwrap(), op);
        let unit = RawRequest::new(4, &Op::ListActions);
        assert_eq!(serde_json::to_string(&unit).unwrap(), r#"{"v":1,"id":4,"op":"ListActions"}"#);
    }

    #[test]
    fn workspace_actions() {
        let op = Op::decode("RequestWorkspaceAction", json!({"action": "focus", "workspace": "3"})).unwrap();
        assert_eq!(op, Op::RequestWorkspaceAction(WorkspaceAction::Focus { workspace: "3".into() }));
        assert_eq!(op.capability(), Some(Capability::DesktopAct));
    }

    #[test]
    fn events_frame_and_back() {
        let ev = Event::OverlayRemoved { id: "x".into() };
        let f = EventFrame::new(&ev, 9, 100);
        assert_eq!(serde_json::to_value(&f).unwrap(), json!({"ev": "OverlayRemoved", "seq": 9, "ts": 100, "body": {"id": "x"}}));
        assert_eq!(f.event().unwrap(), ev);
        let s = EventFrame::new(&Event::Shutdown {}, 1, 0);
        assert_eq!(s.body, json!({}));
        assert_eq!(s.event().unwrap(), Event::Shutdown {});
    }

    #[test]
    fn capability_names() {
        for c in Capability::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.name()));
        }
        assert_eq!(Op::Unsubscribe { topic: Topic::Desktop }.capability(), None);
        assert_eq!(Op::UpdateShellConfig { key: "a".into(), value: json!(1) }.capability(), Some(Capability::ConfigWrite));
    }
}
