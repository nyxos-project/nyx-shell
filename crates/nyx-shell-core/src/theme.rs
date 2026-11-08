//! Theme events, watched once for the whole shell.
//!
//! The source is Theme Engine 2.0's snapshot (`$XDG_RUNTIME_DIR/nyx/theme/system.json`,
//! written by `nyx-themed`, announced with `ThemeChanged`): the resolved theme, read and
//! never parsed again here. Without the engine, the theme choices are read directly
//! (and resolved with `nyx-theme` in builds with the `palette` feature). The generation
//! moves only when the inputs' contents change, so rewriting a file with the same
//! contents is no change.

use nyx_shell_proto::ThemeState;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ThemeSource {
    /// `~/.config/nyx/theme.toml`: the suite's choice.
    pub suite_file: PathBuf,
    /// `~/.config/nyx/themes`: the user's theme files.
    pub themes_dir: PathBuf,
    /// `~/.local/state/omarchy/current/theme`: what Follow Omarchy reads.
    pub omarchy_dir: PathBuf,
    /// Theme Engine 2.0's snapshot of the resolved system theme.
    pub snapshot_file: PathBuf,
}

/// The snapshot format this reads (Theme Engine 2.0's `SNAPSHOT_FORMAT`).
pub const SNAPSHOT_FORMAT: u64 = 1;

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

impl ThemeSource {
    pub fn standard() -> ThemeSource {
        let suite = xdg("XDG_CONFIG_HOME", ".config").join("nyx");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute());
        ThemeSource {
            suite_file: suite.join("theme.toml"),
            themes_dir: suite.join("themes"),
            omarchy_dir: xdg("XDG_STATE_HOME", ".local/state").join("omarchy/current/theme"),
            snapshot_file: runtime
                .map(|r| r.join("nyx/theme/system.json"))
                .unwrap_or_else(|| xdg("XDG_CACHE_HOME", ".cache").join("nyx/theme-runtime/system.json")),
        }
    }

    /// Folders to watch: the suite folder, the themes folder, and Omarchy's `current`
    /// (its `theme` is a link that is replaced on a theme change) and the theme itself.
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut out = vec![self.themes_dir.clone()];
        if let Some(p) = self.suite_file.parent() {
            out.push(p.to_path_buf());
        }
        if let Some(p) = self.omarchy_dir.parent() {
            out.push(p.to_path_buf());
        }
        out.push(self.omarchy_dir.clone());
        if let Some(dir) = self.snapshot_file.parent() {
            out.push(dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                out.push(parent.to_path_buf());
            }
        }
        out
    }

    /// A hash of everything the theme depends on.
    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        std::fs::read(&self.snapshot_file).ok().hash(&mut h);
        std::fs::read(&self.suite_file).ok().hash(&mut h);
        std::fs::read(self.omarchy_dir.join("colors.toml")).ok().hash(&mut h);
        std::fs::read(self.omarchy_dir.join("theme.name")).ok().hash(&mut h);
        let mut themes: Vec<(String, Option<Vec<u8>>)> = std::fs::read_dir(&self.themes_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).ok()))
            .collect();
        themes.sort();
        themes.hash(&mut h);
        h.finish()
    }

    /// The theme now, as generation `generation`.
    pub fn read(&self, generation: u64) -> ThemeState {
        if let Some(t) = self.read_snapshot(generation) {
            return t;
        }
        self.read_files(generation)
    }

    /// Theme Engine 2.0's snapshot, if there is one in a format this knows.
    fn read_snapshot(&self, generation: u64) -> Option<ThemeState> {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&self.snapshot_file).ok()?).ok()?;
        if v.get("format").and_then(|f| f.as_u64()) != Some(SNAPSHOT_FORMAT) {
            tracing::warn!(file = %self.snapshot_file.display(), "theme snapshot in a format this doesn't know; reading the theme files");
            return None;
        }
        let theme = v.get("theme")?;
        let text = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let palette = v.get("legacy")?.as_object()?.iter().filter(|(k, _)| *k != "light").filter_map(|(k, c)| Some((k.clone(), c.as_str()?.to_string()))).collect();
        let suite: toml::Table = std::fs::read_to_string(&self.suite_file).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
        Some(ThemeState {
            generation,
            mode: suite.get("mode").and_then(|m| m.as_str()).unwrap_or("theme").to_string(),
            id: text(theme, "id"),
            name: text(theme, "name"),
            light: text(theme, "appearance") == "light",
            palette: Some(palette),
            source: "theme-engine".into(),
            engine_generation: v.get("generation").and_then(|g| g.as_u64()),
        })
    }

    /// The theme choices read directly, when the engine isn't running.
    fn read_files(&self, generation: u64) -> ThemeState {
        let suite: toml::Table = std::fs::read_to_string(&self.suite_file).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
        let get = |k: &str| suite.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let mode = get("mode").unwrap_or_else(|| "omarchy".into());
        let mut state = ThemeState {
            generation,
            id: get("theme").unwrap_or_else(|| "tokyo-night".into()),
            name: String::new(),
            light: false,
            palette: None,
            mode,
            source: "files".into(),
            engine_generation: None,
        };
        if state.mode == "omarchy" {
            let colors: toml::Table = std::fs::read_to_string(self.omarchy_dir.join("colors.toml")).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
            if !colors.is_empty() {
                state.id = "omarchy".into();
                state.light = colors.get("mode").and_then(|v| v.as_str()) == Some("light");
            }
        }
        palette(&mut state);
        if state.name.is_empty() {
            state.name = state.id.clone();
        }
        state
    }
}

#[cfg(feature = "palette")]
fn palette(state: &mut ThemeState) {
    let theme = nyx_theme::suite_theme("shell");
    let p = &theme.palette;
    let colors = [
        ("bg", &p.bg),
        ("surface", &p.surface),
        ("text", &p.text),
        ("dim_text", &p.dim_text),
        ("accent", &p.accent),
        ("border", &p.border),
        ("muted", &p.muted),
        ("highlight", &p.highlight),
        ("danger", &p.danger),
        ("glow", &p.glow),
        ("shadow", &p.shadow),
    ];
    state.id = theme.id.clone();
    state.name = theme.name.clone();
    state.light = p.light;
    state.palette = Some(colors.into_iter().map(|(k, v)| (k.to_string(), v.clone())).collect());
}

#[cfg(not(feature = "palette"))]
fn palette(_: &mut ThemeState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &std::path::Path) -> ThemeSource {
        ThemeSource {
            suite_file: dir.join("nyx/theme.toml"),
            themes_dir: dir.join("nyx/themes"),
            omarchy_dir: dir.join("omarchy/current/theme"),
            snapshot_file: dir.join("run/nyx/theme/system.json"),
        }
    }

    #[test]
    fn fingerprint_follows_contents() {
        let d = tempfile::tempdir().unwrap();
        let s = source(d.path());
        std::fs::create_dir_all(d.path().join("nyx/themes")).unwrap();
        let empty = s.fingerprint();
        std::fs::write(&s.suite_file, "mode = \"theme\"\ntheme = \"nyx-dark\"\n").unwrap();
        let one = s.fingerprint();
        assert_ne!(empty, one);
        std::fs::write(&s.suite_file, "mode = \"theme\"\ntheme = \"nyx-dark\"\n").unwrap();
        assert_eq!(one, s.fingerprint(), "the same contents written again is no change");
        std::fs::write(d.path().join("nyx/themes/mine.toml"), "x = 1").unwrap();
        assert_ne!(one, s.fingerprint());
    }

    #[test]
    fn reads_identity() {
        let d = tempfile::tempdir().unwrap();
        let s = source(d.path());
        std::fs::create_dir_all(d.path().join("nyx")).unwrap();
        std::fs::write(&s.suite_file, "mode = \"theme\"\ntheme = \"nyx-light\"\n").unwrap();
        let t = s.read(3);
        assert_eq!(t.generation, 3);
        assert_eq!(t.mode, "theme");
        assert_eq!(t.source, "files");
        assert!(!t.name.is_empty());
    }

    #[test]
    fn theme_engine_snapshot_wins() {
        let d = tempfile::tempdir().unwrap();
        let s = source(d.path());
        std::fs::create_dir_all(s.snapshot_file.parent().unwrap()).unwrap();
        let snapshot = r##"{"format":1,"schema":2,"generation":41,"scope":"system","reason":"set",
            "theme":{"id":"catppuccin-mocha","name":"Catppuccin Mocha","appearance":"dark","origin":"bundled"},
            "tokens":{},"legacy":{"bg":"#1e1e2e","text":"#cdd6f4","accent":"#cba6f7","light":"false"},
            "swatches":{},"accents":{},"issues":[]}"##;
        std::fs::write(&s.snapshot_file, snapshot).unwrap();
        let before = s.fingerprint();
        let t = s.read(7);
        assert_eq!((t.generation, t.engine_generation), (7, Some(41)));
        assert_eq!(t.source, "theme-engine");
        assert_eq!(t.id, "catppuccin-mocha");
        assert!(!t.light);
        let p = t.palette.unwrap();
        assert_eq!(p["accent"], "#cba6f7");
        assert!(!p.contains_key("light"));
        std::fs::write(&s.snapshot_file, snapshot.replace("\"format\":1", "\"format\":9")).unwrap();
        assert_ne!(before, s.fingerprint());
        assert_eq!(s.read(8).source, "files", "an unknown format isn't guessed at");
    }
}
