//! The coordinator's state and rules, driven by [`Msg`]s on one task.
//!
//! `Core` never blocks and never does I/O beyond reading its small settings and theme
//! files: anything slow or external becomes an [`Effect`] the runtime carries out,
//! reporting back with another message. Tests drive it directly.

use crate::actions::{self, Environment, Route};
use crate::compositor::{self, Update};
use crate::config::ConfigStore;
use crate::focus::Focus;
use crate::overlay::{Change, Overlays, PANEL1};
use crate::peer::Identity;
use crate::registry::{self, Components};
use crate::state::normalize;
use crate::theme::ThemeSource;
use crate::{ClientId, Owner, now_us};
use nyx_desktop::actions::Dispatch;
use nyx_shell_proto::overlay::PANELS;
use nyx_shell_proto::session::{CompositorStatus, ConfigSummary, IpcStatus, OverlaySummary, ThemeSummary};
use nyx_shell_proto::{
    ActionInfo, ComponentState, ConfigState, DesktopState, ErrorBody, ErrorCode, Event, EventFrame, HealthReport, HelloReply, Op,
    Placement, Reply, Routed, SessionState, SessionStatus, ThemeState, Topic, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where a deferred reply goes.
pub type ReplyTo = (u64, oneshot::Sender<Reply>);

pub enum Msg {
    Connect { client: ClientId, identity: Identity, events: mpsc::Sender<String> },
    Disconnect { client: ClientId },
    Request { client: ClientId, id: u64, op: Op, reply: oneshot::Sender<Reply> },
    Desktop(Update),
    Locked(bool),
    /// `Panel1.Opened(id)` from a GTK panel.
    PanelOpened { id: String },
    /// A watched bus name gained or lost its owner.
    BusOwner { bus: String, present: bool },
    /// The session bus connection is up (or failed).
    Dbus(bool),
    PointerResolved { client: ClientId, overlay: String, monitor: Option<String>, reply: ReplyTo },
    ConfigFiles,
    ThemeFiles,
    /// The first snapshot didn't arrive in time: ready anyway.
    ReadyTimeout,
    Shutdown,
}

pub enum BridgeCmd {
    /// Emit `Panel1.Opened("nyx-shell:ID")` so the GTK panels close.
    Announce(String),
    Toggle { bus: String, reply: Option<ReplyTo> },
}

/// The GTK panels' layer surfaces, by `Panel1` id: a panel is open while one is mapped.
const PANEL_LAYERS: &[(&str, &[&str])] = &[
    ("control-center", &["nyx-control-center"]),
    ("notification-center", &["nyx-notification-center"]),
    ("switcher", &["nyx-switcher", "nyx-overview"]),
];

pub enum Effect {
    Dispatch { steps: Vec<Dispatch>, reply: Option<ReplyTo> },
    Bridge(BridgeCmd),
    Switcher { path: PathBuf, line: String, reply: ReplyTo },
    Launch { argv: Vec<String>, reply: ReplyTo },
    ResolvePointer { client: ClientId, overlay: String, reply: ReplyTo },
    /// Tell systemd the session is ready.
    Ready,
    /// Remember which workspace each virtual monitor shows, for the next login.
    SaveVirtualSession(nyx_desktop::virtual_monitors::SessionState),
}

struct Client {
    identity: Identity,
    name: String,
    events: mpsc::Sender<String>,
    subs: BTreeSet<Topic>,
    seq: u64,
    lagging: bool,
}

pub struct Options {
    pub read_only: bool,
    pub config: ConfigStore,
    pub theme: ThemeSource,
    pub env: Environment,
    pub session_id: String,
    /// Which workspace each virtual monitor showed last session.
    pub virtual_session: nyx_desktop::virtual_monitors::SessionState,
}

pub struct Core {
    session_id: String,
    started: Instant,
    read_only: bool,
    state: SessionState,
    desktop: DesktopState,
    raw: Arc<nyx_desktop::Desktop>,
    /// Whether each panel's layer surface was mapped at the last update.
    panel_layers: BTreeMap<&'static str, bool>,
    last_event: Option<Instant>,
    locked: bool,
    clients: BTreeMap<ClientId, Client>,
    overlays: Overlays,
    focus: Focus,
    components: Components,
    env: Environment,
    actions: Vec<ActionInfo>,
    store: ConfigStore,
    config: ConfigState,
    theme_source: ThemeSource,
    theme_fingerprint: u64,
    theme: ThemeState,
    invocations: u64,
    dropped: u64,
    resyncs: u64,
    effects: Vec<Effect>,
    /// Which workspace each virtual monitor shows, and whether last session's was
    /// brought back yet.
    virtual_session: nyx_desktop::virtual_monitors::SessionState,
    virtual_restored: bool,
}

fn err(code: ErrorCode, msg: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code, msg)
}

fn answer(reply: oneshot::Sender<Reply>, id: u64, result: Result<Value, ErrorBody>) {
    let _ = reply.send(match result {
        Ok(v) => Reply::ok(id, v),
        Err(e) => Reply::err(id, e),
    });
}

fn to_value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// Client names from `Hello`: id characters only, at most 64.
fn display_name(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).take(64).collect()
}

impl Core {
    pub fn new(o: Options) -> Core {
        let now = Instant::now();
        let (config, errors) = o.config.load();
        let theme_fingerprint = o.theme.fingerprint();
        let theme = o.theme.read(1);
        let mut overlays = Overlays::new();
        let desktop = DesktopState::default();
        overlays.set_overrides(config.overlays.clone(), &desktop, false);
        let mut core = Core {
            session_id: o.session_id,
            started: now,
            read_only: o.read_only,
            state: SessionState::Starting,
            desktop,
            raw: Arc::default(),
            panel_layers: BTreeMap::new(),
            last_event: None,
            locked: false,
            clients: BTreeMap::new(),
            overlays,
            focus: Focus::default(),
            components: Components::new(now),
            env: o.env,
            actions: Vec::new(),
            store: o.config,
            config: ConfigState { generation: 1, config, errors },
            theme_source: o.theme,
            theme_fingerprint,
            theme,
            invocations: 0,
            dropped: 0,
            resyncs: 0,
            effects: Vec::new(),
            virtual_session: o.virtual_session,
            virtual_restored: false,
        };
        core.actions = actions::list(&core.env, &core.overlays, &core.components);
        core
    }

    /// Effects produced since the last call, in order.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn desktop(&self) -> &DesktopState {
        &self.desktop
    }

    pub fn config(&self) -> &ConfigState {
        &self.config
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn has_lagging(&self) -> bool {
        self.clients.values().any(|c| c.lagging)
    }

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Connect { client, identity, events } => {
                let name = identity.program.clone();
                self.clients.insert(client, Client { identity, name, events, subs: BTreeSet::new(), seq: 0, lagging: false });
            }
            Msg::Disconnect { client } => self.disconnect(client),
            Msg::Request { client, id, op, reply } => self.request(client, id, op, reply),
            Msg::Desktop(u) => self.desktop_update(u),
            Msg::Locked(l) => self.set_locked(l),
            Msg::PanelOpened { id } => {
                if id.starts_with("nyx-shell:") {
                    return;
                }
                let before = self.overlays.any_restoring_visible();
                let changes = self.overlays.panel_opened(&id, &self.desktop, self.locked);
                self.apply_overlay_changes(before, changes);
            }
            Msg::BusOwner { bus, present } => self.bus_owner(bus, present),
            Msg::Dbus(up) => {
                let state = if up { ComponentState::Ready } else { ComponentState::Failed };
                let detail = (!up).then(|| "no session bus".to_string());
                if self.components.set_builtin(registry::DBUS, state, detail, Instant::now()) {
                    self.components_changed(vec![registry::DBUS.into()]);
                }
            }
            Msg::PointerResolved { overlay, monitor, reply: (id, reply), .. } => {
                let r = self.show_overlay(&overlay, monitor);
                answer(reply, id, r);
            }
            Msg::ConfigFiles => self.reload_config(),
            Msg::ThemeFiles => self.reload_theme(),
            Msg::ReadyTimeout => {
                if self.state == SessionState::Starting {
                    self.state = SessionState::Degraded;
                    self.effects.push(Effect::Ready);
                    self.session_changed();
                }
            }
            Msg::Shutdown => {
                self.state = SessionState::Stopping;
                self.session_changed();
                let ids: Vec<ClientId> = self.clients.keys().copied().collect();
                for c in ids {
                    self.send(c, &Event::Shutdown {});
                }
            }
        }
    }

    // ---------- Requests ----------

    fn request(&mut self, client: ClientId, id: u64, op: Op, reply: oneshot::Sender<Reply>) {
        let Some(c) = self.clients.get(&client) else { return };
        if let Some(cap) = op.capability()
            && !c.identity.can(cap)
        {
            return answer(reply, id, Err(err(ErrorCode::Forbidden, format!("{} may not use {}", c.identity.program, cap.name()))));
        }
        let now = Instant::now();
        let result: Result<Value, ErrorBody> = match op {
            Op::Hello(h) => {
                if h.max < MIN_PROTOCOL_VERSION || h.min > PROTOCOL_VERSION || h.min > h.max {
                    Err(err(
                        ErrorCode::UnsupportedVersion,
                        format!("the server speaks {MIN_PROTOCOL_VERSION} to {PROTOCOL_VERSION}, the client {} to {}", h.min, h.max),
                    ))
                } else {
                    let name = display_name(&h.client);
                    let c = self.clients.get_mut(&client).expect("checked");
                    if !name.is_empty() {
                        c.name = name;
                    }
                    Ok(to_value(&HelloReply {
                        protocol: PROTOCOL_VERSION.min(h.max),
                        server: format!("nyx-shell-core {VERSION}"),
                        session: self.session_id.clone(),
                        capabilities: c.identity.capabilities.iter().copied().collect(),
                        read_only: self.read_only,
                    }))
                }
            }
            Op::GetSessionStatus => Ok(to_value(&self.status())),
            Op::SubscribeSessionEvents => Ok(self.subscribe(client, Topic::Session, "status", to_value(&self.status()))),
            Op::GetDesktopSnapshot => Ok(to_value(&self.desktop)),
            Op::SubscribeDesktopChanges => Ok(self.subscribe(client, Topic::Desktop, "state", to_value(&self.desktop))),
            Op::RequestWorkspaceAction(a) => match self.can_dispatch().and_then(|_| compositor::workspace_dispatch(&a)) {
                Ok(d) => return self.effects.push(Effect::Dispatch { steps: vec![d], reply: Some((id, reply)) }),
                Err(e) => Err(e),
            },
            Op::RequestWindowAction(a) => match self.can_dispatch().and_then(|_| compositor::window_dispatch(&a, &self.desktop)) {
                Ok(steps) => return self.effects.push(Effect::Dispatch { steps, reply: Some((id, reply)) }),
                Err(e) => Err(e),
            },
            Op::RegisterOverlay { policy } => {
                let owner = self.owner(client);
                let r = self.overlays.register(policy, owner);
                if let Ok(s) = &r {
                    self.emit(Event::OverlayChanged { overlay: s.clone() });
                    self.actions_changed();
                }
                r.map(|s| json!({ "overlay": s }))
            }
            Op::UnregisterOverlay { id: overlay } => {
                let before = self.overlays.any_restoring_visible();
                match self.overlays.unregister(&overlay, client) {
                    Ok(()) => {
                        self.after_overlays(before, &[]);
                        self.emit(Event::OverlayRemoved { id: overlay });
                        self.actions_changed();
                        Ok(json!({}))
                    }
                    Err(e) => Err(e),
                }
            }
            Op::ShowOverlay { id: overlay, monitor } => {
                let pointer = self.overlays.policy(&overlay).is_some_and(|p| p.placement == Placement::Pointer);
                if pointer && monitor.is_none() && !self.locked {
                    return self.effects.push(Effect::ResolvePointer { client, overlay, reply: (id, reply) });
                }
                self.show_overlay(&overlay, monitor)
            }
            Op::HideOverlay { id: overlay } => self.overlay_op(|o, d, l| o.hide(&overlay, d, l), &overlay),
            Op::ToggleOverlay { id: overlay } => self.overlay_op(|o, d, l| o.toggle(&overlay, d, l), &overlay),
            Op::GetOverlayState => Ok(json!({ "overlays": self.overlays.all() })),
            Op::SubscribeOverlayState => {
                let all = to_value(&self.overlays.all());
                Ok(self.subscribe(client, Topic::Overlays, "overlays", all))
            }
            Op::RegisterComponent { manifest } => {
                let owner = self.owner(client);
                let r = self.components.register(manifest, owner, now);
                if let Ok(s) = &r {
                    self.components_changed(vec![s.manifest.id.clone()]);
                }
                r.map(|s| json!({ "component": s }))
            }
            Op::ReportReady { id: component } => self.report(client, &component, HealthReport::Ready, None),
            Op::ReportHealth { id: component, state, detail } => self.report(client, &component, state, detail),
            Op::GetComponentStatus { id: None } => Ok(json!({ "components": self.components.all(now) })),
            Op::GetComponentStatus { id: Some(c) } => match self.components.status(&c, now) {
                Some(s) => Ok(json!({ "components": [s] })),
                None => Err(err(ErrorCode::NotFound, format!("no component {c}"))),
            },
            Op::ListActions => {
                self.refresh_env();
                Ok(json!({ "actions": self.actions }))
            }
            Op::InvokeAction { id: action, args } => return self.invoke(id, &action, args, reply),
            Op::SubscribeActionAvailability => {
                self.refresh_env();
                let list = to_value(&self.actions);
                Ok(self.subscribe(client, Topic::Actions, "actions", list))
            }
            Op::GetShellConfig => Ok(to_value(&self.config)),
            Op::UpdateShellConfig { key, value } => self.store.update(&key, value).map(|()| {
                self.reload_config();
                json!({ "generation": self.config.generation })
            }),
            Op::ResetShellConfig { key } => self.store.reset(&key).map(|()| {
                self.reload_config();
                json!({ "generation": self.config.generation })
            }),
            Op::SubscribeConfigChanges => Ok(self.subscribe(client, Topic::Config, "config", to_value(&self.config))),
            Op::GetThemeGeneration => Ok(to_value(&self.theme)),
            Op::SubscribeThemeChanges => Ok(self.subscribe(client, Topic::Theme, "theme", to_value(&self.theme))),
            Op::Unsubscribe { topic } => {
                if let Some(c) = self.clients.get_mut(&client) {
                    c.subs.remove(&topic);
                }
                Ok(json!({}))
            }
        };
        answer(reply, id, result);
    }

    fn owner(&self, client: ClientId) -> Owner {
        let name = self.clients.get(&client).map_or_else(|| "unknown".into(), |c| c.name.clone());
        Owner::Client { id: client, name }
    }

    fn subscribe(&mut self, client: ClientId, topic: Topic, key: &str, snapshot: Value) -> Value {
        let c = self.clients.get_mut(&client).expect("request from a known client");
        c.subs.insert(topic);
        let mut m = serde_json::Map::new();
        m.insert("seq".into(), json!(c.seq));
        m.insert(key.into(), snapshot);
        Value::Object(m)
    }

    fn can_dispatch(&self) -> Result<(), ErrorBody> {
        if self.read_only {
            return Err(err(ErrorCode::ReadOnly, "Shell Core runs read-only"));
        }
        if !self.desktop.connected {
            return Err(err(ErrorCode::Unavailable, "Hyprland is unreachable"));
        }
        Ok(())
    }

    fn report(&mut self, client: ClientId, component: &str, h: HealthReport, detail: Option<String>) -> Result<Value, ErrorBody> {
        let s = self.components.report(component, client, h, detail, Instant::now())?;
        self.components_changed(vec![component.to_string()]);
        Ok(json!({ "component": s }))
    }

    fn invoke(&mut self, id: u64, action: &str, args: Option<Value>, reply: oneshot::Sender<Reply>) {
        if let Some(a) = &args
            && (!a.is_object() || serde_json::to_string(a).map_or(true, |s| s.len() > nyx_shell_proto::action::MAX_ARGS))
        {
            return answer(reply, id, Err(err(ErrorCode::Invalid, "args must be an object of at most 4 KiB")));
        }
        self.refresh_env();
        let route = match actions::resolve(action, &self.env, &self.overlays, &self.components) {
            Ok(r) => r,
            Err(e) => return answer(reply, id, Err(e)),
        };
        if route.external() && self.read_only {
            return answer(reply, id, Err(err(ErrorCode::ReadOnly, format!("{action} reaches outside Shell Core, which runs read-only"))));
        }
        let routed = json!(Routed { routed: route.label() });
        match route {
            Route::Panel1Toggle { bus } => self.effects.push(Effect::Bridge(BridgeCmd::Toggle { bus, reply: Some((id, reply)) })),
            Route::Switcher { line } => {
                let path = self.env.switcher_socket.clone().expect("checked by resolve");
                self.effects.push(Effect::Switcher { path, line, reply: (id, reply) })
            }
            Route::Launch { argv } => self.effects.push(Effect::Launch { argv, reply: (id, reply) }),
            Route::Overlay { id: overlay } => {
                let r = self.overlay_op(|o, d, l| o.toggle(&overlay, d, l), &overlay);
                answer(reply, id, r.map(|_| routed));
            }
            Route::Component { client: owner, .. } => {
                self.invocations += 1;
                let ev = Event::ActionInvoked { action: action.into(), args: args.unwrap_or_else(|| json!({})), invocation: self.invocations };
                self.send(owner, &ev);
                answer(reply, id, Ok(routed));
            }
        }
    }

    // ---------- Overlays and focus ----------

    fn show_overlay(&mut self, overlay: &str, monitor: Option<String>) -> Result<Value, ErrorBody> {
        self.overlay_op(|o, d, l| o.show(overlay, monitor, d, l), overlay)
    }

    fn overlay_op(
        &mut self,
        f: impl FnOnce(&mut Overlays, &DesktopState, bool) -> Result<Vec<Change>, ErrorBody>,
        overlay: &str,
    ) -> Result<Value, ErrorBody> {
        let before = self.overlays.any_restoring_visible();
        let changes = f(&mut self.overlays, &self.desktop, self.locked)?;
        self.apply_overlay_changes(before, changes);
        Ok(json!({ "overlay": self.overlays.get(overlay) }))
    }

    fn apply_overlay_changes(&mut self, before: bool, changes: Vec<Change>) {
        let mut announce = Vec::new();
        for c in &changes {
            if let Change::Changed(s) = c
                && s.visible
                && s.owner.starts_with("client:")
                && s.group.as_deref() == Some(PANELS)
            {
                announce.push(s.id.clone());
            }
        }
        self.after_overlays(before, &announce);
        for c in changes {
            match c {
                Change::Changed(s) => self.emit(Event::OverlayChanged { overlay: s }),
                Change::Removed(id) => self.emit(Event::OverlayRemoved { id }),
            }
        }
    }

    fn after_overlays(&mut self, before: bool, announce: &[String]) {
        for id in announce {
            if self.read_only {
                tracing::info!(overlay = %id, "would tell the GTK panels to close (read-only)");
            } else {
                self.effects.push(Effect::Bridge(BridgeCmd::Announce(format!("nyx-shell:{id}"))));
            }
        }
        let after = self.overlays.any_restoring_visible();
        if !before && after {
            self.focus.opened(&self.desktop);
        } else if before && !after {
            let debounce = Duration::from_millis(self.config.config.focus.debounce_ms);
            if let Some(window) = self.focus.closed(&self.desktop, self.locked, self.config.config.focus.restore, debounce, Instant::now()) {
                if self.read_only {
                    tracing::info!(window, "would give focus back (read-only)");
                } else {
                    if let Some(minimized) = self.desktop.window(&window).map(|w| w.minimized) {
                        let steps = nyx_desktop::actions::activate(&window, minimized, self.desktop.active.workspace.unwrap_or(0));
                        self.effects.push(Effect::Dispatch { steps, reply: None });
                    }
                }
            }
        }
    }

    // ---------- Desktop, lock, buses ----------

    fn desktop_update(&mut self, u: Update) {
        self.last_event = Some(Instant::now());
        self.raw = u.desktop;
        self.follow_panel_layers();
        self.rebuild();
        self.follow_virtual_monitors();
    }

    /// Bring each virtual monitor back to the workspace it showed last session, once,
    /// and from then on remember what each shows.
    fn follow_virtual_monitors(&mut self) {
        if self.read_only || !self.raw.connected || self.raw.hosts.is_empty() {
            return;
        }
        if !self.virtual_restored {
            self.virtual_restored = true;
            let plan = self.virtual_session.plan(&self.raw);
            if !plan.is_empty() {
                let mut steps: Vec<Dispatch> = plan
                    .iter()
                    .map(|id| Dispatch { lua: format!("hl.dsp.focus({{ workspace = \"{id}\" }})"), classic: format!("workspace {id}") })
                    .collect();
                if let Some(m) = self.raw.focused_monitor() {
                    steps.push(Dispatch { lua: format!("hl.dsp.focus({{ monitor = \"{}\" }})", m.name), classic: format!("focusmonitor {}", m.name) });
                }
                self.effects.push(Effect::Dispatch { steps, reply: None });
                return;
            }
        }
        if self.virtual_session.remember(&self.raw) {
            self.effects.push(Effect::SaveVirtualSession(self.virtual_session.clone()));
        }
    }

    /// A GTK panel is open while its layer surface is mapped: Hyprland reports it
    /// opening and closing, however the panel was shown or hidden. Only changes count,
    /// so a panel that announced itself before its surface mapped isn't closed again.
    fn follow_panel_layers(&mut self) {
        if !self.raw.connected {
            return;
        }
        for (id, namespaces) in PANEL_LAYERS {
            let open = namespaces.iter().any(|n| self.raw.layer_open(n));
            if self.panel_layers.insert(id, open) == Some(open) {
                continue;
            }
            let before = self.overlays.any_restoring_visible();
            let changes = self.overlays.panel_state(id, open, &self.desktop, self.locked);
            self.apply_overlay_changes(before, changes);
        }
    }

    /// Normalize the latest compositor state again and tell clients what changed.
    fn rebuild(&mut self) {
        let next = normalize(&self.raw, self.desktop.generation + 1);
        let delta = next.delta_from(&self.desktop);
        if delta.is_empty() {
            return;
        }
        let connected_changed = delta.connected.is_some() || self.state == SessionState::Starting;
        self.desktop = next;
        self.emit(Event::DesktopChanged(delta));
        self.focus.desktop_changed(&self.desktop);
        let before = self.overlays.any_restoring_visible();
        let changes = self.overlays.recompute(&self.desktop, self.locked);
        self.apply_overlay_changes(before, changes);
        if connected_changed {
            let connected = self.desktop.connected;
            let state = if connected { ComponentState::Ready } else { ComponentState::Degraded };
            let detail = (!connected).then(|| "unreachable; showing the last known state".to_string());
            if self.components.set_builtin(registry::HYPRLAND, state, detail, Instant::now()) {
                self.components_changed(vec![registry::HYPRLAND.into()]);
            }
            let was = self.state;
            self.state = if connected { SessionState::Ready } else { SessionState::Degraded };
            if was == SessionState::Starting {
                self.effects.push(Effect::Ready);
            }
            if was != self.state {
                self.session_changed();
            }
        }
    }

    fn set_locked(&mut self, locked: bool) {
        if self.locked == locked {
            return;
        }
        self.locked = locked;
        if locked {
            self.focus.locked();
        }
        let before = self.overlays.any_restoring_visible();
        let mut changes = self.overlays.set_locked_entry(locked, &self.desktop);
        changes.extend(self.overlays.recompute(&self.desktop, locked));
        self.apply_overlay_changes(before, changes);
        self.session_changed();
    }

    fn bus_owner(&mut self, bus: String, present: bool) {
        let changed = if present { self.env.buses.insert(bus.clone()) } else { self.env.buses.remove(&bus) };
        if !changed {
            return;
        }
        if let Some((id, _)) = PANEL1.iter().find(|(_, b)| *b == bus) {
            let (state, detail) = if present { (ComponentState::Ready, None) } else { (ComponentState::Stopped, Some("not running".to_string())) };
            if self.components.set_builtin(id, state, detail, Instant::now()) {
                self.components_changed(vec![(*id).into()]);
            }
            if !present {
                let before = self.overlays.any_restoring_visible();
                let changes = self.overlays.panel_state(id, false, &self.desktop, self.locked);
                self.apply_overlay_changes(before, changes);
            }
        }
        self.actions_changed();
    }

    fn refresh_env(&mut self) {
        let fresh = Environment::discover();
        self.env.switcher_socket = fresh.switcher_socket.or(self.env.switcher_socket.take());
        if fresh.glimpse.is_some() {
            self.env.glimpse = fresh.glimpse;
        }
        self.actions_changed();
    }

    // ---------- Settings and theme ----------

    fn reload_config(&mut self) {
        let (config, errors) = self.store.load();
        if config == self.config.config && errors == self.config.errors {
            return;
        }
        for e in &errors {
            tracing::warn!(error = %e, "shell settings");
        }
        self.config = ConfigState { generation: self.config.generation + 1, config, errors };
        let before = self.overlays.any_restoring_visible();
        let changes = self.overlays.set_overrides(self.config.config.overlays.clone(), &self.desktop, self.locked);
        self.apply_overlay_changes(before, changes);
        self.emit(Event::ConfigChanged { config: self.config.clone() });
    }

    fn reload_theme(&mut self) {
        let fp = self.theme_source.fingerprint();
        if fp == self.theme_fingerprint {
            return;
        }
        self.theme_fingerprint = fp;
        self.theme = self.theme_source.read(self.theme.generation + 1);
        tracing::info!(generation = self.theme.generation, id = %self.theme.id, "theme changed");
        self.emit(Event::ThemeChanged { theme: self.theme.clone() });
    }

    // ---------- Components, actions, session ----------

    fn components_changed(&mut self, ids: Vec<String>) {
        let now = Instant::now();
        let mut all: Vec<String> = Vec::new();
        for id in ids {
            let deps = self.components.dependents(&id);
            for i in std::iter::once(id).chain(deps) {
                if !all.contains(&i) {
                    all.push(i);
                }
            }
        }
        for id in all {
            if let Some(s) = self.components.status(&id, now) {
                self.emit(Event::ComponentChanged { component: s });
            }
        }
        self.actions_changed();
    }

    fn actions_changed(&mut self) {
        let list = actions::list(&self.env, &self.overlays, &self.components);
        if list != self.actions {
            self.actions = list.clone();
            self.emit(Event::ActionsChanged { actions: list });
        }
    }

    fn session_changed(&mut self) {
        let status = self.status();
        self.emit(Event::SessionChanged { status });
    }

    pub fn status(&self) -> SessionStatus {
        let now = Instant::now();
        SessionStatus {
            version: VERSION.into(),
            session: self.session_id.clone(),
            state: self.state,
            read_only: self.read_only,
            uptime_ms: now.duration_since(self.started).as_millis() as u64,
            locked: self.locked,
            compositor: CompositorStatus {
                connected: self.desktop.connected,
                generation: self.desktop.generation,
                last_event_ms_ago: self.last_event.map(|t| now.duration_since(t).as_millis() as u64),
            },
            ipc: IpcStatus { clients: self.clients.len(), dropped_events: self.dropped, resyncs: self.resyncs },
            components: self.components.all(now),
            overlays: self
                .overlays
                .all()
                .into_iter()
                .map(|o| OverlaySummary { id: o.id, owner: o.owner, visible: o.visible })
                .collect(),
            config: ConfigSummary { generation: self.config.generation, errors: self.config.errors.clone() },
            theme: ThemeSummary { generation: self.theme.generation },
        }
    }

    fn disconnect(&mut self, client: ClientId) {
        if self.clients.remove(&client).is_none() {
            return;
        }
        let before = self.overlays.any_restoring_visible();
        let changes = self.overlays.client_gone(client);
        self.apply_overlay_changes(before, changes);
        let ids = self.components.client_gone(client, Instant::now());
        if !ids.is_empty() {
            self.components_changed(ids);
        }
        self.actions_changed();
    }

    // ---------- Events ----------

    fn emit(&mut self, ev: Event) {
        let Some(topic) = ev.topic() else { return };
        let ids: Vec<ClientId> = self.clients.iter().filter(|(_, c)| c.subs.contains(&topic)).map(|(id, _)| *id).collect();
        if ids.is_empty() {
            return;
        }
        for id in ids {
            self.send(id, &ev);
        }
    }

    /// Queue an event on one connection. A full queue drops it and marks the client
    /// for a `Resync`, sent as soon as there's room.
    fn send(&mut self, client: ClientId, ev: &Event) {
        let Some(c) = self.clients.get_mut(&client) else { return };
        if c.lagging && !try_resync(c, &mut self.resyncs) {
            self.dropped += 1;
            return;
        }
        let line = frame(ev, c.seq + 1);
        match c.events.try_send(line) {
            Ok(()) => c.seq += 1,
            Err(mpsc::error::TrySendError::Full(_)) => {
                c.lagging = true;
                self.dropped += 1;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Try again to tell lagging clients to resynchronize.
    pub fn retry_resyncs(&mut self) {
        for c in self.clients.values_mut().filter(|c| c.lagging) {
            try_resync(c, &mut self.resyncs);
        }
    }
}

fn frame(ev: &Event, seq: u64) -> String {
    let mut line = serde_json::to_string(&EventFrame::new(ev, seq, now_us())).expect("events serialize");
    line.push('\n');
    line
}

fn try_resync(c: &mut Client, resyncs: &mut u64) -> bool {
    let ev = Event::Resync { topics: c.subs.iter().copied().collect() };
    match c.events.try_send(frame(&ev, c.seq + 1)) {
        Ok(()) => {
            c.seq += 1;
            c.lagging = false;
            *resyncs += 1;
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
