//! Where the tool looks and who it runs as. Tests build an `Env` around a temporary folder.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use thiserror::Error;

const ROOT_UID: u32 = 0;
const LOG_DIR: &str = "Library/Logs/lfmclean";
const CRASH_LOG: &str = "crash.log";

/// Why the environment cannot be used.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnvError {
    /// Started with `sudo` or as root.
    #[error("lfmclean must not run as root; run it without sudo")]
    RunningAsRoot,
    /// `HOME` is not set.
    #[error("cannot find your home folder: HOME is not set")]
    NoHome,
    /// `HOME` is not an absolute folder owned by the user.
    #[error("{path} cannot be your home folder: {reason}")]
    BadHome {
        /// The refused home.
        path: PathBuf,
        /// Why it was refused.
        reason: &'static str,
    },
}

/// The home folder and user the tool works for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    home: PathBuf,
    uid: u32,
}

impl Env {
    /// The real environment: `$HOME` and the effective user.
    ///
    /// # Errors
    /// See `Env::new`; also `NoHome` when `HOME` is not set.
    pub fn from_system() -> Result<Self, EnvError> {
        let home = std::env::var_os("HOME").ok_or(EnvError::NoHome)?;
        Self::new(home, current_uid())
    }

    /// An environment for `home` and `uid`, after checking both.
    ///
    /// # Errors
    /// `RunningAsRoot` for uid 0, and `BadHome` unless `home` is an absolute folder owned by
    /// `uid` (so `HOME=/` or `HOME=/tmp`, both owned by root, are refused).
    pub fn new(home: impl Into<PathBuf>, uid: u32) -> Result<Self, EnvError> {
        ensure_not_root(uid)?;
        let home = home.into();
        validate_home(&home, uid)?;
        Ok(Self { home, uid })
    }

    /// Skips every check, so tests can pretend to be another user.
    #[cfg(test)]
    pub(crate) fn unchecked(home: impl Into<PathBuf>, uid: u32) -> Self {
        Self {
            home: home.into(),
            uid,
        }
    }

    /// The home folder every cache path is relative to.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The user whose files may be removed.
    #[must_use]
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Where the removal log is kept.
    #[must_use]
    pub fn log_dir(&self) -> PathBuf {
        self.home.join(LOG_DIR)
    }

    /// Where a background thread's panic is written.
    #[must_use]
    pub fn crash_log(&self) -> PathBuf {
        self.log_dir().join(CRASH_LOG)
    }
}

/// The effective user id of this process.
#[must_use]
pub fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Refuses the root user.
///
/// # Errors
/// `RunningAsRoot` for uid 0.
pub fn ensure_not_root(uid: u32) -> Result<(), EnvError> {
    if uid == ROOT_UID {
        return Err(EnvError::RunningAsRoot);
    }
    Ok(())
}

fn validate_home(home: &Path, uid: u32) -> Result<(), EnvError> {
    let bad = |reason| EnvError::BadHome {
        path: home.to_path_buf(),
        reason,
    };
    if !home.is_absolute() {
        return Err(bad("not an absolute path"));
    }
    let meta = fs::metadata(home).map_err(|_| bad("it cannot be read"))?;
    if !meta.is_dir() {
        return Err(bad("not a folder"));
    }
    if meta.uid() != uid {
        return Err(bad("it belongs to another user"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn refuses_root() {
        assert_eq!(ensure_not_root(0), Err(EnvError::RunningAsRoot));
        let tmp = TempDir::new().unwrap();
        assert_eq!(Env::new(tmp.path(), 0), Err(EnvError::RunningAsRoot));
    }

    #[test]
    fn accepts_a_folder_owned_by_the_user() {
        let tmp = TempDir::new().unwrap();

        let env = Env::new(tmp.path(), current_uid()).unwrap();

        assert_eq!(env.log_dir(), tmp.path().join("Library/Logs/lfmclean"));
    }

    #[test]
    fn refuses_a_bad_home() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("file");
        fs::write(&file, "").unwrap();
        for home in [
            PathBuf::from("/"),
            PathBuf::from("relative"),
            tmp.path().join("missing"),
            file,
        ] {
            assert!(
                matches!(
                    Env::new(&home, current_uid()),
                    Err(EnvError::BadHome { .. })
                ),
                "{} must be refused",
                home.display()
            );
        }
    }

    #[test]
    fn refuses_a_home_owned_by_someone_else() {
        let tmp = TempDir::new().unwrap();

        assert!(matches!(
            Env::new(tmp.path(), current_uid() + 1),
            Err(EnvError::BadHome { .. })
        ));
    }
}
