//! A pretend Hyprland and a protocol client, shared by the end-to-end tests.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------- A pretend Hyprland ----------

#[derive(Clone)]
pub struct World {
    pub monitors: Vec<Value>,
    pub workspaces: Vec<Value>,
    pub clients: Vec<Value>,
    pub active: String,
    /// Mapped layer surfaces' namespaces.
    pub layers: Vec<String>,
}

pub fn monitor(id: i64, name: &str, ws: i64, refresh: f64, focused: bool) -> Value {
    json!({"id": id, "name": name, "description": format!("{name} screen"), "make": "Maker", "model": name, "serial": "",
           "width": 1920, "height": 1080, "x": id * 1920, "y": 0, "scale": 1.0, "transform": 0, "focused": focused,
           "disabled": false, "activeWorkspace": {"id": ws, "name": ws.to_string()}, "specialWorkspace": {"id": 0, "name": ""},
           "reserved": [0, 0, 0, 0], "refreshRate": refresh})
}

pub fn workspace(id: i64, monitor: &str) -> Value {
    json!({"id": id, "name": id.to_string(), "monitor": monitor, "windows": 1, "hasfullscreen": false, "tiledLayout": "dwindle"})
}

pub fn client(addr: &str, class: &str, ws: i64, order: i32) -> Value {
    json!({"address": format!("0x{addr}"), "stableId": format!("{order}"), "mapped": true, "hidden": false, "at": [0, 0],
           "size": [800, 600], "workspace": {"id": ws, "name": ws.to_string()}, "floating": false, "pinned": false,
           "monitor": 0, "class": class, "title": format!("{class} window"), "initialClass": class, "pid": 1,
           "fullscreen": 0, "focusHistoryID": order})
}

pub struct Fake {
    pub dir: PathBuf,
    pub world: Arc<Mutex<World>>,
    pub events: Arc<Mutex<Vec<UnixStream>>>,
    pub dispatches: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    pub fn start(dir: &Path, world: World) -> Fake {
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
                        "j/layers" => {
                            let surfaces: Vec<Value> = w.layers.iter().map(|n| json!({"namespace": n})).collect();
                            json!({"eDP-1": {"levels": {"2": surfaces}}}).to_string()
                        }
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

    pub fn wait_subscribed(&self) {
        let t = Instant::now();
        while self.events.lock().unwrap().is_empty() {
            assert!(t.elapsed() < Duration::from_secs(5), "Shell Core never subscribed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn send(&self, lines: &str) {
        for s in self.events.lock().unwrap().iter_mut() {
            let _ = s.write_all(lines.as_bytes());
        }
    }

    pub fn change(&self, f: impl FnOnce(&mut World)) {
        f(&mut self.world.lock().unwrap());
    }

    pub fn shut(&self) {
        let _ = std::fs::remove_file(self.dir.join(".socket.sock"));
        let _ = std::fs::remove_file(self.dir.join(".socket2.sock"));
        for s in self.events.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

// ---------- A protocol client ----------

pub struct Client {
    pub reader: BufReader<UnixStream>,
    pub writer: UnixStream,
    pub next: u64,
    pub events: Vec<Value>,
}

impl Client {
    pub fn connect(socket: &Path) -> Client {
        let s = UnixStream::connect(socket).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let writer = s.try_clone().unwrap();
        Client { reader: BufReader::new(s), writer, next: 0, events: Vec::new() }
    }

    pub fn send_line(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
    }

    pub fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("a line before the timeout");
        assert!(n > 0, "connection closed");
        serde_json::from_str(&line).unwrap()
    }

    /// The reply to the last request, keeping events that arrive first.
    pub fn reply(&mut self) -> Value {
        loop {
            let v = self.read();
            if v.get("ev").is_some() {
                self.events.push(v);
                continue;
            }
            return v;
        }
    }

    pub fn call(&mut self, op: &str, body: Value) -> Result<Value, Value> {
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
    pub fn wait(&mut self, ev: &str, pred: impl Fn(&Value) -> bool) -> Value {
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

pub fn has(list: &Value, key: &str, value: &str) -> bool {
    list.as_array().is_some_and(|a| a.iter().any(|x| x[key] == value))
}

