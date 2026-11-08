//! Session status and diagnostics.

use crate::component::ComponentStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Ready,
    /// Hyprland is unreachable; the rest keeps working on the last known state.
    Degraded,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositorStatus {
    pub connected: bool,
    pub generation: u64,
    pub last_event_ms_ago: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcStatus {
    pub clients: usize,
    pub dropped_events: u64,
    pub resyncs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySummary {
    pub id: String,
    pub owner: String,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSummary {
    pub generation: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeSummary {
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub version: String,
    pub session: String,
    pub state: SessionState,
    pub read_only: bool,
    pub uptime_ms: u64,
    pub locked: bool,
    pub compositor: CompositorStatus,
    pub ipc: IpcStatus,
    pub components: Vec<ComponentStatus>,
    pub overlays: Vec<OverlaySummary>,
    pub config: ConfigSummary,
    pub theme: ThemeSummary,
}
