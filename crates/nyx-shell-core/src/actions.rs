//! Shell actions: a fixed table of routes, plus what components and overlays add.
//!
//! No route runs a command a client names: D-Bus calls, the switcher's socket and one
//! fixed program are all built in.

use crate::overlay::{Overlays, PANEL1};
use crate::registry::Components;
use crate::ClientId;
use nyx_shell_proto::{ActionInfo, ErrorBody, ErrorCode};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where an action goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// `Panel1.Toggle` on a GTK panel's bus name.
    Panel1Toggle { bus: String },
    /// One line on the switcher's socket (`overview`); the time is added when sent.
    Switcher { line: String },
    /// A fixed program and arguments.
    Launch { argv: Vec<String> },
    /// Toggle an overlay in the registry.
    Overlay { id: String },
    /// An `ActionInvoked` event to the component's connection.
    Component { component: String, client: ClientId },
}

impl Route {
    pub fn label(&self) -> String {
        match self {
            Route::Panel1Toggle { .. } => "dbus".into(),
            Route::Switcher { .. } => "socket".into(),
            Route::Launch { .. } => "launch".into(),
            Route::Overlay { .. } => "overlay".into(),
            Route::Component { component, .. } => format!("component:{component}"),
        }
    }

    /// Whether it reaches outside Shell Core's own clients (refused in read-only mode).
    pub fn external(&self) -> bool {
        matches!(self, Route::Panel1Toggle { .. } | Route::Switcher { .. } | Route::Launch { .. })
    }
}

/// Prefix of the per-overlay toggle actions.
pub const OVERLAY_TOGGLE: &str = "shell.overlay.toggle:";

struct Builtin {
    id: &'static str,
    title: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin { id: "shell.control-center.toggle", title: "Control Center" },
    Builtin { id: "shell.notification-center.toggle", title: "Notification Center" },
    Builtin { id: "shell.switcher.overview", title: "Window overview" },
    Builtin { id: "shell.glimpse.toggle", title: "Glimpse" },
];

/// What the built-in routes need to know about the system.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// Bus names that have an owner.
    pub buses: BTreeSet<String>,
    pub switcher_socket: Option<PathBuf>,
    pub glimpse: Option<PathBuf>,
}

impl Environment {
    /// The switcher's socket and Glimpse's program, looked up the way they find themselves.
    pub fn discover() -> Environment {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let switcher_socket = std::env::var_os("NYX_SWITCHER_SOCKET")
            .map(PathBuf::from)
            .or_else(|| runtime.map(|r| r.join("nyx-switcher.sock")));
        Environment { buses: BTreeSet::new(), switcher_socket, glimpse: which("nyx-glimpse") }
    }

    fn switcher_up(&self) -> bool {
        self.switcher_socket.as_deref().is_some_and(is_socket)
    }
}

fn is_socket(p: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(p).is_ok_and(|m| m.file_type().is_socket())
}

/// A program on `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(program))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

fn panel_bus(panel: &str) -> &'static str {
    PANEL1.iter().find(|(id, _)| *id == panel).map(|(_, bus)| *bus).expect("known panel")
}

/// Every action and whether it can run now.
pub fn list(env: &Environment, overlays: &Overlays, components: &Components) -> Vec<ActionInfo> {
    let mut out: Vec<ActionInfo> = BUILTINS
        .iter()
        .map(|b| {
            let (owner, available) = match b.id {
                "shell.control-center.toggle" => {
                    let bus = panel_bus("control-center");
                    (format!("panel1:{bus}"), env.buses.contains(bus))
                }
                "shell.notification-center.toggle" => {
                    let bus = panel_bus("notification-center");
                    (format!("panel1:{bus}"), env.buses.contains(bus))
                }
                "shell.switcher.overview" => (
                    format!("socket:{}", env.switcher_socket.as_deref().map(|p| p.display().to_string()).unwrap_or_default()),
                    env.switcher_up(),
                ),
                _ => ("launch:nyx-glimpse".into(), env.glimpse.is_some()),
            };
            ActionInfo { id: b.id.into(), title: b.title.into(), owner, available }
        })
        .collect();
    for id in overlays.client_overlays() {
        out.push(ActionInfo { id: format!("{OVERLAY_TOGGLE}{id}"), title: format!("Toggle {id}"), owner: format!("overlay:{id}"), available: true });
    }
    for (action, component, up) in components.declared_actions() {
        if out.iter().any(|a| a.id == action) {
            continue;
        }
        out.push(ActionInfo { id: action.clone(), title: action, owner: format!("component:{component}"), available: up });
    }
    out
}

/// Where `id` goes, or why it can't.
pub fn resolve(id: &str, env: &Environment, overlays: &Overlays, components: &Components) -> Result<Route, ErrorBody> {
    let unavailable = |what: &str| ErrorBody::new(ErrorCode::Unavailable, format!("{what} isn't running"));
    match id {
        "shell.control-center.toggle" | "shell.notification-center.toggle" => {
            let panel = id.trim_start_matches("shell.").trim_end_matches(".toggle");
            let bus = panel_bus(panel);
            if !env.buses.contains(bus) {
                return Err(unavailable(bus));
            }
            return Ok(Route::Panel1Toggle { bus: bus.into() });
        }
        "shell.switcher.overview" => {
            if !env.switcher_up() {
                return Err(unavailable("nyx-switcher"));
            }
            return Ok(Route::Switcher { line: "overview".into() });
        }
        "shell.glimpse.toggle" => {
            let program = env.glimpse.as_ref().ok_or_else(|| ErrorBody::new(ErrorCode::Unavailable, "nyx-glimpse isn't installed"))?;
            return Ok(Route::Launch { argv: vec![program.display().to_string(), "--toggle".into()] });
        }
        _ => {}
    }
    if let Some(overlay) = id.strip_prefix(OVERLAY_TOGGLE) {
        return match overlays.policy(overlay) {
            Some(_) if overlays.client_overlays().any(|o| o == overlay) => Ok(Route::Overlay { id: overlay.into() }),
            _ => Err(ErrorBody::new(ErrorCode::NotFound, format!("no overlay {overlay}"))),
        };
    }
    match components.owner_of_action(id) {
        Some((component, Some(client), true)) => Ok(Route::Component { component, client }),
        Some((component, _, _)) => Err(unavailable(&component)),
        None => Err(ErrorBody::new(ErrorCode::NotFound, format!("no action {id}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Owner;
    use nyx_shell_proto::{Category, ComponentManifest, HealthReport, OverlayPolicy};
    use std::time::Instant;

    fn env() -> Environment {
        Environment { buses: BTreeSet::new(), switcher_socket: Some("/nonexistent/sock".into()), glimpse: None }
    }

    #[test]
    fn builtins_follow_what_runs() {
        let t = Instant::now();
        let (o, c) = (Overlays::new(), Components::new(t));
        let mut e = env();
        assert_eq!(resolve("shell.control-center.toggle", &e, &o, &c).unwrap_err().code, ErrorCode::Unavailable);
        e.buses.insert("io.github.design_nexus.nyx.ControlCenter".into());
        assert_eq!(
            resolve("shell.control-center.toggle", &e, &o, &c).unwrap(),
            Route::Panel1Toggle { bus: "io.github.design_nexus.nyx.ControlCenter".into() }
        );
        assert_eq!(resolve("shell.switcher.overview", &e, &o, &c).unwrap_err().code, ErrorCode::Unavailable);
        assert_eq!(resolve("shell.glimpse.toggle", &e, &o, &c).unwrap_err().code, ErrorCode::Unavailable);
        e.glimpse = Some("/usr/bin/nyx-glimpse".into());
        assert_eq!(resolve("shell.glimpse.toggle", &e, &o, &c).unwrap(), Route::Launch { argv: vec!["/usr/bin/nyx-glimpse".into(), "--toggle".into()] });
        let listed = list(&e, &o, &c);
        assert!(listed.iter().find(|a| a.id == "shell.control-center.toggle").unwrap().available);
        assert!(!listed.iter().find(|a| a.id == "shell.notification-center.toggle").unwrap().available);
        assert_eq!(resolve("rm -rf", &e, &o, &c).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn overlays_and_components_add_actions() {
        let t = Instant::now();
        let mut o = Overlays::new();
        let mut c = Components::new(t);
        let owner = Owner::Client { id: 4, name: "qs".into() };
        o.register(OverlayPolicy::new("nyx.inspector", Category::Transient), owner.clone()).unwrap();
        c.register(ComponentManifest { id: "nyx.inspector".into(), actions: vec!["shell.inspector.toggle".into()], ..Default::default() }, owner, t)
            .unwrap();
        let e = env();
        assert_eq!(resolve("shell.overlay.toggle:nyx.inspector", &e, &o, &c).unwrap(), Route::Overlay { id: "nyx.inspector".into() });
        assert_eq!(resolve("shell.overlay.toggle:control-center", &e, &o, &c).unwrap_err().code, ErrorCode::NotFound);
        assert_eq!(resolve("shell.inspector.toggle", &e, &o, &c).unwrap_err().code, ErrorCode::Unavailable, "not ready yet");
        c.report("nyx.inspector", 4, HealthReport::Ready, None, t).unwrap();
        let r = resolve("shell.inspector.toggle", &e, &o, &c).unwrap();
        assert_eq!(r, Route::Component { component: "nyx.inspector".into(), client: 4 });
        assert_eq!(r.label(), "component:nyx.inspector");
        assert!(!r.external());
        let ids: Vec<String> = list(&e, &o, &c).into_iter().map(|a| a.id).collect();
        assert!(ids.contains(&"shell.overlay.toggle:nyx.inspector".to_string()));
        assert!(ids.contains(&"shell.inspector.toggle".to_string()));
    }
}
