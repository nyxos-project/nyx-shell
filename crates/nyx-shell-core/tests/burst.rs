//! Rapid workspace switching: thousands of events as fast as Hyprland can send them.
//! The client that keeps reading must end with exactly the state a fresh snapshot has.
//! `NYX_SHELL_BURST=N` changes the count and `NYX_SHELL_BURST_PACE_US` spaces the
//! switches (0, the default, sends them in one go); `--nocapture` prints the timings.

mod common;

use common::{Client, Fake, World, client, monitor, workspace};
use nyx_shell_core::config::ConfigStore;
use nyx_shell_core::peer::Trust;
use nyx_shell_core::server::{self, ServerOptions};
use nyx_shell_core::theme::ThemeSource;
use nyx_shell_proto::{DesktopDelta, DesktopState};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn rapid_workspace_switches() {
    let n: usize = std::env::var("NYX_SHELL_BURST").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    let pace: u64 = std::env::var("NYX_SHELL_BURST_PACE_US").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let tmp = tempfile::tempdir().unwrap();
    let runtime = tmp.path().join("run");
    // SAFETY: set before any thread starts; this file has one test.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", "sig");
    }
    let fake = Fake::start(
        &runtime.join("hypr/sig"),
        World {
            monitors: vec![monitor(0, "eDP-1", 1, 60.0, true)],
            workspaces: vec![workspace(1, "eDP-1"), workspace(2, "eDP-1")],
            clients: vec![client("a1", "firefox", 1, 0), client("b2", "kitty", 2, 1)],
            active: "a1".into(),
            layers: vec![],
        },
    );
    let root = tmp.path();
    let socket = runtime.join("nyx-shell/core.sock");
    let options = ServerOptions {
        socket: socket.clone(),
        read_only: false,
        hyprland: true,
        dbus: false,
        watch_files: false,
        ready_timeout: Duration::from_secs(2),
        config: ConfigStore { layers: nyx_config::Layers { defaults: root.join("d"), system: root.join("s"), user: root.join("u") } },
        theme: ThemeSource { suite_file: root.join("u/theme.toml"), themes_dir: root.join("u/themes"), omarchy_dir: root.join("o"), snapshot_file: root.join("r/system.json") },
        compositor: Arc::new(nyx_shell_core::compositor::Hyprland),
        trust: Some(Trust { own_dir: PathBuf::from("/nonexistent"), trusted_dirs: vec![], ui_runtimes: vec![] }),
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(server::run(options, async { let _ = stop_rx.await; }, Some(started_tx))).unwrap()
    });
    tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(started_rx).unwrap();
    fake.wait_subscribed();

    let mut c = Client::connect(&socket);
    let mut state: DesktopState = loop {
        let s = c.call("SubscribeDesktopChanges", json!({})).unwrap();
        if s["state"]["connected"] == true {
            c.events.retain(|e| e["seq"].as_u64() > s["seq"].as_u64());
            break serde_json::from_value(s["state"].clone()).unwrap();
        }
        c.call("Unsubscribe", json!({"topic": "desktop"})).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    };

    let t = Instant::now();
    let streams = fake.events.clone();
    let sender = std::thread::spawn(move || {
        for i in 0..n {
            let ws = 1 + (i % 2) as i64;
            let line = format!("workspacev2>>{ws},{ws}\nactivewindowv2>>{}\n", if ws == 1 { "a1" } else { "b2" });
            for s in streams.lock().unwrap().iter_mut() {
                let _ = std::io::Write::write_all(s, line.as_bytes());
            }
            if pace > 0 {
                std::thread::sleep(Duration::from_micros(pace));
            }
        }
        t.elapsed()
    });
    // Follow the protocol: apply deltas, take a snapshot on Resync or a delta that
    // doesn't apply, and compare with a fresh snapshot once the stream goes quiet.
    let (mut deltas, mut resyncs) = (0, 0);
    let mut pending: std::collections::VecDeque<serde_json::Value> = std::mem::take(&mut c.events).into();
    c.reader.get_ref().set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    loop {
        let e = match pending.pop_front() {
            Some(e) => e,
            None => {
                let mut line = String::new();
                match std::io::BufRead::read_line(&mut c.reader, &mut line) {
                    Ok(n) if n > 0 => serde_json::from_str(&line).unwrap(),
                    _ if !sender.is_finished() => continue,
                    _ => {
                        c.reader.get_ref().set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                        let snap: DesktopState = serde_json::from_value(c.call("GetDesktopSnapshot", json!({})).unwrap()).unwrap();
                        pending.extend(std::mem::take(&mut c.events));
                        if pending.is_empty() {
                            assert_eq!(state, snap, "after the stream went quiet");
                            break;
                        }
                        c.reader.get_ref().set_read_timeout(Some(Duration::from_millis(300))).unwrap();
                        continue;
                    }
                }
            }
        };
        match e["ev"].as_str() {
            Some("DesktopChanged") => {
                let d: DesktopDelta = serde_json::from_value(e["body"].clone()).unwrap();
                deltas += 1;
                if state.apply(&d).is_err() {
                    state = serde_json::from_value(c.call("GetDesktopSnapshot", json!({})).unwrap()).unwrap();
                    pending.extend(std::mem::take(&mut c.events));
                }
            }
            Some("Resync") => {
                resyncs += 1;
                state = serde_json::from_value(c.call("GetDesktopSnapshot", json!({})).unwrap()).unwrap();
                pending.extend(std::mem::take(&mut c.events));
            }
            _ => {}
        }
        assert!(t.elapsed() < Duration::from_secs(60), "the client never caught up");
    }
    let total = t.elapsed();
    let sent = sender.join().unwrap();
    let status = c.call("GetSessionStatus", json!({})).unwrap();
    eprintln!(
        "{n} workspace switches ({} events, {pace} µs apart) sent in {:?}; the client matched a fresh snapshot after {:?}: {deltas} deltas, {resyncs} resyncs, {} events dropped",
        n * 2,
        sent,
        total.saturating_sub(Duration::from_millis(300)),
        status["ipc"]["dropped_events"]
    );
    let _ = stop_tx.send(());
    server.join().unwrap();
    fake.shut();
}
