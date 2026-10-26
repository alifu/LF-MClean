//! Which projects use which packages, from the `Podfile.lock` and `Package.resolved` files found
//! in the project folders. Only read; a file that cannot be read is reported, not guessed at.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use jwalk::WalkDir;
use rustix::fs::OFlags;

use super::{Manager, pods, spm};
use crate::env::Env;

/// Where projects usually are, relative to home.
pub const DEFAULT_PROJECT_FOLDERS: &[&str] = &["Documents", "Developer", "Projects"];
/// Deep enough for `<group>/<project>/<App>.xcodeproj/project.xcworkspace/xcshareddata/swiftpm`.
const MAX_DEPTH: usize = 8;
/// Lock files are text of a few hundred kilobytes at most.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const PODFILE_LOCK: &str = "Podfile.lock";
const PACKAGE_RESOLVED: &str = "Package.resolved";
/// Folders full of files that are not projects, or of copies of other projects' lock files.
const NOT_ENTERED: &[&str] = &["node_modules", "build", "Pods", "DerivedData", "Carthage"];
const BUNDLE_SUFFIXES: &[&str] = &[".xcodeproj", ".xcworkspace"];

/// One project's use of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    /// The project name.
    pub project: String,
    /// The project folder, which tells apart projects with the same name.
    pub folder: PathBuf,
    /// The version its lock file names, if it names one.
    pub version: Option<String>,
}

/// Package uses, by manager and lowercased name.
#[derive(Debug, Default)]
pub struct Usage {
    uses: HashMap<(Manager, String), Vec<Use>>,
    /// Folders and lock files that could not be read, with the reason.
    pub problems: Vec<String>,
}

impl Usage {
    /// True when every folder and lock file was read. When it is false, a package that looks
    /// unused may be used by a project that could not be read, so "not used" must not be acted on.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.problems.is_empty()
    }

    /// Every use of this package, any version.
    #[must_use]
    pub fn of(&self, manager: Manager, name: &str) -> &[Use] {
        self.uses
            .get(&(manager, name.to_lowercase()))
            .map_or(&[], Vec::as_slice)
    }

    fn add(
        &mut self,
        manager: Manager,
        name: &str,
        project: &(String, PathBuf),
        version: Option<String>,
    ) {
        let found = Use {
            project: project.0.clone(),
            folder: project.1.clone(),
            version,
        };
        let uses = self.uses.entry((manager, name.to_lowercase())).or_default();
        if !uses.contains(&found) {
            uses.push(found);
        }
    }

    /// Reads one lock file into the uses.
    fn read(&mut self, path: &Path) {
        let project = project_of(path);
        let added = read_limited(path)
            .map_err(|err| err.to_string())
            .and_then(|text| match path.file_name().and_then(OsStr::to_str) {
                Some(PODFILE_LOCK) => pods::parse_lock(&text).map(|pods| {
                    for (name, version) in pods {
                        self.add(Manager::Pods, &name, &project, Some(version));
                    }
                }),
                _ => spm::parse_resolved(&text).map(|pins| {
                    for pin in pins {
                        for name in &pin.names {
                            self.add(Manager::Spm, name, &project, pin.version.clone());
                        }
                    }
                }),
            });
        if let Err(reason) = added {
            self.problems
                .push(format!("{} could not be read: {reason}", path.display()));
        }
    }
}

/// The uses found in `DEFAULT_PROJECT_FOLDERS` of `env`'s home.
#[must_use]
pub fn find_usage(env: &Env) -> Usage {
    let folders: Vec<PathBuf> = DEFAULT_PROJECT_FOLDERS
        .iter()
        .map(|folder| env.home().join(folder))
        .collect();
    usage_in(&folders)
}

/// The uses found in lock files under `folders`, which are searched a few levels deep without
/// following symlinks, skipping hidden and build folders.
#[must_use]
pub fn usage_in(folders: &[PathBuf]) -> Usage {
    let mut usage = Usage::default();
    for folder in folders.iter().filter(|folder| folder.is_dir()) {
        for found in lock_files(folder) {
            match found {
                Ok(path) => usage.read(&path),
                Err(problem) => usage.problems.push(problem),
            }
        }
    }
    usage
}

/// The lock files under `folder`, and a problem for each folder that could not be read.
fn lock_files(folder: &Path) -> Vec<Result<PathBuf, String>> {
    let walk = WalkDir::new(folder)
        .max_depth(MAX_DEPTH)
        .skip_hidden(true)
        .follow_links(false)
        .sort(true)
        .process_read_dir(|_, _, (), children| {
            children.retain(|child| {
                child.as_ref().map_or(true, |entry| {
                    !(entry.file_type.is_dir()
                        && entry
                            .file_name
                            .to_str()
                            .is_some_and(|name| NOT_ENTERED.contains(&name)))
                })
            });
        });
    let mut found = Vec::new();
    for entry in walk {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                found.push(Err(format!("a folder could not be searched: {err}")));
                continue;
            }
        };
        if let Some(err) = entry.read_children.as_ref().and_then(|c| c.error()) {
            let path = entry.path();
            found.push(Err(format!(
                "{} could not be searched: {err}",
                path.display()
            )));
        }
        let is_lock_file = matches!(
            entry.file_name.to_str(),
            Some(PODFILE_LOCK | PACKAGE_RESOLVED)
        );
        if entry.file_type.is_file() && is_lock_file {
            found.push(Ok(entry.path()));
        }
    }
    found
}

/// The name and folder of the project a lock file belongs to: the outermost `.xcodeproj` or
/// `.xcworkspace` it is in (named without the suffix, in the folder holding it), else the folder
/// it is in.
fn project_of(lock_file: &Path) -> (String, PathBuf) {
    let bundle = lock_file.ancestors().skip(1).filter_map(|bundle| {
        let name = bundle.file_name()?.to_str()?;
        let stem = BUNDLE_SUFFIXES
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))?;
        Some((stem.to_string(), bundle.parent()?.to_path_buf()))
    });
    let folder = lock_file.parent().unwrap_or(lock_file);
    bundle.last().unwrap_or_else(|| {
        let name = folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        (name, folder.to_path_buf())
    })
}

/// Reads a lock file that was a regular file when found: a symlink swapped in since is refused,
/// and a pipe cannot make the read wait forever.
fn read_limited(path: &Path) -> io::Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits().cast_signed())
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
            "too large for a lock file",
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::testing::Fixture;

    const SNAPKIT_RESOLVED: &str = r#"{"pins": [{"identity": "snapkit",
        "location": "https://github.com/SnapKit/SnapKit.git",
        "state": {"version": "5.7.1"}}], "version": 2}"#;

    fn write(fx: &Fixture, rel: &str, text: &str) -> PathBuf {
        let path = fx.home().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_podfile_lock_names_its_folder_as_the_project() {
        let fx = Fixture::new();
        write(
            &fx,
            "Documents/Apps/MyCMPApp/iosApp/Podfile.lock",
            "PODS:\n  - Firebase (10.25.0)\n",
        );

        let usage = find_usage(&fx.env);

        assert_eq!(
            usage.of(Manager::Pods, "firebase"),
            [Use {
                project: "iosApp".to_string(),
                folder: fx.home().join("Documents/Apps/MyCMPApp/iosApp"),
                version: Some("10.25.0".to_string()),
            }]
        );
        assert_eq!(usage.problems, Vec::<String>::new());
    }

    #[test]
    fn a_resolved_file_inside_an_xcode_project_names_that_project() {
        let fx = Fixture::new();
        write(
            &fx,
            "Developer/CatWalk/CatWalk.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved",
            SNAPKIT_RESOLVED,
        );

        let usage = find_usage(&fx.env);

        let uses = usage.of(Manager::Spm, "SnapKit");
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].project, "CatWalk");
        assert_eq!(uses[0].version.as_deref(), Some("5.7.1"));
    }

    #[test]
    fn hidden_build_and_dependency_folders_are_not_searched() {
        let fx = Fixture::new();
        for skipped in [
            "Projects/App/.build/checkouts/Dep/Package.resolved",
            "Projects/App/node_modules/x/Package.resolved",
            "Projects/App/Pods/Local/Package.resolved",
            "Projects/App/build/Package.resolved",
        ] {
            write(&fx, skipped, SNAPKIT_RESOLVED);
        }

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Spm, "snapkit"), []);
    }

    #[test]
    fn folders_too_deep_or_outside_the_project_folders_are_not_searched() {
        let fx = Fixture::new();
        write(&fx, "Desktop/App/Package.resolved", SNAPKIT_RESOLVED);
        write(
            &fx,
            "Documents/1/2/3/4/5/6/7/8/Package.resolved",
            SNAPKIT_RESOLVED,
        );

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Spm, "snapkit"), []);
    }

    #[test]
    fn a_symlinked_folder_is_not_followed() {
        let fx = Fixture::new();
        fs::write(fx.outside.join(PACKAGE_RESOLVED), SNAPKIT_RESOLVED).unwrap();
        fs::create_dir_all(fx.home().join("Documents")).unwrap();
        std::os::unix::fs::symlink(&fx.outside, fx.home().join("Documents/link")).unwrap();

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Spm, "snapkit"), []);
    }

    #[test]
    fn a_broken_lock_file_is_reported_and_the_rest_still_count() {
        let fx = Fixture::new();
        let broken = write(&fx, "Documents/A/Package.resolved", "{ not json");
        write(&fx, "Documents/B/Package.resolved", SNAPKIT_RESOLVED);

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Spm, "snapkit").len(), 1);
        assert_eq!(usage.problems.len(), 1);
        assert!(usage.problems[0].contains(&broken.display().to_string()));
    }

    #[test]
    fn the_same_project_found_twice_counts_once_and_a_namesake_counts_apart() {
        let fx = Fixture::new();
        write(&fx, "Documents/App/Package.resolved", SNAPKIT_RESOLVED);
        write(
            &fx,
            "Documents/App/App.xcworkspace/xcshareddata/swiftpm/Package.resolved",
            SNAPKIT_RESOLVED,
        );
        write(
            &fx,
            "Documents/Other/App/Package.resolved",
            SNAPKIT_RESOLVED,
        );

        let usage = find_usage(&fx.env);

        let uses = usage.of(Manager::Spm, "snapkit");
        assert_eq!(uses.len(), 2, "{uses:?}");
        assert!(uses.iter().all(|u| u.project == "App"));
        assert_ne!(uses[0].folder, uses[1].folder);
    }

    #[test]
    fn no_project_folders_means_no_uses() {
        let fx = Fixture::new();

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Pods, "anything"), []);
        assert_eq!(usage.problems, Vec::<String>::new());
    }

    #[test]
    fn a_folder_that_cannot_be_searched_is_a_problem_and_usage_is_incomplete() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        write(&fx, "Documents/Open/Package.resolved", SNAPKIT_RESOLVED);
        let locked = fx.home().join("Documents/Locked");
        fs::create_dir_all(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let usage = find_usage(&fx.env);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(usage.of(Manager::Spm, "snapkit").len(), 1);
        assert_eq!(usage.problems.len(), 1, "{:?}", usage.problems);
        assert!(usage.problems[0].contains("Locked"), "{:?}", usage.problems);
        assert!(!usage.is_complete());
    }

    #[test]
    fn a_symlinked_lock_file_is_not_read() {
        let fx = Fixture::new();
        fs::write(fx.outside.join(PACKAGE_RESOLVED), SNAPKIT_RESOLVED).unwrap();
        fs::create_dir_all(fx.home().join("Documents/App")).unwrap();
        std::os::unix::fs::symlink(
            fx.outside.join(PACKAGE_RESOLVED),
            fx.home().join("Documents/App").join(PACKAGE_RESOLVED),
        )
        .unwrap();

        let usage = find_usage(&fx.env);

        assert_eq!(usage.of(Manager::Spm, "snapkit"), []);
        assert!(usage.is_complete());
    }

    #[test]
    fn reading_refuses_a_symlink_and_a_pipe() {
        let fx = Fixture::new();
        let link = fx.home().join("link.resolved");
        std::os::unix::fs::symlink(fx.outside.join("precious.txt"), &link).unwrap();
        let pipe = fx.home().join("pipe.resolved");
        let made = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap();
        assert!(made.success());

        assert!(read_limited(&link).is_err());
        assert!(read_limited(&pipe).is_err(), "a pipe must not be waited on");
    }
}
