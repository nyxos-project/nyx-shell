//! `nyx-shellctl shortcuts`: Shell Core's keys in an owned Hyprland file.
//!
//! The keys come from `[shortcuts]` in `shell.toml`. A key another bind already uses is
//! a conflict and nothing is written. The file is replaced whole, Hyprland reloads, and
//! if it reports configuration errors the previous file is put back.

use crate::Client;
use nyx_shell_proto::shortcut::{DESCRIPTION_PREFIX, Shortcut};
use nyx_shell_proto::{ConfigState, Op};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Debug, Deserialize)]
pub struct Bind {
    pub modmask: u32,
    pub key: String,
    #[serde(default)]
    pub submap: String,
    #[serde(default)]
    pub description: String,
}

/// (action, keys) pairs and binds of others that use the same keys.
pub fn conflicts<'a>(wanted: &'a [(String, Shortcut)], binds: &'a [Bind]) -> Vec<(&'a str, &'a Bind)> {
    let mut out = Vec::new();
    for (action, s) in wanted {
        for b in binds {
            if b.submap.is_empty() && s.matches(b.modmask, &b.key) && !b.description.starts_with(DESCRIPTION_PREFIX) {
                out.push((action.as_str(), b));
            }
        }
    }
    out
}

/// Two actions given the same keys.
pub fn duplicates(wanted: &[(String, Shortcut)]) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for (i, (a, s)) in wanted.iter().enumerate() {
        for (b, t) in &wanted[i + 1..] {
            if s == t {
                out.push((a.as_str(), b.as_str()));
            }
        }
    }
    out
}

pub fn file_text(wanted: &[(String, Shortcut)]) -> Result<String, String> {
    let mut out = String::from(
        "-- Written by `nyx-shellctl shortcuts --apply` from [shortcuts] in ~/.config/nyx/shell.toml.\n\
         -- Change the keys there; this file is replaced.\n",
    );
    for (action, s) in wanted {
        out.push_str(&s.lua_bind(action)?);
        out.push('\n');
    }
    Ok(out)
}

/// `~/.config/hypr/NAME` (or under `$XDG_CONFIG_HOME`).
pub fn hypr_file(name: &str) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    config.join("hypr").join(name)
}

fn hyprctl_json(args: &[&str]) -> Result<serde_json::Value, String> {
    let out = Command::new("hyprctl").arg("-j").args(args).output().map_err(|e| format!("hyprctl: {e}"))?;
    serde_json::from_slice(&out.stdout).map_err(|e| format!("hyprctl {}: {e}", args.join(" ")))
}

pub fn run(args: &[&str]) -> Result<ExitCode, String> {
    let apply = args.contains(&"--apply");
    let check = args.contains(&"--check");
    if apply == check {
        return Err("give --check or --apply".into());
    }
    let reload = !args.contains(&"--no-reload");
    let file = file_arg(args).unwrap_or_else(|| hypr_file("nyx-shell.lua"));

    let config: ConfigState = serde_json::from_value(Client::connect()?.call(&Op::GetShellConfig)?).map_err(|e| e.to_string())?;
    let wanted: Vec<(String, Shortcut)> = config
        .config
        .shortcuts
        .iter()
        .map(|(a, k)| Shortcut::parse(k).map(|s| (a.clone(), s)))
        .collect::<Result<_, _>>()?;
    let binds: Vec<Bind> = serde_json::from_value(hyprctl_json(&["binds"])?).map_err(|e| e.to_string())?;
    let clash = conflicts(&wanted, &binds);
    let dup = duplicates(&wanted);
    for (action, s) in &wanted {
        println!("{:<40} {}", action, s.canonical());
    }
    for (action, b) in &clash {
        println!("conflict: {action} wants keys already bound to {:?}", if b.description.is_empty() { "an unnamed bind" } else { &b.description });
    }
    for (a, b) in &dup {
        println!("conflict: {a} and {b} have the same keys");
    }
    if !clash.is_empty() || !dup.is_empty() {
        return Ok(ExitCode::FAILURE);
    }
    if check {
        println!("no conflicts");
        return Ok(ExitCode::SUCCESS);
    }
    install(&file, &file_text(&wanted)?, reload)?;
    Ok(ExitCode::SUCCESS)
}

/// `--file PATH`.
pub fn file_arg(args: &[&str]) -> Option<PathBuf> {
    args.iter().position(|a| *a == "--file").and_then(|i| args.get(i + 1)).map(PathBuf::from)
}

/// Replace `file` with `text`; with `reload`, reload Hyprland and put the previous file
/// back if Hyprland reports configuration errors.
pub fn install(file: &Path, text: &str, reload: bool) -> Result<(), String> {
    let previous = std::fs::read(file).ok();
    if previous.as_deref() == Some(text.as_bytes()) {
        println!("{} is up to date", file.display());
        return Ok(());
    }
    write_atomic(file, text.as_bytes())?;
    if reload {
        let reloaded = Command::new("hyprctl").arg("reload").output().map_err(|e| format!("hyprctl reload: {e}"))?;
        let errors: Vec<String> = serde_json::from_value(hyprctl_json(&["configerrors"])?).unwrap_or_default();
        let errors: Vec<String> = errors.into_iter().filter(|e| !e.trim().is_empty()).collect();
        if !reloaded.status.success() || !errors.is_empty() {
            match previous {
                Some(p) => write_atomic(file, &p)?,
                None => std::fs::remove_file(file).map_err(|e| e.to_string())?,
            }
            let _ = Command::new("hyprctl").arg("reload").output();
            return Err(format!("Hyprland reported errors, so the previous file is back: {}", errors.join("; ")));
        }
    }
    println!("wrote {}", file.display());
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("the file has no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".nyx-shell.lua.{}", std::process::id()));
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(modmask: u32, key: &str, description: &str) -> Bind {
        Bind { modmask, key: key.into(), submap: String::new(), description: description.into() }
    }

    #[test]
    fn finds_conflicts_but_not_our_own_binds() {
        let wanted = vec![("shell.control-center.toggle".to_string(), Shortcut::parse("SUPER + D").unwrap())];
        let binds = vec![bind(64, "D", "Control Center"), bind(64, "D", "Nyx Shell: shell.control-center.toggle"), bind(65, "D", "x")];
        let c = conflicts(&wanted, &binds);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1.description, "Control Center");
        let submap = vec![Bind { submap: "resize".into(), ..bind(64, "D", "x") }];
        assert!(conflicts(&wanted, &submap).is_empty(), "other submaps don't clash");
    }

    #[test]
    fn duplicate_keys() {
        let wanted = vec![("a".to_string(), Shortcut::parse("SUPER + D").unwrap()), ("b".to_string(), Shortcut::parse("super+d").unwrap())];
        assert_eq!(duplicates(&wanted), vec![("a", "b")]);
    }

    #[test]
    fn the_file() {
        let wanted = vec![("shell.glimpse.toggle".to_string(), Shortcut::parse("SUPER + SPACE").unwrap())];
        let text = file_text(&wanted).unwrap();
        assert!(text.contains("hl.bind(\"SUPER + SPACE\", hl.dsp.exec_cmd(\"nyx-shellctl invoke shell.glimpse.toggle\")"));
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("hypr/nyx-shell.lua");
        write_atomic(&f, text.as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), text);
    }
}
