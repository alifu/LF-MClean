//! Path checks before any removal. `check` is the only way to get a `SafePath`, and `Remover`
//! only accepts a `SafePath`, so nothing unchecked can be removed.
//!
//! Layers: the root must be one of the fixed cache folders in `rules`; the entry must be
//! strictly inside it, on the same volume, owned by the user and not on the protected lists.

mod rules;

use std::fs::{self, Metadata};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::env::Env;

/// The first rule a path breaks.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardError {
    /// The path is relative.
    #[error("{0} is not an absolute path")]
    NotAbsolute(PathBuf),
    /// Nothing exists at the path.
    #[error("{0} does not exist")]
    NotFound(PathBuf),
    /// The path could not be read, for a reason other than not existing.
    #[error("cannot read {path}: {kind}")]
    Unreadable {
        /// The path that could not be read.
        path: PathBuf,
        /// What went wrong.
        kind: io::ErrorKind,
    },
    /// The path ends in `..` or has no last name.
    #[error("{0} has no file name")]
    NoFileName(PathBuf),
    /// The root is not one of the fixed cache folders.
    #[error("{0} is not one of the cache folders lfmclean cleans")]
    RootNotAllowed(PathBuf),
    /// The entry is not strictly inside the root.
    #[error("{path} is not inside {root}")]
    OutsideRoot {
        /// The resolved entry.
        path: PathBuf,
        /// The resolved root.
        root: PathBuf,
    },
    /// The entry is, contains or is inside a protected path.
    #[error("{0} is protected and is never removed")]
    Protected(PathBuf),
    /// The entry is a socket, FIFO or device.
    #[error("{0} is not a file, folder or symlink")]
    UnsupportedKind(PathBuf),
    /// The entry belongs to another user.
    #[error("{0} belongs to another user")]
    NotOwned(PathBuf),
    /// The entry is on another volume than its root (a mount point).
    #[error("{0} is on another volume")]
    OtherVolume(PathBuf),
    /// The path now leads to something other than what was checked.
    #[error("{0} changed since it was checked")]
    Changed(PathBuf),
}

/// What kind of entry a `SafePath` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A folder.
    Dir,
    /// A symlink, which is removed as a link.
    Symlink,
}

/// A device and inode pair: what a path pointed to when it was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    /// The device (volume).
    pub dev: u64,
    /// The inode on that device.
    pub ino: u64,
}

impl Identity {
    fn of(meta: &Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

/// A path that passed every check. Its fields are private so it can only be made by `check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafePath {
    path: PathBuf,
    root: PathBuf,
    kind: EntryKind,
    entry: Identity,
    parent: Identity,
}

impl SafePath {
    /// The resolved path, with its folders in on-disk form.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// File, folder or symlink.
    #[must_use]
    pub fn kind(&self) -> EntryKind {
        self.kind
    }

    /// The entry as checked; its `dev` is the volume a removal must not leave.
    #[must_use]
    pub fn entry(&self) -> Identity {
        self.entry
    }

    /// The folder holding the entry, as checked.
    #[must_use]
    pub fn parent(&self) -> Identity {
        self.parent
    }

    /// Checks again right before removal, and that the entry is still the one first checked.
    ///
    /// # Errors
    /// Any `check` error, or `Changed` when the path now leads somewhere else.
    pub fn revalidate(&self, env: &Env) -> Result<(), GuardError> {
        let now = check(&self.path, &self.root, env)?;
        if now != *self {
            return Err(GuardError::Changed(self.path.clone()));
        }
        Ok(())
    }
}

/// Checks that `path` may be removed as part of cleaning inside the cache folder `root`.
///
/// # Errors
/// A `GuardError` naming the first rule the path breaks.
pub fn check(path: &Path, root: &Path, env: &Env) -> Result<SafePath, GuardError> {
    if !path.is_absolute() {
        return Err(GuardError::NotAbsolute(path.to_path_buf()));
    }
    let home = canonical(env.home())?;
    let (root, root_meta) = checked_root(root, &home)?;
    let (resolved, meta) = resolve_entry(path)?;

    if !is_strictly_inside(&resolved, &root) {
        return Err(GuardError::OutsideRoot {
            path: resolved,
            root,
        });
    }
    if rules::is_protected(&resolved, &home) {
        return Err(GuardError::Protected(resolved));
    }
    let Some(kind) = kind_of(&meta) else {
        return Err(GuardError::UnsupportedKind(resolved));
    };
    if meta.uid() != env.uid() {
        return Err(GuardError::NotOwned(resolved));
    }
    if meta.dev() != root_meta.dev() {
        return Err(GuardError::OtherVolume(resolved));
    }
    let parent = parent_identity(&resolved)?;

    Ok(SafePath {
        path: resolved,
        root,
        kind,
        entry: Identity::of(&meta),
        parent,
    })
}

fn checked_root(root: &Path, home: &Path) -> Result<(PathBuf, Metadata), GuardError> {
    let root = canonical(root)?;
    if !rules::is_cache_root(&root, home) {
        return Err(GuardError::RootNotAllowed(root));
    }
    let meta = fs::metadata(&root).map_err(|err| fs_error(&root, &err))?;
    Ok((root, meta))
}

/// Resolves the folders above the entry. A symlink entry stays a link; any other entry is
/// resolved too, which gives its name in the on-disk letter case.
fn resolve_entry(path: &Path) -> Result<(PathBuf, Metadata), GuardError> {
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        return Err(GuardError::NoFileName(path.to_path_buf()));
    };
    let joined = canonical(parent)?.join(name);
    let meta = fs::symlink_metadata(&joined).map_err(|err| fs_error(path, &err))?;
    if meta.file_type().is_symlink() {
        return Ok((joined, meta));
    }
    Ok((canonical(&joined)?, meta))
}

fn parent_identity(resolved: &Path) -> Result<Identity, GuardError> {
    let parent = resolved
        .parent()
        .ok_or_else(|| GuardError::NoFileName(resolved.to_path_buf()))?;
    let meta = fs::symlink_metadata(parent).map_err(|err| fs_error(parent, &err))?;
    Ok(Identity::of(&meta))
}

fn canonical(path: &Path) -> Result<PathBuf, GuardError> {
    path.canonicalize().map_err(|err| fs_error(path, &err))
}

fn fs_error(path: &Path, err: &io::Error) -> GuardError {
    match err.kind() {
        io::ErrorKind::NotFound => GuardError::NotFound(path.to_path_buf()),
        kind => GuardError::Unreadable {
            path: path.to_path_buf(),
            kind,
        },
    }
}

/// Component-wise, so `/a/bc` is not inside `/a/b`. Both sides are canonical.
fn is_strictly_inside(child: &Path, parent: &Path) -> bool {
    child != parent && child.starts_with(parent)
}

/// Sockets, FIFOs and devices are refused.
fn kind_of(meta: &Metadata) -> Option<EntryKind> {
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        Some(EntryKind::Symlink)
    } else if file_type.is_dir() {
        Some(EntryKind::Dir)
    } else if file_type.is_file() {
        Some(EntryKind::File)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    use super::*;
    use crate::testing::Fixture;

    const DERIVED_DATA: &str = "Library/Developer/Xcode/DerivedData";
    const GRADLE_CACHES: &str = ".gradle/caches";

    #[test]
    fn accepts_folder_inside_root() {
        let fx = Fixture::new();
        let root = fx.dir(DERIVED_DATA);
        let item = fx.dir("Library/Developer/Xcode/DerivedData/App-abc");

        let safe = check(&item, &root, &fx.env).unwrap();

        assert_eq!(safe.kind(), EntryKind::Dir);
        assert!(safe.path().ends_with("DerivedData/App-abc"));
    }

    #[test]
    fn accepts_file_inside_root() {
        let fx = Fixture::new();
        let root = fx.dir(".gradle/daemon");
        let item = fx.file(".gradle/daemon/8.9/daemon.log");

        assert_eq!(
            check(&item, &root, &fx.env).unwrap().kind(),
            EntryKind::File
        );
    }

    #[test]
    fn returns_the_on_disk_letter_case() {
        let fx = Fixture::new();
        let root = fx.dir(DERIVED_DATA);
        fx.dir("Library/Developer/Xcode/DerivedData/App-abc");

        let safe = check(&root.join("APP-ABC"), &root, &fx.env).unwrap();

        assert!(safe.path().ends_with("DerivedData/App-abc"));
    }

    #[test]
    fn accepts_cache_root_written_in_other_case() {
        let fx = Fixture::new();
        fx.dir(DERIVED_DATA);
        let item = fx.dir("Library/Developer/Xcode/DerivedData/App-abc");
        let root = fx.home().join("library/developer/xcode/deriveddata");

        assert!(check(&item, &root, &fx.env).is_ok());
    }

    #[test]
    fn refuses_the_root_itself() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);

        assert!(matches!(
            check(&root, &root, &fx.env),
            Err(GuardError::OutsideRoot { .. })
        ));
    }

    #[test]
    fn refuses_sibling_outside_root() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let sibling = fx.dir(".gradle/caches-other");

        assert!(matches!(
            check(&sibling, &root, &fx.env),
            Err(GuardError::OutsideRoot { .. })
        ));
    }

    #[test]
    fn refuses_dot_dot_escape() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        fx.dir(".gradle/wrapper");

        assert!(matches!(
            check(&root.join("../wrapper"), &root, &fx.env),
            Err(GuardError::OutsideRoot { .. })
        ));
    }

    #[test]
    fn refuses_path_ending_in_dot_dot() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        fx.dir(".gradle/caches/modules");
        let sneaky = root.join("modules/..");

        assert_eq!(
            check(&sneaky, &root, &fx.env),
            Err(GuardError::NoFileName(sneaky))
        );
    }

    #[test]
    fn refuses_relative_path() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);

        assert_eq!(
            check(Path::new("caches/x"), &root, &fx.env),
            Err(GuardError::NotAbsolute(PathBuf::from("caches/x")))
        );
    }

    #[test]
    fn refuses_missing_path() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);

        assert!(matches!(
            check(&root.join("gone"), &root, &fx.env),
            Err(GuardError::NotFound(_))
        ));
    }

    #[test]
    fn reports_unreadable_paths_with_their_reason() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let file = fx.file(".gradle/caches/file.txt");

        assert!(matches!(
            check(&file.join("x"), &root, &fx.env),
            Err(GuardError::Unreadable {
                kind: io::ErrorKind::NotADirectory,
                ..
            })
        ));
    }

    #[test]
    fn refuses_roots_that_are_not_cache_folders() {
        let fx = Fixture::new();
        for rel in [
            "Documents",
            "Library",
            "Library/Caches",
            ".gradle",
            ".android",
        ] {
            let root = fx.dir(rel);
            let item = fx.dir(&format!("{rel}/inside"));
            assert!(
                matches!(
                    check(&item, &root, &fx.env),
                    Err(GuardError::RootNotAllowed(_))
                ),
                "{rel} must not be a root"
            );
        }
        let item = fx.dir("anything");
        assert!(matches!(
            check(&item, fx.home(), &fx.env),
            Err(GuardError::RootNotAllowed(_))
        ));
        assert!(matches!(
            check(&fx.outside.join("precious.txt"), &fx.outside, &fx.env),
            Err(GuardError::RootNotAllowed(_))
        ));
    }

    #[test]
    fn symlink_is_checked_as_the_link_not_its_target() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let link = root.join("link-out");
        symlink(&fx.outside, &link).unwrap();

        let safe = check(&link, &root, &fx.env).unwrap();

        assert_eq!(safe.kind(), EntryKind::Symlink);
        assert!(safe.path().ends_with("caches/link-out"));
    }

    #[test]
    fn refuses_path_through_symlinked_folder_pointing_outside() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        symlink(&fx.outside, root.join("link-out")).unwrap();

        assert!(matches!(
            check(&root.join("link-out/precious.txt"), &root, &fx.env),
            Err(GuardError::OutsideRoot { .. })
        ));
    }

    #[test]
    fn refuses_sockets() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let socket = root.join("s");
        let _listener = UnixListener::bind(&socket).unwrap();

        assert!(matches!(
            check(&socket, &root, &fx.env),
            Err(GuardError::UnsupportedKind(_))
        ));
    }

    #[test]
    fn refuses_entry_owned_by_another_user() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");

        assert!(matches!(
            check(&item, &root, &fx.other_user_env()),
            Err(GuardError::NotOwned(_))
        ));
    }

    #[test]
    fn revalidate_passes_when_nothing_changed() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let safe = check(&item, &root, &fx.env).unwrap();

        assert_eq!(safe.revalidate(&fx.env), Ok(()));
    }

    #[test]
    fn revalidate_fails_when_folder_was_swapped_for_a_symlink() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let safe = check(&item, &root, &fx.env).unwrap();

        fs::remove_dir_all(&item).unwrap();
        symlink(&fx.outside, &item).unwrap();

        assert!(matches!(
            safe.revalidate(&fx.env),
            Err(GuardError::Changed(_))
        ));
    }

    #[test]
    fn revalidate_fails_when_a_parent_now_leads_elsewhere() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/sub/item");
        fx.dir(".gradle/caches/other/item");
        let safe = check(&item, &root, &fx.env).unwrap();

        fs::remove_dir_all(root.join("sub")).unwrap();
        symlink(root.join("other"), root.join("sub")).unwrap();

        assert!(matches!(
            safe.revalidate(&fx.env),
            Err(GuardError::Changed(_))
        ));
    }
}
