//! Nyx Shell Core: the coordinator of the Nyx shell (`docs/ARCHITECTURE.md`).
//!
//! [`core::Core`] holds every piece of shared state and is driven by messages, so its
//! rules are tested without sockets or Hyprland. [`server`] runs it: the socket, the
//! Hyprland watcher, the D-Bus bridge and the file watchers.

pub mod actions;
pub mod bridge;
pub mod compositor;
pub mod config;
pub mod core;
pub mod focus;
pub mod overlay;
pub mod peer;
pub mod registry;
pub mod server;
pub mod session;
pub mod state;
pub mod theme;

/// A connection, numbered from 1 for the life of the process.
pub type ClientId = u64;

/// Who owns an overlay or a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Client { id: ClientId, name: String },
    /// A client that disconnected.
    Gone { name: String },
    /// A GTK panel on D-Bus.
    Panel1 { bus: String },
    Builtin,
}

impl Owner {
    pub fn label(&self) -> String {
        match self {
            Owner::Client { name, .. } => format!("client:{name}"),
            Owner::Gone { name } => format!("gone:{name}"),
            Owner::Panel1 { bus } => format!("panel1:{bus}"),
            Owner::Builtin => "builtin".into(),
        }
    }

    pub fn client(&self) -> Option<ClientId> {
        match self {
            Owner::Client { id, .. } => Some(*id),
            _ => None,
        }
    }
}

/// Microseconds since the Unix epoch.
pub fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// Window titles in logs: hidden unless the user allowed them (`log.sensitive`, or
/// `NYX_SHELL_LOG_SENSITIVE=1`).
pub struct Redacted<'a>(pub &'a str, pub bool);

impl std::fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.1 { f.write_str(self.0) } else { write!(f, "<{} chars>", self.0.chars().count()) }
    }
}
