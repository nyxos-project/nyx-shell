//! Theme events: the inputs of the suite theme, watched once for the whole shell.
//!
//! The generation moves only when the inputs' contents change, so rewriting a file with
//! the same contents is no change. Theme parsing stays in `nyx-theme` (the `palette`
//! feature); Theme Engine 2.0 replaces this source in Handoff 02.

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
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

impl ThemeSource {
    pub fn standard() -> ThemeSource {
        let suite = xdg("XDG_CONFIG_HOME", ".config").join("nyx");
        ThemeSource {
            suite_file: suite.join("theme.toml"),
            themes_dir: suite.join("themes"),
            omarchy_dir: xdg("XDG_STATE_HOME", ".local/state").join("omarchy/current/theme"),
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
        out
    }

    /// A hash of everything the theme depends on.
    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
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
        ThemeSource { suite_file: dir.join("nyx/theme.toml"), themes_dir: dir.join("nyx/themes"), omarchy_dir: dir.join("omarchy/current/theme") }
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
        assert!(!t.name.is_empty());
    }
}
