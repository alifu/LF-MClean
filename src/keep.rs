//! The keep list: targets, packages and paths that are never removed.
//!
//! It lives in `~/.config/lfmclean/keep.toml`. A missing file is an empty list; a file that exists
//! but cannot be read or understood is an error, so nothing is removed until it is fixed: running
//! without it would remove the very things it protects.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rustix::fs::OFlags;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use crate::catalog::{self, Source, Target};
use crate::env::Env;
use crate::packages::{self, Manager, PackageId};
use crate::scan::Item;

const KEEP_FILE: &str = ".config/lfmclean/keep.toml";
/// A hand-written keep list is a few lines; anything this big is not one.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const HOME_PREFIX: &str = "~/";
/// Only the user reads and writes the keep list.
const FILE_MODE: u32 = 0o600;

/// Why the keep list cannot be used. Every variant means: remove nothing.
#[derive(Debug, Error)]
pub enum KeepError {
    /// The file exists but could not be read.
    #[error(
        "the keep list {path} could not be read ({source}); nothing is removed until it can be read"
    )]
    Unreadable {
        /// The keep file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// The file was read but is not a valid keep list.
    #[error("the keep list {path} is not valid: {reason}; nothing is removed until it is fixed")]
    Invalid {
        /// The keep file.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The file could not be written.
    #[error("the keep list {path} could not be saved: {source}")]
    NotSaved {
        /// The keep file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

/// One kept package, written `pods:Firebase` (every version) or `pods:Firebase@11.0.0`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeptPackage {
    manager: Manager,
    name: String,
    version: Option<String>,
}

impl KeptPackage {
    /// Every version of a package.
    ///
    /// # Errors
    /// When the name could not be written back to the keep list and read again.
    pub fn all_versions(manager: Manager, name: &str) -> Result<Self, String> {
        Self::checked(manager, name, None)
    }

    /// One version of a package.
    ///
    /// # Errors
    /// When the name or version could not be written back to the keep list and read again.
    pub fn version(manager: Manager, name: &str, version: &str) -> Result<Self, String> {
        Self::checked(manager, name, Some(version))
    }

    fn checked(manager: Manager, name: &str, version: Option<&str>) -> Result<Self, String> {
        let usable = |text: &str| {
            !text.is_empty()
                && !text
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '@' || c == ':')
        };
        if !usable(name) {
            return Err(format!("{name:?} is not a usable package name"));
        }
        if version.is_some_and(|version| !usable(version)) {
            return Err(format!("{version:?} is not a usable version of {name}"));
        }
        Ok(Self {
            manager,
            name: name.to_string(),
            version: version.map(str::to_string),
        })
    }

    /// Names match without regard to case. A cache copy whose version is unknown (a repository
    /// clone, an external pod) may hold the kept version, so any entry for the package covers it.
    fn covers(&self, id: &PackageId) -> bool {
        self.manager == id.manager
            && self.name.eq_ignore_ascii_case(&id.name)
            && match (&self.version, &id.version) {
                (None, _) | (Some(_), None) => true,
                (Some(kept), Some(version)) => kept.eq_ignore_ascii_case(version),
            }
    }

    fn same_entry(&self, other: &Self) -> bool {
        self.manager == other.manager
            && self.name.eq_ignore_ascii_case(&other.name)
            && match (&self.version, &other.version) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            }
    }
}

impl fmt::Display for KeptPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.manager.key(), self.name)?;
        match &self.version {
            Some(version) => write!(f, "@{version}"),
            None => Ok(()),
        }
    }
}

impl FromStr for KeptPackage {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (manager, rest) = text
            .split_once(':')
            .ok_or_else(|| format!("package {text:?} has no `pods:` or `spm:` in front"))?;
        let manager = Manager::from_key(manager)
            .ok_or_else(|| format!("package {text:?}: {manager:?} is not `pods` or `spm`"))?;
        let (name, version) = match rest.split_once('@') {
            Some((name, version)) => (name, Some(version)),
            None => (rest, None),
        };
        Self::checked(manager, name, version)
            .map_err(|reason| format!("package {text:?}: {reason}"))
    }
}

/// The file as written by hand or by `save`.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct KeepFile {
    #[serde(default)]
    targets: Vec<String>,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

/// What is never removed, for one home folder. Changes return a new list; nothing is saved
/// until `save`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepList {
    /// The home folder, resolved, so target folders match the paths the guard hands out.
    home: PathBuf,
    targets: BTreeSet<&'static str>,
    packages: BTreeSet<KeptPackage>,
    paths: BTreeSet<PathBuf>,
}

impl KeepList {
    /// A list that keeps nothing.
    #[must_use]
    pub fn empty(env: &Env) -> Self {
        Self {
            home: resolved(env.home()),
            targets: BTreeSet::new(),
            packages: BTreeSet::new(),
            paths: BTreeSet::new(),
        }
    }

    /// Where the keep list of `env` lives.
    #[must_use]
    pub fn file(env: &Env) -> PathBuf {
        env.home().join(KEEP_FILE)
    }

    /// Reads the keep list. Only a file that is truly not there is an empty list.
    ///
    /// # Errors
    /// `Unreadable` or `Invalid` when the file exists, or a symlink to it or to its folder points
    /// nowhere, or it cannot be used; the caller must then remove nothing.
    pub fn load(env: &Env) -> Result<Self, KeepError> {
        let path = Self::file(env);
        let text = match read_limited(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return match dangling_link(&path) {
                    None => Ok(Self::empty(env)),
                    Some(link) => Err(KeepError::Unreadable {
                        source: io::Error::new(
                            io::ErrorKind::NotFound,
                            format!(
                                "{} is a symlink to something that is not there",
                                link.display()
                            ),
                        ),
                        path,
                    }),
                };
            }
            Err(source) => return Err(KeepError::Unreadable { path, source }),
        };
        Self::parse(&text, env).map_err(|reason| KeepError::Invalid { path, reason })
    }

    fn parse(text: &str, env: &Env) -> Result<Self, String> {
        let file: KeepFile = toml::from_str(text).map_err(|err| err.to_string())?;
        let mut list = Self::empty(env);
        for id in &file.targets {
            list = list.with_target(id)?;
        }
        for package in &file.packages {
            list = list.with_package(package.parse()?);
        }
        for path in &file.paths {
            list = list.with_path(path)?;
        }
        Ok(list)
    }

    /// Writes the list, replacing the file only once the new one is complete. When the keep file
    /// is a symlink, the file it points to is replaced and the link stays.
    ///
    /// # Errors
    /// `NotSaved` when the folder or file cannot be written, or a kept path is not UTF-8.
    pub fn save(&self, env: &Env) -> Result<(), KeepError> {
        let path = Self::file(env);
        self.to_toml()
            .and_then(|text| write_whole(&path, &text))
            .map_err(|source| KeepError::NotSaved { path, source })
    }

    fn to_toml(&self) -> io::Result<String> {
        let paths = self
            .paths
            .iter()
            .map(|path| self.written_path(path))
            .collect::<io::Result<_>>()?;
        let file = KeepFile {
            targets: self.targets.iter().map(ToString::to_string).collect(),
            packages: self.packages.iter().map(ToString::to_string).collect(),
            paths,
        };
        toml::to_string(&file).map_err(io::Error::other)
    }

    fn written_path(&self, path: &Path) -> io::Result<String> {
        let text = match path.strip_prefix(&self.home) {
            Ok(rest) => rest.to_str().map(|rest| format!("{HOME_PREFIX}{rest}")),
            Err(_) => path.to_str().map(str::to_string),
        };
        text.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the kept path {} is not UTF-8", path.display()),
            )
        })
    }

    /// True when nothing is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty() && self.packages.is_empty() && self.paths.is_empty()
    }

    /// The kept target ids.
    pub fn targets(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.targets.iter().copied()
    }

    /// The kept packages.
    pub fn packages(&self) -> impl Iterator<Item = &KeptPackage> {
        self.packages.iter()
    }

    /// The kept paths.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.paths.iter().map(PathBuf::as_path)
    }

    /// True when the item, or anything it would take with it, is kept: its target, its package,
    /// a kept path in or around it, or the items of a kept target that share its folders.
    #[must_use]
    pub fn keeps(&self, item: &Item) -> bool {
        let folders = item.removal.folders();
        self.targets.contains(item.target)
            || packages::identify(item).is_some_and(|id| self.keeps_package(&id))
            || folders.iter().any(|folder| self.keeps_path(folder))
            || folders.iter().any(|folder| self.holds_kept_target(folder))
    }

    /// True when this package, or every version of it, is kept.
    #[must_use]
    pub fn keeps_package(&self, id: &PackageId) -> bool {
        self.packages.iter().any(|kept| kept.covers(id))
    }

    /// True when removing `path` would remove something kept: the path itself, a folder it is in,
    /// or a kept path inside it.
    #[must_use]
    pub fn keeps_path(&self, path: &Path) -> bool {
        let path = folded(path);
        self.paths.iter().any(|kept| {
            let kept = folded(kept);
            path.starts_with(&kept) || kept.starts_with(&path)
        })
    }

    /// True when removing `path` would remove an item of a kept target, wherever that target's
    /// folders overlap another's (an Xcode project's package checkouts inside its `DerivedData`).
    fn holds_kept_target(&self, path: &Path) -> bool {
        self.targets
            .iter()
            .filter_map(|id| catalog::find(id))
            .flat_map(|target: &Target| target.sources)
            .any(|source| self.source_overlaps(source, path))
    }

    fn source_overlaps(&self, source: &Source, path: &Path) -> bool {
        let base = folded(&self.home.join(source.root).join(source.sub));
        let folded_path = folded(path);
        if base.starts_with(&folded_path) {
            return true;
        }
        let Ok(rest) = folded_path.strip_prefix(&base) else {
            return false;
        };
        let parts: Vec<_> = rest.components().collect();
        match parts.get(source.depth - 1) {
            // At or below the item level: it is, or is inside, an item if the source offers it.
            Some(entry) => matches_folded(source, &entry.as_os_str().to_string_lossy()),
            // Above it: it holds items only if one is there.
            None => holds_item(path, source, source.depth - parts.len()),
        }
    }
    /// The list with this target kept too.
    ///
    /// # Errors
    /// When lfmclean knows no target with this id.
    pub fn with_target(&self, id: &str) -> Result<Self, String> {
        let target =
            catalog::find(id).ok_or_else(|| format!("{id:?} is not a target lfmclean knows"))?;
        if id.starts_with("purge.") {
            return Err(format!(
                "{id:?} is a project folder found by purge; keep the project folder with a path instead"
            ));
        }
        let mut next = self.clone();
        next.targets.insert(target.id);
        Ok(next)
    }

    /// The list without this target.
    #[must_use]
    pub fn without_target(&self, id: &str) -> Self {
        let mut next = self.clone();
        next.targets.remove(id);
        next
    }

    /// The list with this package kept too.
    #[must_use]
    pub fn with_package(&self, package: KeptPackage) -> Self {
        let mut next = self.without_package(&package);
        next.packages.insert(package);
        next
    }

    /// The list without this exact entry (keeping every version and keeping one are separate).
    #[must_use]
    pub fn without_package(&self, package: &KeptPackage) -> Self {
        let mut next = self.clone();
        next.packages.retain(|kept| !kept.same_entry(package));
        next
    }

    /// The list with this path kept too, written `~/x` or `/x` as in the file.
    ///
    /// # Errors
    /// When the path is relative, contains `.` or `..`, or is `/`.
    pub fn with_path(&self, text: &str) -> Result<Self, String> {
        let path = kept_path(text, &self.home)?;
        let mut next = self.without_path(&path);
        next.paths.insert(path);
        Ok(next)
    }

    /// The list without this path.
    #[must_use]
    pub fn without_path(&self, path: &Path) -> Self {
        let mut next = self.clone();
        let path = folded(path);
        next.paths.retain(|kept| folded(kept) != path);
        next
    }
}

/// True when an entry the source offers is `levels` below `folder`. A folder that cannot be read
/// might hold one, so it counts as holding one.
fn holds_item(folder: &Path, source: &Source, levels: usize) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else { return true };
        let name = entry.file_name().to_string_lossy().into_owned();
        if levels <= 1 {
            return matches_folded(source, &name);
        }
        let is_real_folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        is_real_folder && holds_item(&entry.path(), source, levels - 1)
    })
}

/// The source's filter on a folded name: the filters compare exactly and APFS does not.
fn matches_folded(source: &Source, name: &str) -> bool {
    let name = fold_text(name);
    match source.filter {
        catalog::Filter::Named(names) => names.iter().any(|n| fold_text(n) == name),
        catalog::Filter::Prefix(prefix) => name.starts_with(&fold_text(prefix)),
        catalog::Filter::Suffix(suffix) => name.ends_with(&fold_text(suffix)),
        catalog::Filter::Any | catalog::Filter::DirsOnly => true,
    }
}

/// Reads the file without blocking on a pipe, refusing anything larger than a keep list can be.
fn read_limited(path: &Path) -> io::Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(OFlags::NONBLOCK.bits().cast_signed())
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("larger than {MAX_FILE_BYTES} bytes"),
        ));
    }
    Ok(text)
}

/// The symlink on the way to `path` that points nowhere, if that is why it was not found.
fn dangling_link(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|entry| {
        let is_link = fs::symlink_metadata(entry).ok()?.file_type().is_symlink();
        (is_link && fs::metadata(entry).is_err()).then(|| entry.to_path_buf())
    })
}

/// `~/x` or an absolute path, with `.` and `..` refused so the path means what it says.
fn kept_path(text: &str, home: &Path) -> Result<PathBuf, String> {
    // `components()` quietly drops `.`, so the text itself is checked.
    if text.split('/').any(|part| part == "." || part == "..") {
        return Err(format!("path {text:?} must not contain `.` or `..`"));
    }
    let path = match text.strip_prefix(HOME_PREFIX) {
        Some(rest) => home.join(rest),
        None => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err(format!("path {text:?} must start with `/` or `~/`"));
    }
    if path.file_name().is_none() {
        return Err(format!(
            "path {text:?} cannot be kept: it is the whole disk"
        ));
    }
    Ok(resolved(&path))
}

/// The path with every folder above it resolved as far as they exist, so it matches the paths
/// the guard hands out. The last part is not resolved: a kept symlink protects the link itself.
fn resolved(path: &Path) -> PathBuf {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let mut missing = Vec::new();
    let mut existing = parent;
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            out.extend(missing.iter().rev());
            out.push(name);
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(up), Some(part)) => {
                missing.push(part);
                existing = up;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// APFS ignores case and Unicode normalization, so paths are compared in one form.
/// The path as APFS compares it: without regard to case or Unicode normalization.
pub(crate) fn folded(path: &Path) -> PathBuf {
    PathBuf::from(fold_text(&path.to_string_lossy()))
}

fn fold_text(text: &str) -> String {
    text.nfc().collect::<String>().to_lowercase()
}

/// Writes a new file next to the keep file (never through a symlink, never over an existing
/// file), flushes it, renames it over the keep file and flushes the folder. A keep file that is a
/// symlink has the file it points to replaced.
fn write_whole(path: &Path, text: &str) -> io::Result<()> {
    let is_link = fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    let path = if is_link {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let folder = path
        .parent()
        .ok_or_else(|| io::Error::other("the keep file has no folder"))?;
    fs::create_dir_all(folder)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos());
    let temporary = folder.join(format!(".keep.toml.{}.{nanos}.tmp", process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .custom_flags(OFlags::NOFOLLOW.bits().cast_signed())
        .open(&temporary)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, &path));
    if written.is_err() {
        // The new file never replaced anything; it is only litter now.
        let _ = fs::remove_file(&temporary);
    }
    written?;
    File::open(folder)?.sync_all()
}

#[cfg(test)]
mod tests;
