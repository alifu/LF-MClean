//! Moves a checked entry into `~/.Trash`, where it can be dragged back.
//!
//! The entry is moved out of its pinned parent folder, so exactly the checked entry moves. The
//! move never replaces anything already in the Trash: a name that is taken gets the time added,
//! the way Finder does it. Only renaming into the Trash is needed, which macOS allows without
//! Full Disk Access (reading the Trash folder is what needs it).

use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;

use super::pinned::pin;
use crate::env::Env;
use crate::guard::SafePath;

const TRASH_DIR: &str = ".Trash";
const TRASH_DIR_MODE: u32 = 0o700;
const MAX_NAME_ATTEMPTS: u32 = 100;

/// # Errors
/// I/O errors: the Trash folder is not usable, the entry changed since it was checked, it is
/// on another volume than the Trash, or no free name was found.
pub(super) fn move_to_trash(
    target: &SafePath,
    env: &Env,
    now: DateTime<FixedOffset>,
) -> io::Result<PathBuf> {
    let trash = trash_dir(env)?;
    let pinned = pin(target)?;
    let name = target
        .path()
        .file_name()
        .ok_or_else(|| io::Error::other("path has no file name"))?;
    for attempt in 0..MAX_NAME_ATTEMPTS {
        let destination = trash.join(candidate_name(name, now, attempt));
        match renameat_with(
            &pinned.parent,
            &pinned.name,
            CWD,
            &destination,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => return Ok(destination),
            Err(Errno::EXIST) => {}
            Err(Errno::XDEV) => {
                return Err(io::Error::other(
                    "it is on another volume than your Trash; use --permanent to remove it",
                ));
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(io::Error::other("no free name in the Trash"))
}

/// The user's Trash folder: made if missing, refused if it is a symlink or someone else's.
fn trash_dir(env: &Env) -> io::Result<PathBuf> {
    let trash = env.home().join(TRASH_DIR);
    let meta = match fs::symlink_metadata(&trash) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            DirBuilder::new().mode(TRASH_DIR_MODE).create(&trash)?;
            fs::symlink_metadata(&trash)?
        }
        Err(err) => return Err(err),
    };
    if !meta.file_type().is_dir() || meta.uid() != env.uid() {
        return Err(io::Error::other(format!(
            "{} is not your own Trash folder",
            trash.display()
        )));
    }
    Ok(trash)
}

/// `name`, then `name 15.20.31`, then `name 15.20.31 2` and so on, keeping any extension last.
fn candidate_name(name: &std::ffi::OsStr, now: DateTime<FixedOffset>, attempt: u32) -> OsString {
    if attempt == 0 {
        return name.to_os_string();
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy();
    let time = now.format("%H.%M.%S");
    let counter = if attempt == 1 {
        String::new()
    } else {
        format!(" {attempt}")
    };
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    OsString::from(format!("{stem} {time}{counter}{extension}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use chrono::TimeZone;

    use super::*;

    fn at_15_20_31() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(7 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 26, 15, 20, 31)
            .unwrap()
    }

    #[test]
    fn first_choice_is_the_original_name() {
        assert_eq!(
            candidate_name(OsStr::new("App-abc"), at_15_20_31(), 0),
            "App-abc"
        );
    }

    #[test]
    fn taken_names_get_the_time_then_a_counter() {
        let now = at_15_20_31();
        assert_eq!(
            candidate_name(OsStr::new("App-abc"), now, 1),
            "App-abc 15.20.31"
        );
        assert_eq!(
            candidate_name(OsStr::new("App-abc"), now, 3),
            "App-abc 15.20.31 3"
        );
    }

    #[test]
    fn the_extension_stays_last() {
        assert_eq!(
            candidate_name(OsStr::new("daemon.log"), at_15_20_31(), 1),
            "daemon 15.20.31.log"
        );
        assert_eq!(
            candidate_name(OsStr::new(".konan"), at_15_20_31(), 1),
            ".konan 15.20.31"
        );
    }
}
