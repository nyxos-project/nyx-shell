//! `nyx-shell-core`: the Nyx shell's coordinator. One per graphical session, started by
//! `nyx-shell-core.service`.

use nyx_shell_core::server::{self, Outcome, ServerOptions};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: nyx-shell-core [--read-only] [--socket PATH] [--no-dbus]

  --read-only   Follow the desktop but never act on it: no dispatches, no actions that
                reach other programs, no announcements to the GTK panels.
                Also NYX_SHELL_READ_ONLY=1.
  --socket PATH Listen here instead of $XDG_RUNTIME_DIR/nyx-shell/core.sock
                (also NYX_SHELL_SOCKET).
  --no-dbus     Don't connect to D-Bus (the GTK panels and the lock state).
  --version     Print the version.

Logs go to stderr; RUST_LOG sets the level (default info).";

fn main() -> ExitCode {
    let mut read_only = std::env::var("NYX_SHELL_READ_ONLY").is_ok_and(|v| v == "1");
    let mut socket = None;
    let mut dbus = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--read-only" => read_only = true,
            "--socket" => socket = args.next().map(std::path::PathBuf::from),
            "--no-dbus" => dbus = false,
            "--version" => {
                println!("nyx-shell-core {}", nyx_shell_core::core::VERSION);
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();
    let Some(socket) = socket.or_else(nyx_shell_proto::socket_path) else {
        eprintln!("nyx-shell-core: XDG_RUNTIME_DIR isn't set and no --socket was given");
        return ExitCode::FAILURE;
    };
    let mut options = ServerOptions::standard(socket, read_only);
    options.dbus = dbus;
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("nyx-shell-core: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(server::run(options, stop_signal(), None)) {
        Ok(Outcome::Stopped) => ExitCode::SUCCESS,
        Ok(Outcome::AlreadyRunning) => {
            tracing::info!("another nyx-shell-core runs this session; leaving it be");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "stopped");
            ExitCode::FAILURE
        }
    }
}

async fn stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}
