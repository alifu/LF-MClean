//! LF-MClean: an interactive cleaner for the caches of mobile developers on macOS.

#![warn(missing_docs)]

#[cfg(not(target_os = "macos"))]
compile_error!("lfmclean only supports macOS");

/// Home folder and user.
pub mod env;
/// Path checks before any removal.
pub mod guard;
/// Trash, permanent or dry run.
pub mod mode;
/// The removal log.
pub mod oplog;
/// Removing checked paths.
pub mod remove;

#[cfg(test)]
mod testing;
