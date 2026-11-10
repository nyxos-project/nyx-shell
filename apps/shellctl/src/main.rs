//! `nyx-shellctl`: talk to Nyx Shell Core. Plain std, so a keybind running
//! `nyx-shellctl invoke ACTION` costs a few milliseconds.

mod shortcuts;

use nyx_shell_proto::{ConfigState, EventFrame, Op, RawRequest, Reply, Topic, WindowAction, WindowActionKind};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str = "\
Usage: nyx-shellctl COMMAND

  status                     Session, compositor, components, overlays (--json for all of it)
  snapshot                   The desktop state
  watch [TOPIC…] [--latency] Print events: session desktop overlays actions config theme
  actions                    Every action and whether it can run
  invoke ACTION [ARGS-JSON]  Run an action (what keybinds call)
  overlays                   Overlay states
  show|hide|toggle OVERLAY   Drive an overlay
  components [ID]            Component states
  config                     The merged shell settings and their problems
  config set KEY JSON        Set a setting in ~/.config/nyx/shell.toml
  config reset KEY           Remove your value so the default shows through
  theme                      The theme generation and palette
  shortcuts --check          Compare [shortcuts] with Hyprland's binds
  shortcuts --apply [--file PATH] [--no-reload]
                             Write ~/.config/hypr/nyx-shell.lua, reload Hyprland, and
                             put the old file back if Hyprland reports errors
  window ADDRESS ACTION [WORKSPACE|MONITOR]
                             Run a window action (close, set_floating, set_tiled,
                             toggle_maximized, toggle_fullscreen, toggle_pinned,
                             minimize, move_to_workspace, move_to_monitor…)
  window-controls --print    The Hyprland rules [window_controls] makes
  window-controls --apply [--file PATH] [--no-reload]
                             Write them to ~/.config/hypr/nyx-window-controls.lua, as
                             shortcuts --apply does
  bench [N]                  Time N status round trips
  raw OP [BODY-JSON]         Send any operation

The socket is $XDG_RUNTIME_DIR/nyx-shell/core.sock, or NYX_SHELL_SOCKET.";

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: u64,
    /// Events read while waiting for a reply.
    pub pending: Vec<EventFrame>,
}

impl Client {
    pub fn connect() -> Result<Client, String> {
        let path = nyx_shell_proto::socket_path().ok_or("XDG_RUNTIME_DIR isn't set")?;
        let s = UnixStream::connect(&path).map_err(|e| format!("Nyx Shell Core isn't running ({}: {e})", path.display()))?;
        s.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let writer = s.try_clone().map_err(|e| e.to_string())?;
        let mut c = Client { reader: BufReader::new(s), writer, next: 0, pending: Vec::new() };
        c.call(&Op::Hello(nyx_shell_proto::Hello { client: "nyx-shellctl".into(), min: 1, max: 1 }))?;
        Ok(c)
    }

    pub fn raw(&mut self, op: &str, body: Value) -> Result<Value, String> {
        self.next += 1;
        let req = RawRequest { v: nyx_shell_proto::PROTOCOL_VERSION, id: self.next, op: op.into(), body };
        let mut line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        loop {
            let text = self.read_line()?;
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("bad reply: {e}"))?;
            if v.get("ev").is_some() {
                if let Ok(f) = serde_json::from_value(v) {
                    self.pending.push(f);
                }
                continue;
            }
            let r: Reply = serde_json::from_value(v).map_err(|e| format!("bad reply: {e}"))?;
            if r.id != self.next {
                continue;
            }
            return match (r.ok, r.err) {
                (_, Some(e)) => Err(e.to_string()),
                (Some(v), None) => Ok(v),
                (None, None) => Ok(Value::Null),
            };
        }
    }

    pub fn call(&mut self, op: &Op) -> Result<Value, String> {
        let raw = RawRequest::new(0, op);
        self.raw(&raw.op, raw.body)
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err("Nyx Shell Core closed the connection".into()),
            Ok(_) => Ok(line),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn next_event(&mut self) -> Result<EventFrame, String> {
        if !self.pending.is_empty() {
            return Ok(self.pending.remove(0));
        }
        self.reader.get_ref().set_read_timeout(None).ok();
        loop {
            let text = self.read_line()?;
            if let Ok(f) = serde_json::from_str::<EventFrame>(&text) {
                return Ok(f);
            }
        }
    }
}

fn pretty(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn parse_json(s: &str) -> Result<Value, String> {
    serde_json::from_str(s).map_err(|e| format!("not JSON: {e}"))
}

fn main() -> ExitCode {
    // Output piped into `head` ends quietly instead of panicking.
    // SAFETY: restoring the default action of a signal, before any thread starts.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&a) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("nyx-shellctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(a: &[&str]) -> Result<ExitCode, String> {
    let op = |c: &mut Client, o: Op| -> Result<(), String> {
        pretty(&c.call(&o)?);
        Ok(())
    };
    match a {
        [] | ["-h"] | ["--help"] | ["help"] => println!("{USAGE}"),
        ["status"] => status(&Client::connect()?.call(&Op::GetSessionStatus)?),
        ["status", "--json"] => op(&mut Client::connect()?, Op::GetSessionStatus)?,
        ["snapshot"] => op(&mut Client::connect()?, Op::GetDesktopSnapshot)?,
        ["watch", rest @ ..] => watch(rest)?,
        ["actions"] => op(&mut Client::connect()?, Op::ListActions)?,
        ["invoke", id] => op(&mut Client::connect()?, Op::InvokeAction { id: id.to_string(), args: None })?,
        ["invoke", id, args] => op(&mut Client::connect()?, Op::InvokeAction { id: id.to_string(), args: Some(parse_json(args)?) })?,
        ["overlays"] => op(&mut Client::connect()?, Op::GetOverlayState)?,
        ["show", id] => op(&mut Client::connect()?, Op::ShowOverlay { id: id.to_string(), monitor: None })?,
        ["hide", id] => op(&mut Client::connect()?, Op::HideOverlay { id: id.to_string() })?,
        ["toggle", id] => op(&mut Client::connect()?, Op::ToggleOverlay { id: id.to_string() })?,
        ["components"] => op(&mut Client::connect()?, Op::GetComponentStatus { id: None })?,
        ["components", id] => op(&mut Client::connect()?, Op::GetComponentStatus { id: Some(id.to_string()) })?,
        ["config"] | ["config", "get"] => op(&mut Client::connect()?, Op::GetShellConfig)?,
        ["config", "set", key, value] => op(&mut Client::connect()?, Op::UpdateShellConfig { key: key.to_string(), value: parse_json(value)? })?,
        ["config", "reset", key] => op(&mut Client::connect()?, Op::ResetShellConfig { key: key.to_string() })?,
        ["theme"] => op(&mut Client::connect()?, Op::GetThemeGeneration)?,
        ["shortcuts", rest @ ..] => return shortcuts::run(rest),
        ["window", address, action, rest @ ..] => op(&mut Client::connect()?, window_op(address, action, rest)?)?,
        ["window-controls", rest @ ..] => window_controls(rest)?,
        ["bench"] => bench(1000)?,
        ["bench", n] => bench(n.parse().map_err(|_| "N is a number")?)?,
        ["raw", o] => pretty(&Client::connect()?.raw(o, Value::Null)?),
        ["raw", o, body] => pretty(&Client::connect()?.raw(o, parse_json(body)?)?),
        _ => {
            eprintln!("{USAGE}");
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `window ADDRESS ACTION [TARGET]`: the target is the workspace or monitor the move
/// actions need.
fn window_op(address: &str, action: &str, rest: &[&str]) -> Result<Op, String> {
    let action: WindowActionKind = serde_json::from_value(json!(action)).map_err(|_| format!("unknown window action {action}"))?;
    let target = rest.first().map(|s| s.to_string());
    Ok(Op::RequestWindowAction(WindowAction {
        window: address.trim_start_matches("0x").to_ascii_lowercase(),
        action,
        workspace: (action == WindowActionKind::MoveToWorkspace).then(|| target.clone()).flatten(),
        monitor: (action == WindowActionKind::MoveToMonitor).then_some(target).flatten(),
        follow: false,
    }))
}

fn window_controls(args: &[&str]) -> Result<(), String> {
    let (print, apply) = (args.contains(&"--print"), args.contains(&"--apply"));
    if print == apply {
        return Err("give --print or --apply".into());
    }
    let config: ConfigState = serde_json::from_value(Client::connect()?.call(&Op::GetShellConfig)?).map_err(|e| e.to_string())?;
    let text = nyx_shell_proto::window::rules_lua(&config.config.window_controls);
    if print {
        print!("{text}");
        return Ok(());
    }
    let file = shortcuts::file_arg(args).unwrap_or_else(|| shortcuts::hypr_file("nyx-window-controls.lua"));
    shortcuts::install(&file, &text, !args.contains(&"--no-reload"))
}

fn status(s: &Value) {
    let get = |p: &str| s.pointer(p).cloned().unwrap_or(Value::Null);
    println!("nyx-shell-core {}  session {}", get("/version").as_str().unwrap_or("?"), get("/session").as_str().unwrap_or("?"));
    println!(
        "state {}{}{}  up {} s",
        get("/state").as_str().unwrap_or("?"),
        if get("/read_only") == json!(true) { "  read-only" } else { "" },
        if get("/locked") == json!(true) { "  locked" } else { "" },
        get("/uptime_ms").as_u64().unwrap_or(0) / 1000
    );
    println!(
        "hyprland {}  generation {}  last event {} ms ago",
        if get("/compositor/connected") == json!(true) { "connected" } else { "unreachable" },
        get("/compositor/generation"),
        get("/compositor/last_event_ms_ago")
    );
    println!(
        "clients {}  dropped events {}  resyncs {}",
        get("/ipc/clients"),
        get("/ipc/dropped_events"),
        get("/ipc/resyncs")
    );
    println!("components:");
    for c in get("/components").as_array().into_iter().flatten() {
        let detail = c.get("detail").and_then(|d| d.as_str()).map(|d| format!(" ({d})")).unwrap_or_default();
        println!(
            "  {:<24} {:<10} {:<10} {}{}",
            c["id"].as_str().unwrap_or("?"),
            c["state"].as_str().unwrap_or("?"),
            c["effective"].as_str().unwrap_or("?"),
            c["owner"].as_str().unwrap_or("?"),
            detail
        );
    }
    println!("overlays:");
    for o in get("/overlays").as_array().into_iter().flatten() {
        println!(
            "  {:<24} {:<8} {}",
            o["id"].as_str().unwrap_or("?"),
            if o["visible"] == json!(true) { "visible" } else { "hidden" },
            o["owner"].as_str().unwrap_or("?")
        );
    }
    let errors = get("/config/errors");
    for e in errors.as_array().into_iter().flatten() {
        println!("settings: {}", e.as_str().unwrap_or("?"));
    }
}

fn watch(rest: &[&str]) -> Result<(), String> {
    let latency = rest.contains(&"--latency");
    let names: Vec<&str> = rest.iter().copied().filter(|a| *a != "--latency").collect();
    let topics: Vec<Topic> = if names.is_empty() {
        Topic::ALL.to_vec()
    } else {
        names.iter().map(|n| serde_json::from_value(json!(n)).map_err(|_| format!("unknown topic {n}"))).collect::<Result<_, _>>()?
    };
    let mut c = Client::connect()?;
    for t in topics {
        let op = match t {
            Topic::Session => Op::SubscribeSessionEvents,
            Topic::Desktop => Op::SubscribeDesktopChanges,
            Topic::Overlays => Op::SubscribeOverlayState,
            Topic::Actions => Op::SubscribeActionAvailability,
            Topic::Config => Op::SubscribeConfigChanges,
            Topic::Theme => Op::SubscribeThemeChanges,
        };
        let r = c.call(&op)?;
        eprintln!("subscribed to {} at seq {}", serde_json::to_value(t).unwrap_or_default(), r["seq"]);
    }
    let mut last_seq = 0;
    loop {
        let f = c.next_event()?;
        if last_seq != 0 && f.seq != last_seq + 1 {
            eprintln!("gap: {} after {}", f.seq, last_seq);
        }
        last_seq = f.seq;
        if latency {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64);
            println!("{} seq {} {} µs", f.ev, f.seq, now.saturating_sub(f.ts));
        } else {
            println!("{}", serde_json::to_string(&f).unwrap_or_default());
        }
        std::io::stdout().flush().ok();
    }
}

fn bench(n: usize) -> Result<(), String> {
    let mut c = Client::connect()?;
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        c.call(&Op::GetSessionStatus)?;
        times.push(t.elapsed().as_micros() as u64);
    }
    times.sort_unstable();
    let pct = |p: f64| times[((times.len() as f64 - 1.0) * p) as usize];
    println!("{n} round trips: median {} µs, p95 {} µs, max {} µs", pct(0.5), pct(0.95), times[times.len() - 1]);
    Ok(())
}
