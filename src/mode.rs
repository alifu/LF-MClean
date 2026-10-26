//! How a removal is done.

use serde::{Deserialize, Serialize};

/// What a removal does with the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Checks only; nothing is removed or logged.
    DryRun,
    /// Moves to `~/.Trash`, where it can be dragged back. The default.
    Trash,
    /// Removes for good.
    Permanent,
}
