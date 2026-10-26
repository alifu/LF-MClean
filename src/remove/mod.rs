//! Removes checked paths: to the Trash (default), for good, or not at all (dry run).
//!
//! Every real removal is logged twice: a `started` line before (if that cannot be written,
//! nothing is removed) and a `finished` line after. Both ways of removing work on the checked
//! entry through its pinned parent folder, never through a path that may have changed.

mod pinned;
mod trash;
mod tree;

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Local};
use thiserror::Error;

use crate::env::Env;
use crate::guard::{GuardError, SafePath};
use crate::mode::Mode;
use crate::oplog::{LogEntry, OpLog};

/// Why a removal did not happen, or did not fully happen.
#[derive(Debug, Error)]
pub enum RemoveError {
    /// The path no longer passes the checks; nothing was removed.
    #[error(transparent)]
    Guard(#[from] GuardError),
    /// The log could not be written first; nothing was removed.
    #[error("nothing was removed: the log could not be written before removing {path}: {source}")]
    NotLogged {
        /// The entry that was to be removed.
        path: PathBuf,
        /// Why the log write failed.
        #[source]
        source: io::Error,
    },
    /// Moving to the Trash failed; nothing was moved.
    #[error("could not move {path} to the Trash: {source}")]
    Trash {
        /// The entry that was to be moved.
        path: PathBuf,
        /// Why the move failed.
        #[source]
        source: io::Error,
    },
    /// Removing for good failed, possibly after part of a folder was removed.
    #[error("could not remove {path} (part of it may be gone): {source}")]
    Io {
        /// The entry that was being removed.
        path: PathBuf,
        /// Why the removal stopped.
        #[source]
        source: io::Error,
    },
    /// The removal worked, but its result could not be logged.
    #[error("{path} was removed, but its result could not be written to the log: {source}")]
    ResultNotLogged {
        /// The entry that was removed.
        path: PathBuf,
        /// Why the log write failed.
        #[source]
        source: io::Error,
    },
    /// The removal failed, and so did logging that.
    #[error("{removal}; its result could not be written to the log either: {source}")]
    FailedAndNotLogged {
        /// The removal error.
        removal: Box<RemoveError>,
        /// Why the log write failed.
        #[source]
        source: io::Error,
    },
}

/// Removes checked entries in one `Mode`, logging each one.
pub struct Remover<'a> {
    env: &'a Env,
    log: &'a OpLog,
    mode: Mode,
}

impl<'a> Remover<'a> {
    /// A remover for `env` that records to `log`.
    #[must_use]
    pub fn new(env: &'a Env, log: &'a OpLog, mode: Mode) -> Self {
        Self { env, log, mode }
    }

    /// How this remover removes.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Checks the path again, logs, removes and logs the result. A dry run only checks.
    ///
    /// # Errors
    /// A `RemoveError` saying what failed and whether anything was removed.
    pub fn remove(&self, target: &SafePath, size_bytes: u64) -> Result<(), RemoveError> {
        target.revalidate(self.env)?;
        if self.mode == Mode::DryRun {
            return Ok(());
        }
        let path = target.path();
        let started = now();
        self.log
            .append(&LogEntry::started(started, path, self.mode, size_bytes))
            .map_err(|source| RemoveError::NotLogged {
                path: path.to_path_buf(),
                source,
            })?;
        let result = self.run(target, started);
        let error = result.as_ref().err().map(ToString::to_string);
        let logged = self.log.append(&LogEntry::finished(
            now(),
            path,
            self.mode,
            size_bytes,
            error,
        ));
        combine(path, result, logged)
    }

    fn run(&self, target: &SafePath, now: DateTime<FixedOffset>) -> Result<(), RemoveError> {
        let path = target.path().to_path_buf();
        match self.mode {
            Mode::DryRun => Ok(()),
            Mode::Trash => trash::move_to_trash(target, self.env, now)
                .map(drop)
                .map_err(|source| RemoveError::Trash { path, source }),
            Mode::Permanent => {
                tree::remove_permanently(target).map_err(|source| RemoveError::Io { path, source })
            }
        }
    }
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// Reports both failures when the removal and the log write both fail.
fn combine(
    path: &Path,
    result: Result<(), RemoveError>,
    logged: io::Result<()>,
) -> Result<(), RemoveError> {
    match (result, logged) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(removal), Ok(())) => Err(removal),
        (Ok(()), Err(source)) => Err(RemoveError::ResultNotLogged {
            path: path.to_path_buf(),
            source,
        }),
        (Err(removal), Err(source)) => Err(RemoveError::FailedAndNotLogged {
            removal: Box::new(removal),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    use super::*;
    use crate::guard::check;
    use crate::oplog::Stage;
    use crate::testing::Fixture;

    const SIZE: u64 = 1234;
    const GRADLE_CACHES: &str = ".gradle/caches";
    const READ_ONLY_DIR: u32 = 0o555;
    const WRITABLE_DIR: u32 = 0o755;
    const READ_ONLY_FILE: u32 = 0o444;

    fn open_log(fx: &Fixture) -> OpLog {
        OpLog::open(&fx.env).unwrap()
    }

    /// Checks `item` inside `root` and removes it in `mode`.
    fn remove(
        fx: &Fixture,
        log: &OpLog,
        mode: Mode,
        root: &Path,
        item: &Path,
    ) -> Result<(), RemoveError> {
        let safe = check(item, root, &fx.env).unwrap();
        Remover::new(&fx.env, log, mode).remove(&safe, SIZE)
    }

    #[test]
    fn dry_run_removes_nothing_and_logs_nothing() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let log = open_log(&fx);

        remove(&fx, &log, Mode::DryRun, &root, &item).unwrap();

        assert!(item.is_dir());
        assert_eq!(log.read_all().unwrap().entries, vec![]);
    }

    #[test]
    fn permanent_removes_the_folder_and_logs_before_and_after() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let deep = fx.dir(".gradle/caches/modules-2/files/deep");
        let top = root.join("modules-2");
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Permanent, &root, &top).unwrap();

        assert!(!deep.exists());
        assert!(!top.exists());
        assert!(root.is_dir());
        let entries = log.read_all().unwrap().entries;
        let stages: Vec<Stage> = entries.iter().map(|entry| entry.stage).collect();
        assert_eq!(stages, vec![Stage::Started, Stage::Finished]);
        assert_eq!(entries[1].mode, Mode::Permanent);
        assert_eq!(entries[1].size_bytes, SIZE);
        assert_eq!(entries[1].error, None);
        assert!(entries[1].path.ends_with("caches/modules-2"));
    }

    #[test]
    fn permanent_removes_a_file() {
        let fx = Fixture::new();
        let root = fx.dir(".gradle/daemon");
        let item = fx.file(".gradle/daemon/daemon.log");
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Permanent, &root, &item).unwrap();

        assert!(!item.exists());
    }

    #[test]
    fn permanent_removes_a_symlink_but_not_its_target() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let link = root.join("link-out");
        symlink(&fx.outside, &link).unwrap();
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Permanent, &root, &link).unwrap();

        assert!(fs::symlink_metadata(&link).is_err());
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn permanent_folder_with_a_symlink_inside_keeps_the_target() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        symlink(&fx.outside, item.join("link-out")).unwrap();
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Permanent, &root, &item).unwrap();

        assert!(!item.exists());
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn refuses_when_the_entry_changed_after_the_check() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let log = open_log(&fx);
        let safe = check(&item, &root, &fx.env).unwrap();
        fs::remove_dir_all(&item).unwrap();
        symlink(&fx.outside, &item).unwrap();

        let result = Remover::new(&fx.env, &log, Mode::Permanent).remove(&safe, SIZE);

        assert!(matches!(
            result,
            Err(RemoveError::Guard(GuardError::Changed(_)))
        ));
        assert!(fs::symlink_metadata(&item).is_ok());
        assert!(fx.outside_is_intact());
        assert_eq!(log.read_all().unwrap().entries, vec![]);
    }

    /// Skips `revalidate`, as if the swap happened just after it.
    #[test]
    fn pinning_refuses_an_entry_swapped_after_the_last_check() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let safe = check(&item, &root, &fx.env).unwrap();
        fs::remove_dir_all(&item).unwrap();
        symlink(&fx.outside, &item).unwrap();

        assert!(tree::remove_permanently(&safe).is_err());
        assert!(trash::move_to_trash(&safe, &fx.env, now()).is_err());
        assert!(
            fs::symlink_metadata(&item)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn pinning_refuses_a_parent_swapped_after_the_last_check() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/sub/item");
        fx.dir(".gradle/caches/other/item");
        let safe = check(&item, &root, &fx.env).unwrap();
        fs::rename(root.join("sub"), root.join("sub-old")).unwrap();
        fs::rename(root.join("other"), root.join("sub")).unwrap();

        assert!(tree::remove_permanently(&safe).is_err());
        assert!(trash::move_to_trash(&safe, &fx.env, now()).is_err());
        assert!(root.join("sub/item/data.bin").is_file());
        assert!(root.join("sub-old/item/data.bin").is_file());
    }

    #[test]
    fn nothing_is_removed_when_the_log_cannot_be_written() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let log = open_log(&fx);
        fs::set_permissions(log.path(), fs::Permissions::from_mode(READ_ONLY_FILE)).unwrap();

        let result = remove(&fx, &log, Mode::Permanent, &root, &item);

        assert!(matches!(result, Err(RemoveError::NotLogged { .. })));
        assert!(item.is_dir());
    }

    #[test]
    fn records_a_failed_removal() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let log = open_log(&fx);
        fs::set_permissions(&item, fs::Permissions::from_mode(READ_ONLY_DIR)).unwrap();

        let result = remove(&fx, &log, Mode::Permanent, &root, &item);

        fs::set_permissions(&item, fs::Permissions::from_mode(WRITABLE_DIR)).unwrap();
        assert!(matches!(result, Err(RemoveError::Io { .. })));
        let entries = log.read_all().unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert!(entries[1].error.is_some());
    }

    #[test]
    fn trash_moves_the_folder_into_the_trash_folder() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Trash, &root, &item).unwrap();

        assert!(!item.exists());
        assert!(fx.home().join(".Trash/modules-2/data.bin").is_file());
        assert_eq!(log.read_all().unwrap().entries[1].mode, Mode::Trash);
    }

    #[test]
    fn trash_moves_a_symlink_but_not_its_target() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let link = root.join("link-out");
        symlink(&fx.outside, &link).unwrap();
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Trash, &root, &link).unwrap();

        assert!(fs::symlink_metadata(&link).is_err());
        let trashed = fs::symlink_metadata(fx.home().join(".Trash/link-out")).unwrap();
        assert!(trashed.file_type().is_symlink());
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn trash_never_replaces_an_item_already_there() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        let earlier = fx.dir(".Trash/modules-2");
        fs::write(earlier.join("data.bin"), "earlier").unwrap();
        let log = open_log(&fx);

        remove(&fx, &log, Mode::Trash, &root, &item).unwrap();

        assert_eq!(
            fs::read_to_string(earlier.join("data.bin")).unwrap(),
            "earlier"
        );
        let names: Vec<String> = fs::read_dir(fx.home().join(".Trash"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|name| name.starts_with("modules-2 ")));
    }

    #[test]
    fn trash_refuses_a_trash_folder_that_is_a_symlink() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/modules-2");
        symlink(&fx.outside, fx.home().join(".Trash")).unwrap();
        let log = open_log(&fx);

        let result = remove(&fx, &log, Mode::Trash, &root, &item);

        assert!(matches!(result, Err(RemoveError::Trash { .. })));
        assert!(item.is_dir());
        assert!(!fx.outside.join("modules-2").exists());
    }

    #[test]
    fn combine_reports_every_failure() {
        let path = Path::new("/x");
        let removal = || {
            Err(RemoveError::Trash {
                path: path.to_path_buf(),
                source: io::Error::other("busy"),
            })
        };
        let log_failure = || Err(io::Error::other("disk full"));

        assert!(combine(path, Ok(()), Ok(())).is_ok());
        assert!(matches!(
            combine(path, removal(), Ok(())),
            Err(RemoveError::Trash { .. })
        ));
        assert!(matches!(
            combine(path, Ok(()), log_failure()),
            Err(RemoveError::ResultNotLogged { .. })
        ));
        assert!(matches!(
            combine(path, removal(), log_failure()),
            Err(RemoveError::FailedAndNotLogged { .. })
        ));
    }

    #[test]
    #[ignore = "mounts a small disk image with hdiutil"]
    fn permanent_stops_at_a_mount_point() {
        let fx = Fixture::new();
        let root = fx.dir(GRADLE_CACHES);
        let item = fx.dir(".gradle/caches/x");
        let mount = item.join("mounted");
        fs::create_dir(&mount).unwrap();
        let image = fx.outside.join("volume.dmg");
        let hdiutil = |args: &[&str]| {
            let status = Command::new("hdiutil").args(args).status().unwrap();
            assert!(status.success(), "hdiutil {args:?} failed");
        };
        let (image_arg, mount_arg) = (image.to_str().unwrap(), mount.to_str().unwrap());
        hdiutil(&[
            "create",
            "-size",
            "4m",
            "-fs",
            "APFS",
            "-volname",
            "lfmclean-test",
            "-quiet",
            image_arg,
        ]);
        hdiutil(&[
            "attach",
            image_arg,
            "-mountpoint",
            mount_arg,
            "-nobrowse",
            "-quiet",
        ]);
        fs::write(mount.join("other-volume.txt"), "keep me").unwrap();
        let log = open_log(&fx);

        let result = remove(&fx, &log, Mode::Permanent, &root, &item);

        let kept = mount.join("other-volume.txt").exists();
        hdiutil(&["detach", mount_arg, "-quiet"]);
        assert!(matches!(result, Err(RemoveError::Io { .. })));
        assert!(kept, "a file on the other volume was removed");
    }
}
