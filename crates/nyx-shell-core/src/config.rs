//! The `shell` settings domain over nyx-config's layers.
//!
//! `/etc/nyxos/defaults/shell.toml` < `/etc/nyxos/overrides/shell.toml` <
//! `~/.config/nyx/shell.toml`. A layer that isn't valid TOML is skipped and reported;
//! a section that doesn't validate falls back to its defaults.

use nyx_config::{Layer, Layers};
use nyx_shell_proto::{ErrorBody, ErrorCode, ShellConfig};
use std::path::PathBuf;
use toml::{Table, Value};

pub const DOMAIN: &str = "shell";

#[derive(Debug, Clone)]
pub struct ConfigStore {
    pub layers: Layers,
}

impl ConfigStore {
    pub fn standard() -> ConfigStore {
        ConfigStore { layers: Layers::standard() }
    }

    /// The folders to watch.
    pub fn dirs(&self) -> Vec<PathBuf> {
        vec![self.layers.defaults.clone(), self.layers.system.clone(), self.layers.user.clone()]
    }

    fn merged(&self, errors: &mut Vec<String>) -> Table {
        let mut out = Table::new();
        for layer in [Layer::Defaults, Layer::System, Layer::User] {
            match self.layers.read(layer, DOMAIN) {
                Ok(t) => nyx_config::merge(&mut out, t),
                Err(e) => errors.push(format!("{} layer: {e:#}; skipped", layer.name())),
            }
        }
        out
    }

    /// The settings and the problems found in them.
    pub fn load(&self) -> (ShellConfig, Vec<String>) {
        let mut errors = Vec::new();
        let table = self.merged(&mut errors);
        let (config, more) = ShellConfig::from_table(&table);
        errors.extend(more);
        (config, errors)
    }

    /// Set `key` (dotted) in the user's layer, if the result validates. Names with dots
    /// in them (overlay ids) are set through their whole section (`overlays`).
    pub fn update(&self, key: &str, value: serde_json::Value) -> Result<(), ErrorBody> {
        let value = Value::try_from(value).map_err(|e| ErrorBody::new(ErrorCode::Invalid, format!("{key}: {e}")))?;
        let mut before_errors = Vec::new();
        let mut table = self.merged(&mut before_errors);
        let (_, before) = ShellConfig::from_table(&table);
        set_dotted(&mut table, key, value.clone()).map_err(|e| ErrorBody::new(ErrorCode::Invalid, e))?;
        let (_, after) = ShellConfig::from_table(&table);
        if let Some(new) = after.iter().find(|e| !before.contains(e)) {
            return Err(ErrorBody::new(ErrorCode::Invalid, new.clone()));
        }
        self.layers
            .set(Layer::User, DOMAIN, key, value)
            .map_err(|e| ErrorBody::new(ErrorCode::Invalid, format!("{e:#}")))
    }

    /// Remove the user's value for `key`, so the layer below shows through.
    pub fn reset(&self, key: &str) -> Result<(), ErrorBody> {
        self.layers.reset(Layer::User, DOMAIN, key).map_err(|e| ErrorBody::new(ErrorCode::Invalid, format!("{e:#}")))
    }
}

fn set_dotted(table: &mut Table, key: &str, value: Value) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if key.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("invalid settings key: {key}"));
    }
    let (last, parents) = parts.split_last().expect("not empty");
    let mut at = table;
    for p in parents {
        let entry = at.entry(p.to_string()).or_insert_with(|| Value::Table(Table::new()));
        let Value::Table(next) = entry else { return Err(format!("{key}: {p} isn't a table")) };
        at = next;
    }
    at.insert(last.to_string(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(dir: &std::path::Path) -> ConfigStore {
        ConfigStore { layers: Layers { defaults: dir.join("defaults"), system: dir.join("system"), user: dir.join("user") } }
    }

    #[test]
    fn layers_and_updates() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path());
        std::fs::create_dir_all(d.path().join("defaults")).unwrap();
        std::fs::write(d.path().join("defaults/shell.toml"), "[focus]\nrestore = false\n[ipc]\nmax_queue = 128\n").unwrap();
        let (c, e) = s.load();
        assert!(e.is_empty());
        assert!(!c.focus.restore);
        s.update("focus.restore", json!(true)).unwrap();
        let (c, _) = s.load();
        assert!(c.focus.restore, "the user's layer wins");
        assert_eq!(c.ipc.max_queue, 128);
        let bad = s.update("ipc.max_queue", json!(1)).unwrap_err();
        assert_eq!(bad.code, ErrorCode::Invalid);
        assert_eq!(s.load().0.ipc.max_queue, 128, "nothing written");
        assert_eq!(s.update("focus.restore", json!(null)).unwrap_err().code, ErrorCode::Invalid);
        s.reset("focus.restore").unwrap();
        assert!(!s.load().0.focus.restore, "back to the default layer");
    }

    #[test]
    fn whole_sections_for_dotted_names() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path());
        s.update("overlays", json!({"nyx.inspector": {"fullscreen": "block"}})).unwrap();
        let (c, e) = s.load();
        assert!(e.is_empty(), "{e:?}");
        assert!(c.overlays.contains_key("nyx.inspector"));
        assert_eq!(s.update("shortcuts", json!({"shell.x": "NOPE + D"})).unwrap_err().code, ErrorCode::Invalid);
    }

    #[test]
    fn a_broken_layer_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path());
        std::fs::create_dir_all(d.path().join("user")).unwrap();
        std::fs::write(d.path().join("user/shell.toml"), "this is = = not toml").unwrap();
        let (c, e) = s.load();
        assert_eq!(c, ShellConfig::default());
        assert_eq!(e.len(), 1);
        assert!(e[0].starts_with("user layer"));
    }
}
