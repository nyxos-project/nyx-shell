//! Who a client is, and what it may do (`docs/SECURITY.md`).

use nyx_shell_proto::Capability;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The program, when it's one Shell Core recognizes; else the executable's name.
    pub program: String,
    pub pid: Option<i32>,
    pub capabilities: BTreeSet<Capability>,
}

impl Identity {
    pub fn can(&self, c: Capability) -> bool {
        self.capabilities.contains(&c)
    }

    /// Everything (tests, and the in-process callers).
    pub fn trusted(program: &str) -> Identity {
        Identity { program: program.into(), pid: None, capabilities: Capability::ALL.into_iter().collect() }
    }
}

/// Where trust comes from: Shell Core's own folder, the system's trusted folders, and
/// the configured Quickshell executables.
#[derive(Debug, Clone)]
pub struct Trust {
    pub own_dir: PathBuf,
    pub trusted_dirs: Vec<PathBuf>,
    pub ui_runtimes: Vec<PathBuf>,
}

impl Trust {
    pub fn standard(ui_runtimes: &[String]) -> Trust {
        let own_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
        Trust {
            own_dir,
            trusted_dirs: nyx_peer::trusted_dirs(Path::new(nyx_peer::TRUSTED_DIRS_FILE)),
            ui_runtimes: ui_runtimes.iter().map(PathBuf::from).collect(),
        }
    }

    /// Classify the program at `exe`.
    pub fn classify(&self, exe: Option<&Path>, pid: Option<i32>) -> Identity {
        use Capability::*;
        let read_only = || [DesktopRead, WindowOwn].into_iter().collect::<BTreeSet<_>>();
        let Some(exe) = exe else {
            return Identity { program: "unknown".into(), pid, capabilities: read_only() };
        };
        let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("unknown").to_string();
        let trusted = nyx_peer::program_in(exe, &self.own_dir, &self.trusted_dirs);
        let (program, capabilities) = match trusted.as_deref() {
            Some("nyx-shellctl") => ("nyx-shellctl".to_string(), Capability::ALL.into_iter().collect()),
            Some("nyx-settings") => ("nyx-settings".to_string(), [DesktopRead, ConfigWrite, WindowOwn].into_iter().collect()),
            Some("quickshell") => ("quickshell".to_string(), ui()),
            _ if self.ui_runtimes.iter().any(|r| r == exe) => ("quickshell".to_string(), ui()),
            _ => (name, read_only()),
        };
        Identity { program, pid, capabilities }
    }
}

fn ui() -> BTreeSet<Capability> {
    use Capability::*;
    [DesktopRead, DesktopAct, OverlayManage, ComponentRegister, ActionInvoke, WindowOwn].into_iter().collect()
}

/// The executable a process runs.
pub fn exe_of(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trust() -> Trust {
        Trust {
            own_dir: "/opt/nyx-shell/bin".into(),
            trusted_dirs: vec!["/nix/store/abc-nyx-settings/bin".into(), "/nix/store/def-quickshell/bin".into()],
            ui_runtimes: vec!["/usr/bin/quickshell".into()],
        }
    }

    #[test]
    fn shellctl_from_its_own_folder_gets_everything() {
        let id = trust().classify(Some(Path::new("/opt/nyx-shell/bin/nyx-shellctl")), Some(1));
        assert_eq!(id.program, "nyx-shellctl");
        assert!(id.can(Capability::ConfigWrite) && id.can(Capability::DesktopAct));
    }

    #[test]
    fn a_shellctl_elsewhere_is_just_a_reader() {
        let id = trust().classify(Some(Path::new("/tmp/nyx-shellctl")), Some(1));
        assert!(id.can(Capability::DesktopRead));
        assert!(!id.can(Capability::ActionInvoke));
    }

    #[test]
    fn quickshell_and_settings() {
        let qs = trust().classify(Some(Path::new("/usr/bin/quickshell")), None);
        assert_eq!(qs.program, "quickshell");
        assert!(qs.can(Capability::OverlayManage) && !qs.can(Capability::ConfigWrite));
        let nix_qs = trust().classify(Some(Path::new("/nix/store/def-quickshell/bin/.quickshell-wrapped")), None);
        assert_eq!(nix_qs.program, "quickshell");
        let settings = trust().classify(Some(Path::new("/nix/store/abc-nyx-settings/bin/nyx-settings")), None);
        assert!(settings.can(Capability::ConfigWrite) && !settings.can(Capability::DesktopAct));
    }

    #[test]
    fn unknown_programs_read_only() {
        let id = trust().classify(Some(Path::new("/usr/bin/python3")), None);
        assert_eq!(id.program, "python3");
        assert_eq!(id.capabilities, [Capability::DesktopRead, Capability::WindowOwn].into_iter().collect());
        assert_eq!(trust().classify(None, None).program, "unknown");
    }
}
