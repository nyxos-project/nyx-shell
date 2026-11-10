//! Running Shell Core: the socket, connections, the watchers, and carrying out effects.

use crate::actions::Environment;
use crate::compositor::Compositor;
use crate::config::ConfigStore;
use crate::core::{BridgeCmd, Core, Effect, Msg, Options, ReplyTo};
use crate::peer::{Trust, exe_of};
use crate::session::{self, Claimed};
use crate::theme::ThemeSource;
use crate::{ClientId, now_us};
use nyx_shell_proto::frame::DecodeError;
use nyx_shell_proto::{ErrorBody, ErrorCode, MAX_LINE, MIN_PROTOCOL_VERSION, Op, PROTOCOL_VERSION, RawRequest, Reply, Routed};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

pub struct ServerOptions {
    pub socket: PathBuf,
    pub read_only: bool,
    /// Follow Hyprland (off in tests that feed the desktop themselves).
    pub hyprland: bool,
    /// The D-Bus bridge (panels, logind).
    pub dbus: bool,
    pub watch_files: bool,
    pub ready_timeout: Duration,
    pub config: ConfigStore,
    pub theme: ThemeSource,
    pub compositor: Arc<dyn Compositor>,
    /// How clients are classified; `None` for the standard rules.
    pub trust: Option<Trust>,
}

impl ServerOptions {
    pub fn standard(socket: PathBuf, read_only: bool) -> ServerOptions {
        ServerOptions {
            socket,
            read_only,
            hyprland: true,
            dbus: true,
            watch_files: true,
            ready_timeout: Duration::from_secs(2),
            config: ConfigStore::standard(),
            theme: ThemeSource::standard(),
            compositor: Arc::new(crate::compositor::Hyprland),
            trust: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stopped,
    AlreadyRunning,
}

/// Run until `shutdown` resolves. `started` receives the core's inbox once the socket
/// is listening (tests feed it desktop updates).
pub async fn run(
    o: ServerOptions,
    shutdown: impl Future<Output = ()>,
    started: Option<oneshot::Sender<mpsc::UnboundedSender<Msg>>>,
) -> anyhow::Result<Outcome> {
    let claim = match session::claim(&o.socket)? {
        Claimed::Ours(c) => c,
        Claimed::AlreadyRunning => return Ok(Outcome::AlreadyRunning),
    };
    let listener = UnixListener::bind(&o.socket)?;
    session::restrict(&o.socket)?;
    tracing::info!(socket = %o.socket.display(), read_only = o.read_only, "listening");

    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut core = Core::new(Options {
        read_only: o.read_only,
        config: o.config.clone(),
        theme: o.theme.clone(),
        env: Environment::discover(),
        session_id: session::session_id(),
        virtual_session: if o.hyprland {
            nyx_desktop::virtual_monitors::SessionState::path().map(|p| nyx_desktop::virtual_monitors::SessionState::load(&p)).unwrap_or_default()
        } else {
            Default::default()
        },
    });
    for e in &core.config().errors {
        tracing::warn!(error = %e, "shell settings");
    }
    let max_queue = core.config().config.ipc.max_queue;
    let mut ui_runtimes = core.config().config.ipc.ui_runtimes.clone();
    let (trust_tx, trust_rx) = tokio::sync::watch::channel(o.trust.clone().unwrap_or_else(|| Trust::standard(&ui_runtimes)));

    let _watcher = o.hyprland.then(|| {
        let tx = tx.clone();
        crate::compositor::watch(move |u| {
            let _ = tx.send(Msg::Desktop(u));
        })
    });
    let bridge = o.dbus.then(|| {
        let (btx, brx) = mpsc::unbounded_channel();
        tokio::spawn(crate::bridge::run(tx.clone(), brx));
        btx
    });
    let _files = if o.watch_files { watch_files(&o.config, &o.theme, tx.clone()) } else { None };
    {
        let tx = tx.clone();
        let timeout = o.ready_timeout;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let _ = tx.send(Msg::ReadyTimeout);
        });
    }
    tokio::spawn(accept(listener, tx.clone(), trust_rx, max_queue));
    if let Some(s) = started {
        let _ = s.send(tx.clone());
    }

    tokio::pin!(shutdown);
    loop {
        let lagging = core.has_lagging();
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                core.handle(msg);
            }
            _ = tokio::time::sleep(Duration::from_millis(250)), if lagging => core.retry_resyncs(),
            _ = &mut shutdown => {
                tracing::info!("stopping");
                session::notify("STOPPING=1");
                core.handle(Msg::Shutdown);
                // Let the writers flush the shutdown event.
                tokio::time::sleep(Duration::from_millis(50)).await;
                break;
            }
        }
        for effect in core.take_effects() {
            carry_out(effect, &o, bridge.as_ref(), &tx);
        }
        // `ipc.ui_runtimes` changed: new connections are classified with it.
        if o.trust.is_none() && core.config().config.ipc.ui_runtimes != ui_runtimes {
            ui_runtimes = core.config().config.ipc.ui_runtimes.clone();
            tracing::info!(?ui_runtimes, "trusted UI runtimes changed");
            let _ = trust_tx.send(Trust::standard(&ui_runtimes));
        }
    }
    let _ = std::fs::remove_file(&claim.socket);
    Ok(Outcome::Stopped)
}

fn reply(to: ReplyTo, result: Result<serde_json::Value, ErrorBody>) {
    let (id, tx) = to;
    let _ = tx.send(match result {
        Ok(v) => Reply::ok(id, v),
        Err(e) => Reply::err(id, e),
    });
}

fn carry_out(effect: Effect, o: &ServerOptions, bridge: Option<&mpsc::UnboundedSender<BridgeCmd>>, inbox: &mpsc::UnboundedSender<Msg>) {
    match effect {
        Effect::Ready => {
            tracing::info!("ready");
            session::notify("READY=1");
        }
        Effect::SaveVirtualSession(state) => {
            tokio::task::spawn_blocking(move || {
                let saved = nyx_desktop::virtual_monitors::SessionState::path().and_then(|p| state.save(&p));
                if let Err(e) = saved {
                    tracing::debug!(error = %e, "couldn't remember the virtual monitors' workspaces");
                }
            });
        }
        Effect::Dispatch { steps, reply: to } => {
            let compositor = o.compositor.clone();
            tokio::task::spawn_blocking(move || {
                let mut first = None;
                for step in &steps {
                    let r = compositor.dispatch(step);
                    if let Err(e) = &r {
                        tracing::debug!(dispatch = %step.classic, error = %e, "refused");
                    }
                    first.get_or_insert(r);
                }
                if let Some(to) = to {
                    let r = first.unwrap_or(Ok(())).map(|()| serde_json::json!({})).map_err(|e| ErrorBody::new(ErrorCode::Unavailable, e));
                    reply(to, r);
                }
            });
        }
        Effect::Bridge(cmd) => match bridge {
            Some(b) => {
                let _ = b.send(cmd);
            }
            None => {
                if let BridgeCmd::Toggle { reply: Some(to), .. } = cmd {
                    reply(to, Err(ErrorBody::new(ErrorCode::Unavailable, "D-Bus is off")));
                }
            }
        },
        Effect::Switcher { path, line, reply: to } => {
            tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let sent = std::os::unix::net::UnixStream::connect(&path).and_then(|mut s| {
                    s.set_write_timeout(Some(Duration::from_millis(200)))?;
                    s.write_all(format!("{line} {}\n", now_us()).as_bytes())
                });
                let r = sent
                    .map(|()| serde_json::json!(Routed { routed: "socket".into() }))
                    .map_err(|e| ErrorBody::new(ErrorCode::Unavailable, format!("nyx-switcher: {e}")));
                reply(to, r);
            });
        }
        Effect::Launch { argv, reply: to } => {
            let r = launch(&argv).map(|()| serde_json::json!(Routed { routed: "launch".into() }));
            reply(to, r);
        }
        Effect::Run { argv, reply: to } => {
            tokio::spawn(async move {
                let r = run_to_end(&argv).await.map(|()| serde_json::json!({}));
                reply(to, r);
            });
        }
        Effect::ResolvePointer { client, overlay, reply: to } => {
            let compositor = o.compositor.clone();
            let inbox = inbox.clone();
            tokio::spawn(async move {
                let monitor = tokio::task::spawn_blocking(move || compositor.pointer_monitor()).await.ok().flatten();
                let _ = inbox.send(Msg::PointerResolved { client, overlay, monitor, reply: to });
            });
        }
    }
}

/// Start a built-in program, detached, through uwsm when it's there (so it gets its own
/// systemd scope, as NyxOS's keybinds do).
fn launch(argv: &[String]) -> Result<(), ErrorBody> {
    let mut full: Vec<String> = Vec::new();
    if crate::actions::which("uwsm").is_some() {
        full.extend(["uwsm".into(), "app".into(), "--".into()]);
    }
    full.extend(argv.iter().cloned());
    let (program, args) = full.split_first().ok_or_else(|| ErrorBody::new(ErrorCode::Internal, "nothing to launch"))?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).process_group(0);
    let mut child = cmd.spawn().map_err(|e| ErrorBody::new(ErrorCode::Unavailable, format!("{program}: {e}")))?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

/// Run a built-in program and wait for it, at most [`RUN_TIMEOUT`].
async fn run_to_end(argv: &[String]) -> Result<(), ErrorBody> {
    let (program, args) = argv.split_first().ok_or_else(|| ErrorBody::new(ErrorCode::Internal, "nothing to run"))?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let unavailable = |e: String| ErrorBody::new(ErrorCode::Unavailable, format!("{program}: {e}"));
    let mut child = cmd.spawn().map_err(|e| unavailable(e.to_string()))?;
    match tokio::time::timeout(RUN_TIMEOUT, child.wait()).await {
        Ok(Ok(s)) if s.success() => Ok(()),
        Ok(Ok(s)) => Err(unavailable(s.to_string())),
        Ok(Err(e)) => Err(unavailable(e.to_string())),
        Err(_) => Err(unavailable("didn't finish in time".into())),
    }
}

const RUN_TIMEOUT: Duration = Duration::from_secs(5);

async fn accept(listener: UnixListener, tx: mpsc::UnboundedSender<Msg>, trust: tokio::sync::watch::Receiver<Trust>, max_queue: usize) {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let mut next: ClientId = 0;
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let Ok(cred) = stream.peer_cred() else { continue };
        if cred.uid() != uid {
            tracing::warn!(uid = cred.uid(), "refused a connection from another user");
            continue;
        }
        let pid = cred.pid();
        let identity = trust.borrow().classify(pid.and_then(exe_of).as_deref(), pid);
        next += 1;
        let client = next;
        tracing::debug!(client, program = %identity.program, "connected");
        let (events_tx, events_rx) = mpsc::channel(max_queue);
        if tx.send(Msg::Connect { client, identity, events: events_tx }).is_err() {
            return;
        }
        tokio::spawn(connection(stream, client, tx.clone(), events_rx));
    }
}

/// Read requests one at a time (each waits for its reply, so a client can't queue
/// unbounded work) and write replies and events.
async fn connection(stream: UnixStream, client: ClientId, tx: mpsc::UnboundedSender<Msg>, mut events: mpsc::Receiver<String>) {
    let (read, mut write) = stream.into_split();
    let (replies_tx, mut replies) = mpsc::channel::<String>(4);
    let writer = tokio::spawn(async move {
        let (mut replies_open, mut events_open) = (true, true);
        while replies_open || events_open {
            let line = tokio::select! {
                biased;
                r = replies.recv(), if replies_open => match r { Some(l) => l, None => { replies_open = false; continue } },
                e = events.recv(), if events_open => match e { Some(l) => l, None => { events_open = false; continue } },
            };
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match read_line(&mut reader, &mut buf).await {
            Ok(Line::Eof) | Err(_) => break,
            Ok(Line::TooLong) => {
                let _ = replies_tx.send(line(&Reply::err(0, ErrorBody::new(ErrorCode::TooLarge, "lines are at most 1 MiB")))).await;
                break;
            }
            Ok(Line::Data) => {}
        }
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match decode(&buf) {
            Err(r) => r,
            Ok((id, op)) => {
                let (otx, orx) = oneshot::channel();
                if tx.send(Msg::Request { client, id, op, reply: otx }).is_err() {
                    break;
                }
                orx.await.unwrap_or_else(|_| Reply::err(id, ErrorBody::new(ErrorCode::Internal, "no reply")))
            }
        };
        if replies_tx.send(line(&reply)).await.is_err() {
            break;
        }
    }
    let _ = tx.send(Msg::Disconnect { client });
    drop(replies_tx);
    let _ = writer.await;
    tracing::debug!(client, "disconnected");
}

enum Line {
    Data,
    Eof,
    TooLong,
}

async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<Line> {
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { Line::Eof } else { Line::Data });
        }
        let (take, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        buf.extend_from_slice(&available[..take]);
        r.consume(take);
        if buf.len() > MAX_LINE {
            return Ok(Line::TooLong);
        }
        if done {
            buf.pop();
            return Ok(Line::Data);
        }
    }
}

fn line(r: &Reply) -> String {
    let mut s = serde_json::to_string(r).expect("replies serialize");
    s.push('\n');
    s
}

/// A request line, or the reply that refuses it.
fn decode(buf: &[u8]) -> Result<(u64, Op), Reply> {
    let bad = |id: u64, msg: String| Reply::err(id, ErrorBody::new(ErrorCode::BadRequest, msg));
    let raw: RawRequest = match serde_json::from_slice(buf) {
        Ok(r) => r,
        Err(e) => {
            let id = serde_json::from_slice::<serde_json::Value>(buf).ok().and_then(|v| v.get("id").and_then(|i| i.as_u64())).unwrap_or(0);
            return Err(bad(id, format!("not a request: {e}")));
        }
    };
    if !(MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&raw.v) {
        return Err(Reply::err(
            raw.id,
            ErrorBody::new(ErrorCode::UnsupportedVersion, format!("the server speaks {MIN_PROTOCOL_VERSION} to {PROTOCOL_VERSION}")),
        ));
    }
    match Op::decode(&raw.op, raw.body) {
        Ok(op) => Ok((raw.id, op)),
        Err(DecodeError::UnknownOp) => Err(Reply::err(raw.id, ErrorBody::new(ErrorCode::UnknownOp, format!("no operation {:?}", raw.op)))),
        Err(DecodeError::BadBody(e)) => Err(bad(raw.id, format!("{}: {e}", raw.op))),
    }
}

/// Watch the settings and theme folders; tell the core after a short quiet period.
fn watch_files(config: &ConfigStore, theme: &ThemeSource, tx: mpsc::UnboundedSender<Msg>) -> Option<Arc<Mutex<notify::RecommendedWatcher>>> {
    use notify::{RecursiveMode, Watcher};
    let _ = std::fs::create_dir_all(&config.layers.user);
    let (ftx, mut frx) = mpsc::unbounded_channel::<Kind>();
    let theme_dirs = theme.dirs();
    let classify = {
        let t = theme.clone();
        move |p: &Path| -> Option<Kind> {
            if p.file_name().is_some_and(|n| n == "shell.toml") {
                return Some(Kind::Config);
            }
            let omarchy_current = t.omarchy_dir.parent();
            let theme_input = p == t.suite_file || p.starts_with(&t.themes_dir) || p.starts_with(&t.omarchy_dir) || p.parent() == omarchy_current;
            theme_input.then_some(Kind::Theme)
        }
    };
    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        for p in &event.paths {
            if let Some(k) = classify(p) {
                let _ = ftx.send(k);
            }
        }
    });
    let watcher = match watcher {
        Ok(w) => Arc::new(Mutex::new(w)),
        Err(e) => {
            tracing::warn!(error = %e, "settings and theme files aren't watched");
            return None;
        }
    };
    let mut dirs: Vec<PathBuf> = config.dirs();
    dirs.extend(theme_dirs);
    dirs.sort();
    dirs.dedup();
    {
        let mut w = watcher.lock().expect("not poisoned");
        for d in &dirs {
            if d.is_dir() {
                let _ = w.watch(d, RecursiveMode::NonRecursive);
            }
        }
    }
    let omarchy = theme.omarchy_dir.clone();
    let rewatch = watcher.clone();
    tokio::spawn(async move {
        while let Some(first) = frx.recv().await {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let (mut config, mut theme) = (first == Kind::Config, first == Kind::Theme);
            while let Ok(k) = frx.try_recv() {
                config |= k == Kind::Config;
                theme |= k == Kind::Theme;
            }
            if theme {
                // Omarchy replaces its `current/theme` link: watch the new target.
                let mut w = rewatch.lock().expect("not poisoned");
                let _ = w.unwatch(&omarchy);
                if omarchy.is_dir() {
                    let _ = w.watch(&omarchy, RecursiveMode::NonRecursive);
                }
            }
            if config && tx.send(Msg::ConfigFiles).is_err() {
                return;
            }
            if theme && tx.send(Msg::ThemeFiles).is_err() {
                return;
            }
        }
    });
    Some(watcher)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Config,
    Theme,
}
