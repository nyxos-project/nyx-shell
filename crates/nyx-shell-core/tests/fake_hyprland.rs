//! Shell Core end to end: a real socket, a pretend Hyprland answering on its sockets,
//! and clients speaking the protocol. One test function, because the Hyprland sockets
//! are found through process-wide environment variables.

use nyx_shell_core::config::ConfigStore;
use nyx_shell_core::peer::Trust;
use nyx_shell_core::server::{self, Outcome, ServerOptions};
use nyx_shell_core::theme::ThemeSource;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------- A pretend Hyprland ----------

#[derive(Clone)]
struct World {
    monitors: Vec<Value>,
    workspaces: Vec<Value>,
    clients: Vec<Value>,
    active: String,
}

fn monitor(id: i64, name: &str, ws: i64, refresh: f64, focused: bool) -> Value {
    json!({"id": id, "name": name, "description": format!("{name} screen"), "make": "Maker", "model": name, "serial": "",
           "width": 1920, "height": 1080, "x": id * 1920, "y": 0, "scale": 1.0, "transform": 0, "focused": focused,
           "disabled": false, "activeWorkspace": {"id": ws, "name": ws.to_string()}, "specialWorkspace": {"id": 0, "name": ""},
           "reserved": [0, 0, 0, 0], "refreshRate": refresh})
}

fn workspace(id: i64, monitor: &str) -> Value {
    json!({"id": id, "name": id.to_string(), "monitor": monitor, "windows": 1, "hasfullscreen": false, "tiledLayout": "dwindle"})
}

fn client(addr: &str, class: &str, ws: i64, order: i32) -> Value {
    json!({"address": format!("0x{addr}"), "stableId": format!("{order}"), "mapped": true, "hidden": false, "at": [0, 0],
           "size": [800, 600], "workspace": {"id": ws, "name": ws.to_string()}, "floating": false, "pinned": false,
           "monitor": 0, "class": class, "title": format!("{class} window"), "initialClass": class, "pid": 1,
           "fullscreen": 0, "focusHistoryID": order})
}

struct Fake {
    dir: PathBuf,
    world: Arc<Mutex<World>>,
    events: Arc<Mutex<Vec<UnixStream>>>,
    dispatches: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn start(dir: &Path, world: World) -> Fake {
        std::fs::create_dir_all(dir).unwrap();
        let commands = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        let event_listener = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
        let world = Arc::new(Mutex::new(world));
        let events = Arc::new(Mutex::new(Vec::new()));
        let dispatches = Arc::new(Mutex::new(Vec::new()));
        {
            let (world, dispatches) = (world.clone(), dispatches.clone());
            std::thread::spawn(move || {
                for mut s in commands.incoming().flatten() {
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let w = world.lock().unwrap().clone();
                    let reply = match req.as_str() {
                        "j/monitors all" | "j/monitors" => serde_json::to_string(&w.monitors).unwrap(),
                        "j/workspaces" => serde_json::to_string(&w.workspaces).unwrap(),
                        "j/clients" => serde_json::to_string(&w.clients).unwrap(),
                        "j/activewindow" => json!({"address": format!("0x{}", w.active)}).to_string(),
                        "j/cursorpos" => json!({"x": 10, "y": 10}).to_string(),
                        d if d.starts_with("dispatch ") => {
                            dispatches.lock().unwrap().push(d["dispatch ".len()..].to_string());
                            "ok".into()
                        }
                        _ => "unknown request".into(),
                    };
                    let _ = s.write_all(reply.as_bytes());
                }
            });
        }
        {
            let events = events.clone();
            std::thread::spawn(move || {
                for s in event_listener.incoming().flatten() {
                    events.lock().unwrap().push(s);
                }
            });
        }
        Fake { dir: dir.to_path_buf(), world, events, dispatches }
    }

    fn wait_subscribed(&self) {
        let t = Instant::now();
        while self.events.lock().unwrap().is_empty() {
            assert!(t.elapsed() < Duration::from_secs(5), "Shell Core never subscribed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&self, lines: &str) {
        for s in self.events.lock().unwrap().iter_mut() {
            let _ = s.write_all(lines.as_bytes());
        }
    }

    fn change(&self, f: impl FnOnce(&mut World)) {
        f(&mut self.world.lock().unwrap());
    }

    fn shut(&self) {
        let _ = std::fs::remove_file(self.dir.join(".socket.sock"));
        let _ = std::fs::remove_file(self.dir.join(".socket2.sock"));
        for s in self.events.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

// ---------- A protocol client ----------

struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: u64,
    events: Vec<Value>,
}

impl Client {
    fn connect(socket: &Path) -> Client {
        let s = UnixStream::connect(socket).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let writer = s.try_clone().unwrap();
        Client { reader: BufReader::new(s), writer, next: 0, events: Vec::new() }
    }

    fn send_line(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("a line before the timeout");
        assert!(n > 0, "connection closed");
        serde_json::from_str(&line).unwrap()
    }

    /// The reply to the last request, keeping events that arrive first.
    fn reply(&mut self) -> Value {
        loop {
            let v = self.read();
            if v.get("ev").is_some() {
                self.events.push(v);
                continue;
            }
            return v;
        }
    }

    fn call(&mut self, op: &str, body: Value) -> Result<Value, Value> {
        self.next += 1;
        let req = json!({"v": 1, "id": self.next, "op": op, "body": body});
        self.send_line(&req.to_string());
        let r = self.reply();
        assert_eq!(r["id"], self.next);
        match r.get("err") {
            Some(e) => Err(e.clone()),
            None => Ok(r["ok"].clone()),
        }
    }

    /// The next event named `ev` whose body passes `pred`.
    fn wait(&mut self, ev: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let t = Instant::now();
        loop {
            if let Some(i) = self.events.iter().position(|e| e["ev"] == ev && pred(&e["body"])) {
                return self.events.remove(i);
            }
            assert!(t.elapsed() < Duration::from_secs(10), "no {ev} event");
            let v = self.read();
            if v.get("ev").is_some() {
                self.events.push(v);
            }
        }
    }
}

fn has(list: &Value, key: &str, value: &str) -> bool {
    list.as_array().is_some_and(|a| a.iter().any(|x| x[key] == value))
}

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
