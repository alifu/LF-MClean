//! Opens the folder holding a checked entry so the removal works on that exact folder, never on
//! whatever a path happens to lead to by then.

use std::ffi::CString;
use std::fmt::Display;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, CWD, Mode as FileMode, OFlags, Stat, fstat, openat, statat};

use crate::guard::{Identity, SafePath};

/// No symlink anywhere in the path. macOS refuses `NOFOLLOW_ANY` together with `NOFOLLOW`
/// (`EINVAL`), and it already covers the last component.
const PARENT_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW_ANY);

/// The checked entry's folder, opened, and the entry's name inside it.
pub(super) struct Pinned {
    pub parent: OwnedFd,
    pub name: CString,
}

/// Opens the entry's folder and checks that both the folder and the entry inside it are still
/// the ones that were checked.
///
/// # Errors
/// I/O errors, or `Other` when either one changed.
pub(super) fn pin(target: &SafePath) -> io::Result<Pinned> {
    let path = target.path();
    let (Some(parent_path), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path has no parent",
        ));
    };
    let parent = openat(CWD, parent_path, PARENT_FLAGS, FileMode::empty())?;
    if identity(&fstat(&parent)?) != target.parent() {
        return Err(changed(&parent_path.display()));
    }
    let name = CString::new(name.as_bytes()).map_err(io::Error::other)?;
    if identity(&statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)?) != target.entry() {
        return Err(changed(&path.display()));
    }
    Ok(Pinned { parent, name })
}

/// Matches `std::os::unix::fs::MetadataExt`, which `guard` uses.
#[allow(clippy::cast_sign_loss)]
pub(super) fn identity(stat: &Stat) -> Identity {
    Identity {
        dev: stat.st_dev as u64,
        ino: stat.st_ino,
    }
}

pub(super) fn changed(what: &dyn Display) -> io::Error {
    io::Error::other(format!(
        "{what} changed since it was checked; it was left in place"
    ))
}
