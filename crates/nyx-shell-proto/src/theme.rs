//! Theme events. Theme Engine 2.0 (Handoff 02) owns tokens; this carries what changed.

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
    /// Core colours by token (`bg`, `surface`, `text`, `accent`, …); `None` in builds
    /// without the `palette` feature.
    pub palette: Option<BTreeMap<String, String>>,
}
