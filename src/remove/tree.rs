//! Permanent removal through open folder handles instead of paths.
//!
//! Below the pinned parent every step is relative to an open handle, so a folder swapped for a
//! symlink mid-way is never followed: symlinks are removed as links, and a folder on another
//! volume (a mount point) stops the removal before anything in it is touched.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::fs::{
    AtFlags, Dir, FileType, Mode as FileMode, OFlags, Stat, fstat, openat, statat, unlinkat,
};

use super::pinned::{changed, identity, pin};
use crate::guard::SafePath;

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// # Errors
/// I/O errors, or `Other` when the tree changed since it was checked or crosses onto another
/// volume. Entries removed before the error stay removed.
pub(super) fn remove_permanently(target: &SafePath) -> io::Result<()> {
    let pinned = pin(target)?;
    remove_at(pinned.parent.as_fd(), &pinned.name, target.entry().dev)
}

/// Removes `name` inside the open folder `parent`, staying on volume `dev`.
fn remove_at(parent: BorrowedFd<'_>, name: &CStr, dev: u64) -> io::Result<()> {
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if !is_dir(&stat) {
        return Ok(unlinkat(parent, name, AtFlags::empty())?);
    }
    let found = identity(&stat);
    if found.dev != dev {
        return Err(io::Error::other(format!(
            "{} is on another volume (a mount point); it was left in place",
            name.to_string_lossy()
        )));
    }
    let dir = openat(parent, name, DIR_FLAGS, FileMode::empty())?;
    if identity(&fstat(&dir)?) != found {
        return Err(changed(&name.to_string_lossy()));
    }
    for child in child_names(&dir)? {
        remove_at(dir.as_fd(), &child, dev)?;
    }
    Ok(unlinkat(parent, name, AtFlags::REMOVEDIR)?)
}

fn child_names(dir: &OwnedFd) -> io::Result<Vec<CString>> {
    let mut reader = Dir::read_from(dir)?;
    let mut names = Vec::new();
    while let Some(entry) = reader.read() {
        let name = entry?.file_name().to_owned();
        if !matches!(name.as_bytes(), b"." | b"..") {
            names.push(name);
        }
    }
    Ok(names)
}

fn is_dir(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode) == FileType::Directory
}
