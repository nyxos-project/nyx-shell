//! Shell actions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub id: String,
    pub title: String,
    /// `panel1:BUSNAME`, `socket:PATH`, `launch:PROGRAM`, `overlay:ID` or `component:ID`.
    pub owner: String,
    pub available: bool,
}

/// Where an invocation went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routed {
    pub routed: String,
}

/// Largest `args` object accepted by `InvokeAction`, serialized.
pub const MAX_ARGS: usize = 4096;
