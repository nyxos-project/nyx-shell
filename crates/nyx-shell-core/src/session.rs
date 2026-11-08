//! One Shell Core per session: the lock file, stale sockets, and telling systemd.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Held for as long as this instance owns the socket.
pub struct Claim {
    pub socket: PathBuf,
    _lock: File,
}

pub enum Claimed {
    Ours(Claim),
    /// Another instance holds the lock or answers on the socket.
    AlreadyRunning,
}

/// Take the session's socket: make its folder private, take the lock beside it, and
/// remove a socket a crashed instance left behind.
pub fn claim(socket: &Path) -> io::Result<Claimed> {
    let dir = socket.parent().ok_or_else(|| io::Error::other("the socket path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let lock_path = PathBuf::from(format!("{}.lock", socket.display()));
    let lock = File::options().create(true).truncate(false).write(true).open(&lock_path)?;
    // SAFETY: flock on a file descriptor this function owns.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Ok(Claimed::AlreadyRunning);
        }
        return Err(e);
    }
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Ok(Claimed::AlreadyRunning);
        }
        tracing::info!(socket = %socket.display(), "removing a stale socket");
        std::fs::remove_file(socket)?;
    }
    Ok(Claimed::Ours(Claim { socket: socket.to_path_buf(), _lock: lock }))
}

/// Make the socket owner-only once bound.
pub fn restrict(socket: &Path) -> io::Result<()> {
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
}

/// `sd_notify`: `READY=1`, `STOPPING=1`, `STATUS=…`. Does nothing outside systemd.
pub fn notify(state: &str) {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::{SocketAddr, UnixDatagram};
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else { return };
    let Ok(sock) = UnixDatagram::unbound() else { return };
    let bytes = path.as_bytes();
    let sent = if let Some(name) = bytes.strip_prefix(b"@") {
        SocketAddr::from_abstract_name(name).and_then(|a| sock.send_to_addr(state.as_bytes(), &a))
    } else {
        sock.send_to(state.as_bytes(), &path)
    };
    if let Err(e) = sent {
        tracing::warn!(error = %e, "couldn't notify systemd");
    }
}

/// A random id for this session's Shell Core.
pub fn session_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid").map(|s| s.trim().to_string()).unwrap_or_else(|_| format!("{:x}", crate::now_us()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn one_instance_and_stale_sockets() {
        let d = tempfile::tempdir().unwrap();
        let socket = d.path().join("run/core.sock");
        let Claimed::Ours(first) = claim(&socket).unwrap() else { panic!("first claim") };
        assert!(matches!(claim(&socket).unwrap(), Claimed::AlreadyRunning), "the lock is held");
        let mode = std::fs::metadata(socket.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // A crashed instance: socket left behind, lock released.
        {
            let _l = UnixListener::bind(&socket).unwrap();
        }
        drop(first);
        assert!(socket.exists());
        let Claimed::Ours(_second) = claim(&socket).unwrap() else { panic!("stale socket replaced") };
        assert!(!socket.exists());
    }
}
