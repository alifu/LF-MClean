//! LF-MClean: an interactive cleaner for the caches of mobile developers on macOS.

#![warn(missing_docs)]

#[cfg(not(target_os = "macos"))]
compile_error!("lfmclean only supports macOS");

/// The fixed list of caches lfmclean knows about.
pub mod catalog;
/// The command line.
pub mod cli;
/// Running other programs.
pub mod command;
/// Home folder and user.
pub mod env;
/// Sizes and ages as people read them.
pub mod format;
/// Path checks before any removal.
pub mod guard;
/// Trash, permanent or dry run.
pub mod mode;
/// The removal log.
pub mod oplog;
/// Removing checked paths.
pub mod remove;
/// Finding and measuring caches.
pub mod scan;

#[cfg(test)]
mod testing;
