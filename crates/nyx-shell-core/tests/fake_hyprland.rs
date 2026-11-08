//! Shell Core end to end: a real socket, a pretend Hyprland answering on its sockets,
//! and clients speaking the protocol. One test function, because the Hyprland sockets
//! are found through process-wide environment variables.

use nyx_shell_core::config::ConfigStore;
use nyx_shell_core::peer::Trust;
use nyx_shell_core::server::{self, Outcome, ServerOptions};
use nyx_shell_core::theme::ThemeSource;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod common;

use common::{Client, Fake, World, client, has, monitor, workspace};

// ---------- The scenario ----------

fn world() -> World {
    World {
        monitors: vec![monitor(0, "eDP-1", 1, 60.0, true)],
        workspaces: vec![workspace(1, "eDP-1")],
        clients: vec![client("a1", "firefox", 1, 0), client("b2", "kitty", 1, 1)],
        active: "a1".into(),
    }
}

#[test]
fn end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let runtime = tmp.path().join("run");
    let hypr_dir = runtime.join("hypr/sig");
    // SAFETY: set before any thread starts; this file has one test.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "sig");
        std::env::set_var("NYX_SWITCHER_SOCKET", tmp.path().join("no-switcher.sock"));
    }
    let fake = Fake::start(&hypr_dir, world());

    let config = ConfigStore {
        layers: nyx_config_layers(tmp.path()),
    };
    std::fs::create_dir_all(&config.layers.defaults).unwrap();
    std::fs::write(config.layers.defaults.join("shell.toml"), "[ipc]\nmax_queue = 16\n").unwrap();
    let theme = ThemeSource {
        suite_file: tmp.path().join("cfg/nyx/theme.toml"),
        themes_dir: tmp.path().join("cfg/nyx/themes"),
        omarchy_dir: tmp.path().join("state/omarchy/current/theme"),
    };
    let me = std::env::current_exe().unwrap();
    let trust = Trust { own_dir: PathBuf::from("/nonexistent"), trusted_dirs: vec![], ui_runtimes: vec![me] };
    let socket = runtime.join("nyx-shell/core.sock");
    let options = |socket: PathBuf| ServerOptions {
        socket,
        read_only: false,
        hyprland: true,
        dbus: false,
        watch_files: false,
        ready_timeout: Duration::from_secs(2),
        config: config.clone(),
        theme: theme.clone(),
        compositor: Arc::new(nyx_shell_core::compositor::Hyprland),
        trust: Some(trust.clone()),
    };

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let opts = options(socket.clone());
    let server = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(server::run(opts, async { let _ = stop_rx.await; }, Some(started_tx))).unwrap()
    });
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let _inbox = rt.block_on(started_rx).expect("server started");

    // A second instance leaves the first alone.
    let second = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    assert_eq!(second.block_on(server::run(options(socket.clone()), async {}, None)).unwrap(), Outcome::AlreadyRunning);
    let mode = std::fs::metadata(&socket).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap();
    assert_eq!(mode, 0o600);

    fake.wait_subscribed();

    // Snapshot.
    let mut c = Client::connect(&socket);
    let hello = c.call("Hello", json!({"client": "e2e", "min": 1, "max": 1})).unwrap();
    assert!(hello["capabilities"].as_array().unwrap().contains(&json!("overlay.manage")));
    let sub = {
        let t = Instant::now();
        loop {
            let s = c.call("SubscribeDesktopChanges", json!({})).unwrap();
            if s["state"]["connected"] == true {
                break s;
            }
            assert!(t.elapsed() < Duration::from_secs(5));
            c.call("Unsubscribe", json!({"topic": "desktop"})).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let state = &sub["state"];
    assert_eq!(state["windows"].as_array().unwrap().len(), 2);
    assert_eq!(state["monitors"][0]["refresh_rate"], 60.0);
    assert_eq!(state["active"]["window"], "a1");
    let mut generation = state["generation"].as_u64().unwrap();
    let base_seq = sub["seq"].as_u64().unwrap();
    c.events.retain(|e| e["seq"].as_u64().unwrap() > base_seq);

    // A window opens.
    fake.change(|w| w.clients.insert(0, client("c3", "nautilus", 1, 0)));
    fake.send("openwindow>>c3,1,nautilus,Files\n");
    let d = c.wait("DesktopChanged", |b| has(&b["windows"]["upsert"], "address", "c3"));
    assert_eq!(d["body"]["base"], generation);
    generation = d["body"]["generation"].as_u64().unwrap();

    // A workspace switch.
    fake.change(|w| {
        w.workspaces.push(workspace(2, "eDP-1"));
        w.monitors[0] = monitor(0, "eDP-1", 2, 60.0, true);
    });
    fake.send("createworkspacev2>>2,2\nworkspacev2>>2,2\n");
    let d = c.wait("DesktopChanged", |b| b["active"]["workspace"] == 2);
    assert!(d["body"]["generation"].as_u64().unwrap() > generation);

    // A monitor is plugged in, then out.
    fake.change(|w| w.monitors.push(monitor(1, "DP-3", 3, 144.0, false)));
    fake.send("monitoraddedv2>>1,DP-3,Dell\n");
    let d = c.wait("DesktopChanged", |b| has(&b["monitors"]["upsert"], "name", "DP-3"));
    let dp3 = d["body"]["monitors"]["upsert"].as_array().unwrap().iter().find(|m| m["name"] == "DP-3").unwrap().clone();
    assert_eq!(dp3["refresh_rate"], 144.0);
    fake.change(|w| w.monitors.retain(|m| m["name"] != "DP-3"));
    fake.send("monitorremovedv2>>1,DP-3,Dell\n");
    c.wait("DesktopChanged", |b| b["monitors"]["remove"] == json!(["DP-3"]));

    // A mode and scale change: Hyprland sends no event of its own, NyxOS's hook does.
    fake.change(|w| {
        w.monitors[0]["width"] = json!(2560);
        w.monitors[0]["height"] = json!(1440);
        w.monitors[0]["scale"] = json!(1.25);
    });
    fake.send("custom>>nyx-shell-monitors\n");
    c.wait("DesktopChanged", |b| {
        b["monitors"]["upsert"].as_array().is_some_and(|a| a.iter().any(|m| m["name"] == "eDP-1" && m["width"] == 2560 && m["logical_width"] == 2048))
    });

    // Window and workspace actions reach Hyprland, checked against the state.
    c.call("RequestWindowAction", json!({"window": "c3", "action": "focus"})).unwrap();
    c.call("RequestWorkspaceAction", json!({"action": "focus", "workspace": "1"})).unwrap();
    let sent = fake.dispatches.lock().unwrap().clone();
    assert_eq!(sent[0], "hl.dsp.focus({ window = \"address:0xc3\" })");
    assert!(sent.contains(&"hl.dsp.focus({ workspace = \"1\" })".to_string()));
    assert_eq!(c.call("RequestWindowAction", json!({"window": "ffff", "action": "close"})).unwrap_err()["code"], "not_found");
    assert_eq!(c.call("RequestWorkspaceAction", json!({"action": "focus", "workspace": "1\" })"})).unwrap_err()["code"], "invalid");

    // Malformed and refused requests.
    c.send_line("this is not json");
    assert_eq!(c.reply()["err"]["code"], "bad_request");
    c.send_line(r#"{"v":1,"id":77,"op":"RunCommand","body":{"cmd":"rm -rf /"}}"#);
    let r = c.reply();
    assert_eq!((r["id"].clone(), r["err"]["code"].clone()), (json!(77), json!("unknown_op")));
    c.send_line(r#"{"v":9,"id":78,"op":"GetSessionStatus"}"#);
    assert_eq!(c.reply()["err"]["code"], "unsupported_version");
    c.send_line(r#"{"v":1,"id":79,"op":"ShowOverlay","body":{"id":5}}"#);
    assert_eq!(c.reply()["err"]["code"], "bad_request");
    assert_eq!(c.call("UpdateShellConfig", json!({"key": "focus.restore", "value": false})).unwrap_err()["code"], "forbidden");

    // Overlays across clients; the owner leaving takes its overlay.
    let mut watcher = Client::connect(&socket);
    watcher.call("SubscribeOverlayState", json!({})).unwrap();
    let mut owner = Client::connect(&socket);
    owner.call("RegisterOverlay", json!({"policy": {"id": "nyx.test", "keyboard": "exclusive"}})).unwrap();
    owner.call("RegisterComponent", json!({"manifest": {"id": "nyx.test", "overlays": ["nyx.test"]}})).unwrap();
    owner.call("ReportReady", json!({"id": "nyx.test"})).unwrap();
    let shown = c.call("ShowOverlay", json!({"id": "nyx.test"})).unwrap();
    assert_eq!(shown["overlay"]["visible"], true);
    assert_eq!(shown["overlay"]["monitor"], "eDP-1");
    watcher.wait("OverlayChanged", |b| b["overlay"]["id"] == "nyx.test" && b["overlay"]["visible"] == true);
    drop(owner);
    watcher.wait("OverlayRemoved", |b| b["id"] == "nyx.test");
    let comps = c.call("GetComponentStatus", json!({"id": "nyx.test"})).unwrap();
    assert_eq!(comps["components"][0]["state"], "failed");
    // The runtime comes back and registers again.
    let mut owner = Client::connect(&socket);
    owner.call("RegisterComponent", json!({"manifest": {"id": "nyx.test"}})).unwrap();
    owner.call("ReportReady", json!({"id": "nyx.test"})).unwrap();
    drop(owner);

    // A slow client: its queue fills, it gets a Resync, the others are unaffected.
    let mut slow = Client::connect(&socket);
    slow.call("SubscribeDesktopChanges", json!({})).unwrap();
    for i in 0..1500 {
        fake.send(&format!("windowtitlev2>>c3,{i} {}\n", "x".repeat(500)));
        if i % 50 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_micros(300));
    }
    let status = c.call("GetSessionStatus", json!({})).unwrap();
    assert!(status["ipc"]["dropped_events"].as_u64().unwrap() > 0, "{status}");
    slow.wait("Resync", |b| b["topics"] == json!(["desktop"]));
    c.events.clear();

    // A reconnecting client gets a snapshot equal to what everyone else holds.
    let mut again = Client::connect(&socket);
    let snap = again.call("SubscribeDesktopChanges", json!({})).unwrap()["state"].clone();
    assert_eq!(snap, c.call("GetDesktopSnapshot", json!({})).unwrap());

    // Hyprland restarts: disconnected, then a fresh snapshot from the new instance.
    fake.shut();
    again.wait("DesktopChanged", |b| b["connected"] == false);
    assert_eq!(c.call("GetSessionStatus", json!({})).unwrap()["state"], "degraded");
    let mut w2 = world();
    w2.clients = vec![client("d4", "foot", 1, 0)];
    w2.active = "d4".into();
    let fake2 = Fake::start(&hypr_dir, w2);
    fake2.wait_subscribed();
    let d = again.wait("DesktopChanged", |b| b["connected"] == true);
    assert!(has(&d["body"]["windows"]["upsert"], "address", "d4"));
    assert_eq!(c.call("GetSessionStatus", json!({})).unwrap()["state"], "ready");

    // A line over 1 MiB is refused and the connection closed.
    let mut big = Client::connect(&socket);
    let huge = format!("{{\"v\":1,\"id\":1,\"op\":\"Hello\",\"body\":{{\"client\":\"{}\"}}}}", "a".repeat(1_100_000));
    let _ = big.writer.write_all(huge.as_bytes());
    assert_eq!(big.reply()["err"]["code"], "too_large");

    // Shutdown reaches clients and removes the socket.
    let _ = stop_tx.send(());
    again.wait("Shutdown", |_| true);
    assert_eq!(server.join().unwrap(), Outcome::Stopped);
    assert!(!socket.exists());
    fake2.shut();
}

fn nyx_config_layers(root: &Path) -> nyx_config::Layers {
    nyx_config::Layers { defaults: root.join("etc/defaults"), system: root.join("etc/overrides"), user: root.join("cfg/nyx") }
}
