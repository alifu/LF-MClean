//! Where the tool looks and who it runs as. Tests build an `Env` around a temporary folder.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use crate::catalog::{Base, Source};

const ROOT_UID: u32 = 0;
const LOG_DIR: &str = "Library/Logs/lfmclean";
const CRASH_LOG: &str = "crash.log";
/// Asked for the user's private cache folder; an absolute path, so `PATH` cannot swap it.
const GETCONF: &str = "/usr/bin/getconf";
const USER_CACHE_VARIABLE: &str = "DARWIN_USER_CACHE_DIR";
const USER_TEMP_VARIABLE: &str = "DARWIN_USER_TEMP_DIR";
/// Name the private cache and temporary folders instead of `getconf`; checked the same way. For
/// tests that run the program with another `HOME`, which must not see the real private folders.
pub const USER_CACHE_OVERRIDE: &str = "LFMCLEAN_USER_CACHE_DIR";
/// See `USER_CACHE_OVERRIDE`.
pub const USER_TEMP_OVERRIDE: &str = "LFMCLEAN_USER_TEMP_DIR";
/// Where macOS keeps each user's private folders. A private cache folder elsewhere is refused.
const USER_FOLDERS: &str = "/private/var/folders";

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
    /// The user's private cache folder, resolved; `None` when it is unknown or refused.
    user_cache: Option<PathBuf>,
    /// The user's private temporary folder, resolved; `None` when it is unknown or refused.
    user_temp: Option<PathBuf>,
}

impl Env {
    /// The real environment: `$HOME` and the effective user.
    ///
    /// # Errors
    /// See `Env::new`; also `NoHome` when `HOME` is not set.
    pub fn from_system() -> Result<Self, EnvError> {
        let home = std::env::var_os("HOME").ok_or(EnvError::NoHome)?;
        let env = Self::new(home, current_uid())?;
        let private = |variable, override_variable| {
            system_user_dir(variable, override_variable)
                .and_then(|dir| checked_user_dir(&dir, env.uid))
        };
        let user_cache = private(USER_CACHE_VARIABLE, USER_CACHE_OVERRIDE);
        let user_temp = private(USER_TEMP_VARIABLE, USER_TEMP_OVERRIDE);
        Ok(Self {
            user_cache,
            user_temp,
            ..env
        })
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
        Ok(Self {
            home,
            uid,
            user_cache: None,
            user_temp: None,
        })
    }

    /// The same environment with `dir` as the private cache folder, after checking it is a
    /// folder of this user inside `/private/var/folders`.
    #[must_use]
    pub fn with_user_cache(self, dir: &Path) -> Self {
        let user_cache = checked_user_dir(dir, self.uid);
        Self { user_cache, ..self }
    }

    /// The same environment with `dir` as the private temporary folder, checked like
    /// `with_user_cache`.
    #[must_use]
    pub fn with_user_temp(self, dir: &Path) -> Self {
        let user_temp = checked_user_dir(dir, self.uid);
        Self { user_temp, ..self }
    }

    /// Skips every check, so tests can pretend to be another user.
    #[cfg(test)]
    pub(crate) fn unchecked(home: impl Into<PathBuf>, uid: u32) -> Self {
        Self {
            home: home.into(),
            uid,
            user_cache: None,
            user_temp: None,
        }
    }

    /// Uses `dir` as the private cache folder without checking it, so tests can use a
    /// temporary folder.
    #[cfg(test)]
    pub(crate) fn with_user_cache_unchecked(self, dir: &Path) -> Self {
        Self {
            user_cache: dir.canonicalize().ok(),
            ..self
        }
    }

    /// Uses `dir` as the private temporary folder without checking it.
    #[cfg(test)]
    pub(crate) fn with_user_temp_unchecked(self, dir: &Path) -> Self {
        Self {
            user_temp: dir.canonicalize().ok(),
            ..self
        }
    }

    /// The home folder every cache path is relative to.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The user's private cache folder, resolved, when it is known.
    #[must_use]
    pub fn user_cache(&self) -> Option<&Path> {
        self.user_cache.as_deref()
    }

    /// The user's private temporary folder, resolved, when it is known.
    #[must_use]
    pub fn user_temp(&self) -> Option<&Path> {
        self.user_temp.as_deref()
    }

    /// The folder a source's `root` names; `None` when its base folder is unknown.
    #[must_use]
    pub fn source_root(&self, source: &Source) -> Option<PathBuf> {
        let base = match source.base {
            Base::Home => self.home(),
            Base::UserCache => self.user_cache()?,
            Base::UserTemp => self.user_temp()?,
        };
        Some(base.join(source.root))
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

/// The private folder `override_variable` names, or else the one `getconf variable` gives;
/// `None` when neither says.
fn system_user_dir(variable: &str, override_variable: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(override_variable) {
        return Some(PathBuf::from(dir));
    }
    let output = Command::new(GETCONF).arg(variable).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let dir = text.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// A private folder, resolved, when it is an absolute folder of `uid` inside
/// `/private/var/folders`; `None` otherwise, and then nothing in it is offered.
fn checked_user_dir(dir: &Path, uid: u32) -> Option<PathBuf> {
    if !dir.is_absolute() {
        return None;
    }
    let resolved = dir.canonicalize().ok()?;
    let inside_user_folders =
        resolved != Path::new(USER_FOLDERS) && resolved.starts_with(USER_FOLDERS);
    let meta = fs::metadata(&resolved).ok()?;
    (inside_user_folders && meta.is_dir() && meta.uid() == uid).then_some(resolved)
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
    fn the_private_cache_folder_must_be_inside_the_user_folders() {
        let tmp = TempDir::new().unwrap();
        let env = Env::new(tmp.path(), current_uid()).unwrap();

        for dir in [
            Path::new("/"),
            Path::new("relative"),
            Path::new(USER_FOLDERS),
        ] {
            assert_eq!(checked_user_dir(dir, current_uid()), None, "{dir:?}");
        }
        assert_eq!(
            env.clone().with_user_cache(Path::new("/tmp")).user_cache(),
            None
        );
    }

    #[test]
    fn the_private_cache_folder_must_belong_to_the_user() {
        let Some(dir) = system_user_dir(USER_CACHE_VARIABLE, "LFMCLEAN_TEST_UNSET") else {
            return;
        };

        let own = checked_user_dir(&dir, current_uid());

        assert!(own.as_ref().is_some_and(|d| d.starts_with(USER_FOLDERS)));
        assert_eq!(checked_user_dir(&dir, current_uid() + 1), None);
    }

    #[test]
    fn a_source_of_the_private_cache_folder_needs_that_folder() {
        let tmp = TempDir::new().unwrap();
        let env = Env::new(tmp.path(), current_uid()).unwrap();
        let source = Source::user_cache("com.apple.dt.Instruments", crate::catalog::Filter::Any);
        let home_source = Source::children(".gradle/caches", crate::catalog::Filter::Any);

        assert_eq!(env.source_root(&source), None);
        assert_eq!(
            env.source_root(&home_source),
            Some(tmp.path().join(".gradle/caches"))
        );
        let with = env.with_user_cache_unchecked(tmp.path());
        let resolved = tmp.path().canonicalize().unwrap();
        assert_eq!(
            with.source_root(&source),
            Some(resolved.join("com.apple.dt.Instruments"))
        );
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
