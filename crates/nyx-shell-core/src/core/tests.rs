use super::*;
use crate::compositor::Update;
use crate::peer::Identity;
use nyx_shell_proto::{Capability, Category, ComponentManifest, Hello, Keyboard, OverlayPolicy};
use std::sync::Arc;

struct Harness {
    core: Core,
    _dir: tempfile::TempDir,
    clients: BTreeMap<ClientId, mpsc::Receiver<String>>,
}

fn harness(read_only: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigStore {
        layers: nyx_config::Layers { defaults: dir.path().join("defaults"), system: dir.path().join("system"), user: dir.path().join("user") },
    };
    let theme = ThemeSource {
        suite_file: dir.path().join("user/theme.toml"),
        themes_dir: dir.path().join("user/themes"),
        omarchy_dir: dir.path().join("omarchy/current/theme"),
        snapshot_file: dir.path().join("run/nyx/theme/system.json"),
    };
    let core = Core::new(Options {
        read_only,
        config,
        theme,
        env: Environment::default(),
        session_id: "test".into(),
        virtual_session: Default::default(),
    });
    Harness { core, _dir: dir, clients: BTreeMap::new() }
}

impl Harness {
    fn connect(&mut self, id: ClientId, identity: Identity, queue: usize) {
        let (tx, rx) = mpsc::channel(queue);
        self.core.handle(Msg::Connect { client: id, identity, events: tx });
        self.clients.insert(id, rx);
    }

    fn call(&mut self, client: ClientId, op: Op) -> Result<Value, ErrorBody> {
        let (tx, mut rx) = oneshot::channel();
        self.core.handle(Msg::Request { client, id: 1, op, reply: tx });
        match rx.try_recv() {
            Ok(r) => match (r.ok, r.err) {
                (_, Some(e)) => Err(e),
                (Some(v), None) => Ok(v),
                _ => Ok(Value::Null),
            },
            Err(_) => Err(ErrorBody::new(ErrorCode::Internal, "deferred")),
        }
    }

    fn events(&mut self, client: ClientId) -> Vec<EventFrame> {
        let rx = self.clients.get_mut(&client).unwrap();
        let mut out = Vec::new();
        while let Ok(line) = rx.try_recv() {
            out.push(serde_json::from_str(&line).unwrap());
        }
        out
    }

    fn desktop(&mut self, d: nyx_desktop::Desktop) {
        self.core.handle(Msg::Desktop(Update { desktop: Arc::new(d) }));
    }
}

fn ui(program: &str) -> Identity {
    use Capability::*;
    Identity { program: program.into(), pid: None, capabilities: [DesktopRead, DesktopAct, OverlayManage, ComponentRegister, ActionInvoke].into_iter().collect() }
}

fn reader() -> Identity {
    Identity { program: "python3".into(), pid: None, capabilities: [Capability::DesktopRead].into_iter().collect() }
}

fn transient(id: &str) -> OverlayPolicy {
    OverlayPolicy { keyboard: Keyboard::Exclusive, ..OverlayPolicy::new(id, Category::Transient) }
}

#[test]
fn starts_ready_once_hyprland_answers() {
    let mut h = harness(false);
    assert_eq!(h.core.state(), SessionState::Starting);
    h.desktop(crate::state::tests::desktop());
    assert_eq!(h.core.state(), SessionState::Ready);
    assert!(h.core.take_effects().iter().any(|e| matches!(e, Effect::Ready)));
    let mut gone = crate::state::tests::desktop();
    gone.connected = false;
    h.desktop(gone);
    assert_eq!(h.core.state(), SessionState::Degraded);
}

#[test]
fn snapshot_then_deltas_with_sequence_numbers() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    let mut d = crate::state::tests::desktop();
    h.desktop(d.clone());
    let sub = h.call(1, Op::SubscribeDesktopChanges).unwrap();
    assert_eq!(sub["seq"], 0);
    let base = sub["state"]["generation"].as_u64().unwrap();
    d.apply(&nyx_desktop::event::Event::parse("activewindowv2>>b2"));
    h.desktop(d.clone());
    h.desktop(d.clone());
    let ev = h.events(1);
    assert_eq!(ev.len(), 1, "an update that changes nothing sends nothing");
    assert_eq!(ev[0].ev, "DesktopChanged");
    assert_eq!(ev[0].seq, 1);
    assert_eq!(ev[0].body["base"], base);
    let mut state: DesktopState = serde_json::from_value(sub["state"].clone()).unwrap();
    let delta: nyx_shell_proto::DesktopDelta = serde_json::from_value(ev[0].body.clone()).unwrap();
    state.apply(&delta).unwrap();
    assert_eq!(&state, h.core.desktop());
}

#[test]
fn capabilities_are_enforced() {
    let mut h = harness(false);
    h.connect(1, reader(), 8);
    h.connect(2, ui("quickshell"), 8);
    assert!(h.call(1, Op::GetDesktopSnapshot).is_ok());
    assert_eq!(h.call(1, Op::RegisterOverlay { policy: transient("x") }).unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(h.call(1, Op::InvokeAction { id: "shell.glimpse.toggle".into(), args: None }).unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(h.call(2, Op::UpdateShellConfig { key: "focus.restore".into(), value: json!(false) }).unwrap_err().code, ErrorCode::Forbidden);
}

#[test]
fn hello_negotiates() {
    let mut h = harness(true);
    h.connect(1, ui("quickshell"), 8);
    let r = h.call(1, Op::Hello(Hello { client: "nyx-shell-qs".into(), min: 1, max: 3 })).unwrap();
    assert_eq!(r["protocol"], 1);
    assert_eq!(r["read_only"], true);
    assert_eq!(h.call(1, Op::Hello(Hello { client: "x".into(), min: 2, max: 3 })).unwrap_err().code, ErrorCode::UnsupportedVersion);
    h.call(1, Op::RegisterOverlay { policy: transient("a") }).unwrap();
    assert_eq!(h.call(1, Op::GetOverlayState).unwrap()["overlays"].as_array().unwrap().iter().find(|o| o["id"] == "a").unwrap()["owner"], "client:nyx-shell-qs");
}

#[test]
fn read_only_never_acts() {
    let mut h = harness(true);
    h.connect(1, Identity::trusted("nyx-shellctl"), 8);
    h.desktop(crate::state::tests::desktop());
    h.core.take_effects();
    let window = nyx_shell_proto::WindowAction { window: "a1".into(), action: nyx_shell_proto::WindowActionKind::Close, workspace: None, monitor: None, follow: false };
    assert_eq!(h.call(1, Op::RequestWindowAction(window)).unwrap_err().code, ErrorCode::ReadOnly);
    h.core.env.buses.insert("io.github.design_nexus.nyx.ControlCenter".into());
    assert_eq!(h.call(1, Op::InvokeAction { id: "shell.control-center.toggle".into(), args: None }).unwrap_err().code, ErrorCode::ReadOnly);
    // Overlays still work, but the GTK panels aren't told and focus isn't moved.
    h.call(1, Op::RegisterOverlay { policy: transient("a") }).unwrap();
    h.call(1, Op::ShowOverlay { id: "a".into(), monitor: None }).unwrap();
    h.call(1, Op::HideOverlay { id: "a".into() }).unwrap();
    assert!(h.core.take_effects().is_empty());
}

#[test]
fn window_actions_become_dispatches() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 8);
    h.desktop(crate::state::tests::desktop());
    h.core.take_effects();
    let (tx, _rx) = oneshot::channel();
    let window = nyx_shell_proto::WindowAction { window: "b2".into(), action: nyx_shell_proto::WindowActionKind::Focus, workspace: None, monitor: None, follow: false };
    h.core.handle(Msg::Request { client: 1, id: 9, op: Op::RequestWindowAction(window), reply: tx });
    let effects = h.core.take_effects();
    let Some(Effect::Dispatch { steps, reply: Some((9, _)) }) = effects.first() else { panic!("a dispatch") };
    assert_eq!(steps[0].classic, "focuswindow address:0xb2");
}

#[test]
fn focus_goes_back_after_a_transient_overlay() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    let mut d = crate::state::tests::desktop();
    h.desktop(d.clone());
    h.call(1, Op::RegisterOverlay { policy: transient("a") }).unwrap();
    h.call(1, Op::ShowOverlay { id: "a".into(), monitor: None }).unwrap();
    // The overlay took the keyboard: Hyprland reports no active window.
    d.apply(&nyx_desktop::event::Event::parse("activewindowv2>>"));
    h.desktop(d.clone());
    h.core.take_effects();
    h.call(1, Op::HideOverlay { id: "a".into() }).unwrap();
    let effects = h.core.take_effects();
    let restore = effects.iter().find_map(|e| match e {
        Effect::Dispatch { steps, reply: None } => Some(steps[0].classic.clone()),
        _ => None,
    });
    assert_eq!(restore.as_deref(), Some("focuswindow address:0xa1"));
}

#[test]
fn locking_hides_overlays_and_stops_focus_restore() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    h.desktop(crate::state::tests::desktop());
    h.call(1, Op::SubscribeOverlayState).unwrap();
    h.call(1, Op::SubscribeSessionEvents).unwrap();
    h.call(1, Op::RegisterOverlay { policy: transient("a") }).unwrap();
    h.call(1, Op::ShowOverlay { id: "a".into(), monitor: None }).unwrap();
    h.events(1);
    h.core.take_effects();
    h.core.handle(Msg::Locked(true));
    let ev = h.events(1);
    assert!(ev.iter().any(|e| e.ev == "OverlayChanged" && e.body["overlay"]["id"] == "a" && e.body["overlay"]["visible"] == false));
    assert!(ev.iter().any(|e| e.ev == "OverlayChanged" && e.body["overlay"]["id"] == "lock-screen" && e.body["overlay"]["visible"] == true));
    assert!(ev.iter().any(|e| e.ev == "SessionChanged" && e.body["status"]["locked"] == true));
    assert!(!h.core.take_effects().iter().any(|e| matches!(e, Effect::Dispatch { .. })), "no focus restore while locked");
    assert_eq!(h.call(1, Op::ShowOverlay { id: "a".into(), monitor: None }).unwrap_err().code, ErrorCode::Locked);
}

#[test]
fn gtk_panels_and_client_overlays_exclude_each_other() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    h.desktop(crate::state::tests::desktop());
    h.call(1, Op::RegisterOverlay { policy: transient("inspector") }).unwrap();
    h.call(1, Op::ShowOverlay { id: "inspector".into(), monitor: None }).unwrap();
    let effects = h.core.take_effects();
    assert!(effects.iter().any(|e| matches!(e, Effect::Bridge(BridgeCmd::Announce(id)) if id == "nyx-shell:inspector")));
    h.core.handle(Msg::PanelOpened { id: "control-center".into() });
    let o = h.call(1, Op::GetOverlayState).unwrap();
    let find = |id: &str| o["overlays"].as_array().unwrap().iter().find(|x| x["id"] == id).unwrap().clone();
    assert_eq!(find("inspector")["visible"], false);
    assert_eq!(find("control-center")["visible"], true);
    // Our own announcements are ignored.
    h.core.handle(Msg::PanelOpened { id: "nyx-shell:inspector".into() });
    assert!(h.core.overlays.get("control-center").unwrap().visible);
}

#[test]
fn panels_follow_their_layer_surfaces() {
    let mut h = harness(false);
    let visible = |h: &Harness, id: &str| h.core.overlays.get(id).unwrap().visible;
    let mut d = crate::state::tests::desktop();
    h.desktop(d.clone());
    // Announced before its surface maps: the layer catching up doesn't close it.
    h.core.handle(Msg::PanelOpened { id: "control-center".into() });
    h.desktop(d.clone());
    assert!(visible(&h, "control-center"));
    d.layers.insert("nyx-control-center".into(), 1);
    h.desktop(d.clone());
    assert!(visible(&h, "control-center"));
    // Closed however it was hidden (Escape, a click outside): its surface unmaps.
    d.layers.remove("nyx-control-center");
    h.desktop(d.clone());
    assert!(!visible(&h, "control-center"));
    // Shown without announcing itself, and the switcher's overview counts as the switcher.
    d.layers.insert("nyx-overview".into(), 1);
    h.desktop(d.clone());
    assert!(visible(&h, "switcher"));
    // Hyprland unreachable: the last known state stays.
    let mut gone = d.clone();
    gone.connected = false;
    gone.layers.clear();
    h.desktop(gone);
    assert!(visible(&h, "switcher"));
}

#[test]
fn panel_buses_drive_components_and_actions() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    h.call(1, Op::SubscribeActionAvailability).unwrap();
    h.core.handle(Msg::Dbus(true));
    h.core.handle(Msg::BusOwner { bus: "io.github.design_nexus.nyx.ControlCenter".into(), present: true });
    let ev = h.events(1);
    let actions = ev.iter().rev().find(|e| e.ev == "ActionsChanged").unwrap();
    let cc = actions.body["actions"].as_array().unwrap().iter().find(|a| a["id"] == "shell.control-center.toggle").unwrap();
    assert_eq!(cc["available"], true);
    let s = h.call(1, Op::GetComponentStatus { id: Some("control-center".into()) }).unwrap();
    assert_eq!(s["components"][0]["state"], "ready");
    h.call(1, Op::InvokeAction { id: "shell.control-center.toggle".into(), args: None }).unwrap_err();
    assert!(h.core.take_effects().iter().any(|e| matches!(e, Effect::Bridge(BridgeCmd::Toggle { bus, .. }) if bus.ends_with("ControlCenter"))));
}

#[test]
fn component_actions_reach_their_owner() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    h.connect(2, Identity::trusted("nyx-shellctl"), 64);
    let manifest = ComponentManifest { id: "nyx.osd-sample".into(), actions: vec!["shell.osd-sample.show".into()], ..Default::default() };
    h.call(1, Op::RegisterComponent { manifest }).unwrap();
    assert_eq!(h.call(2, Op::InvokeAction { id: "shell.osd-sample.show".into(), args: None }).unwrap_err().code, ErrorCode::Unavailable);
    h.call(1, Op::ReportReady { id: "nyx.osd-sample".into() }).unwrap();
    let r = h.call(2, Op::InvokeAction { id: "shell.osd-sample.show".into(), args: Some(json!({"level": 40})) }).unwrap();
    assert_eq!(r["routed"], "component:nyx.osd-sample");
    let ev = h.events(1);
    let inv = ev.iter().find(|e| e.ev == "ActionInvoked").expect("sent unsubscribed");
    assert_eq!(inv.body["args"]["level"], 40);
    assert_eq!(h.call(2, Op::InvokeAction { id: "shell.osd-sample.show".into(), args: Some(json!([1])) }).unwrap_err().code, ErrorCode::Invalid);
    let big = json!({ "x": "y".repeat(5000) });
    assert_eq!(h.call(2, Op::InvokeAction { id: "shell.osd-sample.show".into(), args: Some(big) }).unwrap_err().code, ErrorCode::Invalid);
}

#[test]
fn a_client_that_leaves_takes_its_overlays_and_fails_its_components() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 64);
    h.connect(2, reader(), 64);
    h.call(2, Op::SubscribeOverlayState).unwrap();
    h.call(2, Op::SubscribeSessionEvents).unwrap();
    h.call(1, Op::RegisterComponent { manifest: ComponentManifest { id: "x".into(), ..Default::default() } }).unwrap();
    h.call(1, Op::ReportReady { id: "x".into() }).unwrap();
    h.call(1, Op::RegisterOverlay { policy: transient("x") }).unwrap();
    h.events(2);
    h.core.handle(Msg::Disconnect { client: 1 });
    let ev = h.events(2);
    assert!(ev.iter().any(|e| e.ev == "OverlayRemoved" && e.body["id"] == "x"));
    assert!(ev.iter().any(|e| e.ev == "ComponentChanged" && e.body["component"]["state"] == "failed"));
}

#[test]
fn slow_clients_get_a_resync() {
    let mut h = harness(false);
    h.connect(1, ui("quickshell"), 2);
    h.call(1, Op::SubscribeDesktopChanges).unwrap();
    let mut d = crate::state::tests::desktop();
    for i in 0..6 {
        d.windows[0].title = format!("title {i}");
        h.desktop(d.clone());
    }
    assert!(h.core.has_lagging());
    let first = h.events(1);
    assert_eq!(first.len(), 2, "the queue held two");
    h.core.retry_resyncs();
    let next = h.events(1);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].ev, "Resync");
    assert_eq!(next[0].body["topics"], json!(["desktop"]));
    assert_eq!(next[0].seq, 3, "no gap in sequence numbers");
    assert!(!h.core.has_lagging());
    assert!(h.core.status().ipc.dropped_events >= 4);
}

#[test]
fn settings_update_and_reach_overlays() {
    let mut h = harness(false);
    h.connect(1, Identity::trusted("nyx-shellctl"), 64);
    h.call(1, Op::SubscribeConfigChanges).unwrap();
    h.call(1, Op::RegisterOverlay { policy: transient("a") }).unwrap();
    let r = h.call(1, Op::UpdateShellConfig { key: "overlays".into(), value: json!({"a": {"fullscreen": "block"}}) }).unwrap();
    assert_eq!(r["generation"], 2);
    assert_eq!(h.call(1, Op::GetOverlayState).unwrap()["overlays"].as_array().unwrap().iter().find(|o| o["id"] == "a").unwrap()["fullscreen"], "block");
    assert!(h.events(1).iter().any(|e| e.ev == "ConfigChanged" && e.body["config"]["generation"] == 2));
    assert_eq!(h.call(1, Op::UpdateShellConfig { key: "ipc.max_queue".into(), value: json!(0) }).unwrap_err().code, ErrorCode::Invalid);
    h.call(1, Op::ResetShellConfig { key: "overlays".into() }).unwrap();
    assert_eq!(h.core.config().generation, 3);
}

#[test]
fn theme_generation_moves_only_on_change() {
    let mut h = harness(false);
    h.connect(1, reader(), 64);
    let g = h.call(1, Op::SubscribeThemeChanges).unwrap()["theme"]["generation"].as_u64().unwrap();
    h.core.handle(Msg::ThemeFiles);
    assert!(h.events(1).is_empty(), "nothing changed");
    let suite = h.core.theme_source.suite_file.clone();
    std::fs::create_dir_all(suite.parent().unwrap()).unwrap();
    std::fs::write(&suite, "mode = \"theme\"\ntheme = \"nyx-light\"\n").unwrap();
    h.core.handle(Msg::ThemeFiles);
    let ev = h.events(1);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].body["theme"]["generation"], g + 1);
    assert_eq!(ev[0].body["theme"]["mode"], "theme");
}

#[test]
fn shutdown_tells_everyone() {
    let mut h = harness(false);
    h.connect(1, reader(), 8);
    h.core.handle(Msg::Shutdown);
    assert!(h.events(1).iter().any(|e| e.ev == "Shutdown"));
    assert_eq!(h.core.state(), SessionState::Stopping);
}

#[test]
fn virtual_monitors_come_back_then_are_remembered() {
    let mut h = harness(false);
    h.core.virtual_session.active.insert("S|O|H".into(), vec![1, 3]);

    let region = |name: &str, active: i64| nyx_desktop::Monitor {
        id: 0,
        name: name.into(),
        description: String::new(),
        make: "S".into(),
        model: "O".into(),
        serial: String::new(),
        width: 1920,
        height: 1080,
        x: 0,
        y: 0,
        scale: 1.0,
        transform: 0,
        refresh_rate: 60.0,
        focused: name.ends_with("v1"),
        disabled: false,
        active_workspace: active,
        special_workspace: None,
        reserved: [0; 4],
    };
    let workspace = |id: i64, name: &str, monitor: &str| nyx_desktop::Workspace {
        id,
        name: name.into(),
        monitor: monitor.into(),
        windows: 0,
        has_fullscreen: false,
        tiled_layout: String::new(),
    };
    let mut d = nyx_desktop::Desktop { connected: true, ..Default::default() };
    d.monitors = vec![region("DP-3.v1", 1), region("DP-3.v2", 201)];
    d.hosts = vec![nyx_desktop::VirtualHost { name: "DP-3".into(), stable_id: "S|O|H".into(), regions: vec!["DP-3.v1".into(), "DP-3.v2".into()], ..Default::default() }];
    d.regions.insert("DP-3.v2".into(), nyx_desktop::VirtualRegion { host: "DP-3".into(), index: 2, count: 2, workspace_base: 200, ..Default::default() });
    d.workspaces = vec![workspace(1, "1", "DP-3.v1"), workspace(201, "1", "DP-3.v2"), workspace(203, "3", "DP-3.v2")];

    h.desktop(d.clone());
    let effects = h.core.take_effects();
    let steps: Vec<&str> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Dispatch { steps, .. } => Some(steps.iter().map(|s| s.classic.as_str())),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(steps, vec!["workspace 203", "focusmonitor DP-3.v1"], "the right region goes back to its 3, then focus returns");
    assert!(!effects.iter().any(|e| matches!(e, Effect::SaveVirtualSession(_))), "nothing is remembered before that");

    d.monitors[1].active_workspace = 203;
    h.desktop(d.clone());
    assert!(!h.core.take_effects().iter().any(|e| matches!(e, Effect::SaveVirtualSession(_))), "it shows what was remembered");

    d.monitors[0].active_workspace = 2;
    d.workspaces.push(workspace(2, "2", "DP-3.v1"));
    h.desktop(d);
    let saved = h.core.take_effects().into_iter().find_map(|e| match e {
        Effect::SaveVirtualSession(s) => Some(s),
        _ => None,
    });
    assert_eq!(saved.unwrap().active["S|O|H"], vec![2, 3]);
}
