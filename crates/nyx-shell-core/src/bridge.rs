//! D-Bus: the GTK panels' `Panel1`, their bus names, and logind's lock state.
//!
//! Optional: without a session bus the panels' actions are unavailable and nothing
//! else changes. Every call has a short timeout so a stuck panel can't stall anything.

use crate::core::{BridgeCmd, Msg};
use crate::overlay::PANEL1;
use futures_util::StreamExt;
use nyx_shell_proto::{ErrorBody, ErrorCode, Reply, Routed};
use std::time::Duration;
use tokio::sync::mpsc;
use zbus::Connection;

pub const INTERFACE: &str = "io.github.design_nexus.nyx.Panel1";
pub const PATH: &str = "/io/github/design_nexus/nyx/Panel";
const TIMEOUT: Duration = Duration::from_secs(2);

pub async fn run(tx: mpsc::UnboundedSender<Msg>, mut cmds: mpsc::UnboundedReceiver<BridgeCmd>) {
    let conn = match Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "no session bus: the GTK panels' actions are unavailable");
            let _ = tx.send(Msg::Dbus(false));
            while let Some(cmd) = cmds.recv().await {
                if let BridgeCmd::Toggle { reply: Some((id, reply)), .. } = cmd {
                    let _ = reply.send(Reply::err(id, ErrorBody::new(ErrorCode::Unavailable, "no session bus")));
                }
            }
            return;
        }
    };
    let _ = tx.send(Msg::Dbus(true));
    let names = watch_names(&conn, tx.clone());
    let opened = watch_opened(&conn, tx.clone());
    let lock = watch_lock(tx.clone());
    let commands = async {
        while let Some(cmd) = cmds.recv().await {
            let conn = conn.clone();
            let tx = tx.clone();
            tokio::spawn(async move { command(&conn, cmd, &tx).await });
        }
    };
    tokio::join!(names, opened, lock, commands);
}

async fn command(conn: &Connection, cmd: BridgeCmd, tx: &mpsc::UnboundedSender<Msg>) {
    match cmd {
        BridgeCmd::Announce(id) => {
            let sent = conn.emit_signal(Option::<zbus::names::BusName>::None, PATH, INTERFACE, "Opened", &(id.as_str(),)).await;
            if let Err(e) = sent {
                tracing::warn!(error = %e, "couldn't announce an overlay to the GTK panels");
            }
        }
        BridgeCmd::Toggle { bus, reply } => {
            let r = tokio::time::timeout(TIMEOUT, conn.call_method(Some(bus.as_str()), PATH, Some(INTERFACE), "Toggle", &())).await;
            let result = match r {
                Ok(Ok(_)) => Ok(serde_json::json!(Routed { routed: "dbus".into() })),
                Ok(Err(e)) => Err(ErrorBody::new(ErrorCode::Unavailable, format!("{bus}: {e}"))),
                Err(_) => Err(ErrorBody::new(ErrorCode::Unavailable, format!("{bus} didn't answer"))),
            };
            if let Some((id, reply)) = reply {
                let _ = reply.send(match result {
                    Ok(v) => Reply::ok(id, v),
                    Err(e) => Reply::err(id, e),
                });
            }
        }
        BridgeCmd::Refresh(panels) => {
            for (id, bus) in panels {
                let r = tokio::time::timeout(TIMEOUT, conn.call_method(Some(bus.as_str()), PATH, Some(INTERFACE), "IsOpen", &())).await;
                let open = match r {
                    Ok(Ok(m)) => m.body().deserialize::<bool>().unwrap_or(false),
                    _ => false,
                };
                let _ = tx.send(Msg::PanelState { id, open });
            }
        }
    }
}

async fn watch_names(conn: &Connection, tx: mpsc::UnboundedSender<Msg>) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return };
    let Ok(mut changes) = dbus.receive_name_owner_changed().await else { return };
    for (_, bus) in PANEL1 {
        let Ok(name) = zbus::names::BusName::try_from(*bus) else { continue };
        let present = dbus.name_has_owner(name).await.unwrap_or(false);
        let _ = tx.send(Msg::BusOwner { bus: (*bus).into(), present });
    }
    while let Some(signal) = changes.next().await {
        let Ok(args) = signal.args() else { continue };
        let name = args.name().as_str();
        if PANEL1.iter().any(|(_, b)| *b == name) {
            let _ = tx.send(Msg::BusOwner { bus: name.into(), present: args.new_owner().is_some() });
        }
    }
}

async fn watch_opened(conn: &Connection, tx: mpsc::UnboundedSender<Msg>) {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE)
        .and_then(|b| b.member("Opened"))
        .and_then(|b| b.path(PATH))
        .map(|b| b.build());
    let Ok(rule) = rule else { return };
    let Ok(mut stream) = zbus::MessageStream::for_match_rule(rule, conn, Some(32)).await else {
        tracing::warn!("couldn't listen for the GTK panels");
        return;
    };
    while let Some(Ok(msg)) = stream.next().await {
        if let Ok(id) = msg.body().deserialize::<String>() {
            let _ = tx.send(Msg::PanelOpened { id });
        }
    }
}

/// logind's `LockedHint` for this user's session. NyxOS's lock path sets it.
async fn watch_lock(tx: mpsc::UnboundedSender<Msg>) {
    let Ok(system) = Connection::system().await else {
        tracing::warn!("no system bus: the lock state isn't followed");
        return;
    };
    let proxy = zbus::Proxy::new(&system, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session").await;
    let Ok(proxy) = proxy else { return };
    match proxy.get_property::<bool>("LockedHint").await {
        Ok(l) => {
            let _ = tx.send(Msg::Locked(l));
        }
        Err(e) => {
            tracing::warn!(error = %e, "no logind session: the lock state isn't followed");
            return;
        }
    }
    let mut changes = proxy.receive_property_changed::<bool>("LockedHint").await;
    while let Some(c) = changes.next().await {
        if let Ok(l) = c.get().await {
            tracing::info!(locked = l, "session lock");
            let _ = tx.send(Msg::Locked(l));
        }
    }
}
