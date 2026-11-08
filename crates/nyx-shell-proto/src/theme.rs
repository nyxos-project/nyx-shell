//! Theme events. Theme Engine 2.0 owns themes and tokens; this carries what changed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ThemeState {
    /// Bumped whenever the theme's inputs change.
    pub generation: u64,
    /// `omarchy` (follow Omarchy) or `theme`.
    pub mode: String,
    pub id: String,
    pub name: String,
    pub light: bool,
    /// Colours by their established names (`bg`, `surface`, `text`, `dim_text`, `accent`,
    /// …): from Theme Engine 2.0's snapshot, else (builds with the `palette` feature)
    /// resolved from the theme files; `None` otherwise.
    pub palette: Option<BTreeMap<String, String>>,
    /// `theme-engine` (Theme Engine 2.0's snapshot) or `files` (the theme choices read
    /// directly, when the engine isn't running).
    #[serde(default)]
    pub source: String,
    /// The engine's own generation, when the snapshot is the source.
    #[serde(default)]
    pub engine_generation: Option<u64>,
}
