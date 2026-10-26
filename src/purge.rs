//! Finding the build folders of the user's Gradle and Kotlin Multiplatform projects. Only looks;
//! every folder found is checked by `guard::check_project` before it can be offered.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use jwalk::WalkDir;

use crate::env::Env;
use crate::guard::{self, SafePath};
use crate::packages::DEFAULT_PROJECT_FOLDERS;
use crate::scan::size::disk_usage;
use crate::scan::{Item, Removal, modified};

/// Deep enough for `<group>/<client>/<project>/<module>`, counted from the folder searched.
const MAX_DEPTH: usize = 8;
const SETTINGS_FILES: &[&str] = &["settings.gradle", "settings.gradle.kts"];
const MODULE_FILES: &[&str] = &["build.gradle", "build.gradle.kts"];
/// Folders that hold other things than projects, or copies of them.
const NOT_ENTERED: &[&str] = &[
    "node_modules",
    "build",
    "Pods",
    "DerivedData",
    "Carthage",
    "Library",
];
const IOS_APP: &str = "iosApp";
/// The search stops after this many entries, so a huge tree or a stalled mount cannot hang it.
const MAX_ENTRIES: usize = 400_000;
/// Problems listed; more are counted.
const MAX_PROBLEMS: usize = 100;
/// What Gradle and the Kotlin and Android plugins put at the top of a `build` folder.
const GENERATED: &[&str] = &[
    "classes",
    "intermediates",
    "outputs",
    "tmp",
    "generated",
    "kotlin",
    "libs",
    "reports",
    "bootstrap",
    "distributions",
    "test-results",
    "jacoco",
    "publications",
    "scripts",
    "install",
    "compose",
    "ksp",
    "kapt",
    "xcode-frameworks",
    "cocoapods",
    "processedResources",
    "resources",
    "docs",
    "dokka",
    "jni",
    "bin",
    "poms",
    "kover",
    "stripped",
    "native",
    "kotlin-js-store",
    "js",
    "wasm",
    "compileSync",
    "cinterop",
    "ivy",
    "meta-inf",
];
/// How a `.gitignore` line can name a `build` folder.
const IGNORES_BUILD: &[&str] = &["build", "**/build", "*/build", "build/*", "build/**"];
const MAX_GITIGNORE_BYTES: u64 = 1024 * 1024;

/// One project and the folders `purge` can remove from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// The name of its folder.
    pub name: String,
    /// Where it is.
    pub folder: PathBuf,
    /// When its folder or its Gradle files last changed; build output does not count.
    pub changed: Option<DateTime<FixedOffset>>,
    /// The folders that can go, largest first.
    pub items: Vec<Item>,
}

impl Project {
    /// Disk space of the folders.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|item| item.size_bytes).sum()
    }
}

/// The projects found, and what could not be looked at.
#[derive(Debug, Default)]
pub struct Found {
    /// Projects that have something to remove, by name.
    pub projects: Vec<Project>,
    /// Folders that could not be searched, and folders that were refused.
    pub problems: Vec<String>,
}

/// The folders searched when none are named: `~/Documents`, `~/Developer` and `~/Projects`.
#[must_use]
pub fn default_folders(env: &Env) -> Vec<PathBuf> {
    DEFAULT_PROJECT_FOLDERS
        .iter()
        .map(|folder| env.home().join(folder))
        // As on disk, so a folder that is a link does not have every project refused.
        .map(|folder| folder.canonicalize().unwrap_or(folder))
        .collect()
}

/// The folders named with `--paths`: `~/…` or absolute, existing, and inside the home folder.
///
/// # Errors
/// Why a path cannot be searched.
pub fn folders_from(env: &Env, paths: &[String]) -> Result<Vec<PathBuf>, String> {
    let home = env
        .home()
        .canonicalize()
        .map_err(|err| format!("the home folder cannot be read: {err}"))?;
    paths
        .iter()
        .map(|text| {
            let path = match text.strip_prefix("~/") {
                Some(rest) => env.home().join(rest),
                None => PathBuf::from(text),
            };
            if !path.is_absolute() {
                return Err(format!("{text:?} must start with ~/ or /"));
            }
            let resolved = path
                .canonicalize()
                .map_err(|err| format!("{text:?} cannot be searched: {err}"))?;
            if resolved == home || !resolved.starts_with(&home) {
                return Err(format!("{text:?} must be a folder inside your home folder"));
            }
            Ok(resolved)
        })
        .collect()
}

/// The projects under `folders`, which are searched a few levels deep without following symlinks,
/// skipping hidden folders and build output.
#[must_use]
pub fn find_projects(env: &Env, folders: &[PathBuf]) -> Found {
    let mut found = Found::default();
    let mut gradle_dirs = BTreeSet::new();
    let mut settings_dirs = BTreeSet::new();
    for folder in folders.iter().filter(|folder| folder.is_dir()) {
        walk(
            folder,
            &mut gradle_dirs,
            &mut settings_dirs,
            &mut found.problems,
        );
    }
    let mut by_root: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for dir in &gradle_dirs {
        // The outermost settings file owns the build, so a composite build (`build-logic`) is part
        // of its project, and keeping the project keeps it.
        let root = dir
            .ancestors()
            .filter(|ancestor| settings_dirs.contains(*ancestor))
            .last()
            .unwrap_or(dir);
        by_root
            .entry(root.to_path_buf())
            .or_default()
            .push(dir.clone());
    }
    for (root, modules) in by_root {
        let (project, problems) = project_at(env, &root, &modules);
        found.problems.extend(problems);
        found.projects.extend(project);
    }
    if found.problems.len() > MAX_PROBLEMS {
        let more = found.problems.len() - MAX_PROBLEMS;
        found.problems.truncate(MAX_PROBLEMS);
        found.problems.push(format!("and {more} more"));
    }
    found
        .projects
        .sort_by_key(|project| (project.name.to_lowercase(), project.folder.clone()));
    found
}

fn walk(
    folder: &Path,
    gradle_dirs: &mut BTreeSet<PathBuf>,
    settings_dirs: &mut BTreeSet<PathBuf>,
    problems: &mut Vec<String>,
) {
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
    for (seen, entry) in walk.into_iter().enumerate() {
        if seen >= MAX_ENTRIES {
            problems.push(format!(
                "{} was searched only in part: it has too many entries",
                folder.display()
            ));
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                problems.push(format!("a folder could not be searched: {err}"));
                continue;
            }
        };
        if let Some(err) = entry.read_children.as_ref().and_then(|c| c.error()) {
            problems.push(format!(
                "{} could not be searched: {err}",
                entry.path().display()
            ));
        }
        let Some(name) = entry
            .file_name
            .to_str()
            .filter(|_| entry.file_type.is_file())
        else {
            continue;
        };
        let Some(dir) = entry.path().parent().map(Path::to_path_buf) else {
            continue;
        };
        if SETTINGS_FILES.contains(&name) {
            settings_dirs.insert(dir.clone());
            gradle_dirs.insert(dir);
        } else if MODULE_FILES.contains(&name) {
            gradle_dirs.insert(dir);
        }
    }
}

/// The project whose folder is `root` and whose Gradle folders are `modules`, with its removable
/// folders; `None` when it has none.
fn project_at(env: &Env, root: &Path, modules: &[PathBuf]) -> (Option<Project>, Vec<String>) {
    // The guard names paths as they are on disk; the project folder must be named the same way.
    let root = &root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut candidates: Vec<(PathBuf, &'static str)> = modules
        .iter()
        .map(|module| (module.join("build"), "purge.build"))
        .collect();
    candidates.push((root.join(".gradle"), "purge.gradle"));
    candidates.push((root.join(".kotlin"), "purge.kotlin"));
    candidates.push((root.join(IOS_APP).join("build"), "purge.ios-build"));
    candidates.push((root.join(IOS_APP).join("Pods"), "purge.pods"));
    let mut items = Vec::new();
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for (path, target) in candidates {
        // `iosApp` can be a module too: its `build` is one folder, offered once.
        if !seen.insert(path.clone()) {
            continue;
        }
        // Absent folders are the normal case; present ones that are refused are worth saying.
        if fs::symlink_metadata(&path).is_err() {
            continue;
        }
        match guard::check_project(&path, root, env) {
            Ok(safe) => {
                let target = if target == "purge.build" && !looks_generated(&safe, root) {
                    "purge.build-unsure"
                } else {
                    target
                };
                items.push(item_for(root, target, safe));
            }
            Err(err) => problems.push(format!("{} is not offered: {err}", path.display())),
        }
    }
    if items.is_empty() {
        return (None, problems);
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    let project = Project {
        name: root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        folder: root.clone(),
        changed: changed(root),
        items,
    };
    (Some(project), problems)
}

/// True when a `build` folder holds only what Gradle makes, or git ignores it: then nothing in it
/// is the user's own work. Anything else is offered with more care.
fn looks_generated(safe: &SafePath, root: &Path) -> bool {
    let build = safe.path();
    let Ok(entries) = fs::read_dir(build) else {
        return false;
    };
    let only_generated = entries.flatten().all(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| GENERATED.contains(&name))
    });
    only_generated
        || [build.parent(), Some(root)]
            .into_iter()
            .flatten()
            .any(ignores_build)
}

/// True when the `.gitignore` in `folder` has a line naming `build`.
fn ignores_build(folder: &Path) -> bool {
    use std::io::Read;
    let file = folder.join(".gitignore");
    let Ok(meta) = fs::symlink_metadata(&file) else {
        return false;
    };
    if !meta.is_file() || meta.len() > MAX_GITIGNORE_BYTES {
        return false;
    }
    let Ok(mut text) = fs::File::open(&file).map(|f| f.take(MAX_GITIGNORE_BYTES)) else {
        return false;
    };
    let mut content = String::new();
    if text.read_to_string(&mut content).is_err() {
        return false;
    }
    content.lines().any(|line| {
        let line = line.trim().trim_start_matches('/').trim_end_matches('/');
        IGNORES_BUILD.contains(&line)
    })
}

fn item_for(root: &Path, target: &'static str, safe: SafePath) -> Item {
    let usage = disk_usage(safe.path());
    let name = safe
        .path()
        .strip_prefix(root)
        .unwrap_or_else(|_| safe.path())
        .display()
        .to_string();
    Item {
        target,
        name,
        size_bytes: usage.bytes,
        partial: usage.unreadable > 0,
        modified: modified(safe.path()),
        removal: Removal::Paths {
            main: safe,
            with: Vec::new(),
        },
    }
}

/// The newest change of the folder itself and of the Gradle files in it.
fn changed(root: &Path) -> Option<DateTime<FixedOffset>> {
    SETTINGS_FILES
        .iter()
        .chain(MODULE_FILES)
        .map(|name| root.join(name))
        .chain(std::iter::once(root.to_path_buf()))
        .filter_map(|path| modified(&path))
        .max()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::testing::Fixture;

    fn kmp(fx: &Fixture, rel: &str) {
        fx.file(&format!("{rel}/settings.gradle.kts"));
        fx.file(&format!("{rel}/build.gradle.kts"));
        fx.file(&format!("{rel}/composeApp/build.gradle.kts"));
        fx.dir(&format!("{rel}/composeApp/build/classes"));
        fx.dir(&format!("{rel}/build/outputs"));
        fx.dir(&format!("{rel}/.gradle"));
        fx.dir(&format!("{rel}/.kotlin"));
        fx.dir(&format!("{rel}/iosApp/iosApp.xcodeproj"));
        fx.dir(&format!("{rel}/iosApp/build/xcode-frameworks"));
        fx.file(&format!("{rel}/iosApp/Podfile"));
        fx.dir(&format!("{rel}/iosApp/Pods"));
    }

    fn names(project: &Project) -> Vec<&str> {
        let mut names: Vec<&str> = project.items.iter().map(|i| i.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn a_kmp_project_offers_each_build_folder() {
        let fx = Fixture::new();
        kmp(&fx, "Documents/App");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.problems, Vec::<String>::new());
        let [project] = &found.projects[..] else {
            panic!("{found:?}")
        };
        assert_eq!(project.name, "App");
        assert_eq!(
            names(project),
            [
                ".gradle",
                ".kotlin",
                "build",
                "composeApp/build",
                "iosApp/Pods",
                "iosApp/build"
            ]
        );
        let target = |name: &str| {
            project
                .items
                .iter()
                .find(|i| i.name == name)
                .unwrap()
                .target
        };
        assert_eq!(target("iosApp/Pods"), "purge.pods");
        assert_eq!(target("composeApp/build"), "purge.build");
        assert!(project.changed.is_some());
    }

    fn target_of(found: &Found, name: &str) -> &'static str {
        found.projects[0]
            .items
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .target
    }

    #[test]
    fn a_build_folder_with_other_files_is_offered_with_more_care() {
        let fx = Fixture::new();
        fx.file("Documents/App/settings.gradle.kts");
        fx.file("Documents/App/build/notes.md");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(target_of(&found, "build"), "purge.build-unsure");
    }

    #[test]
    fn an_ignored_or_empty_build_folder_is_ordinary() {
        let fx = Fixture::new();
        fx.file("Documents/Ignored/settings.gradle.kts");
        fx.file("Documents/Ignored/.gitignore");
        fs::write(
            fx.home().join("Documents/Ignored/.gitignore"),
            "*.log\n/build/\n",
        )
        .unwrap();
        fx.file("Documents/Ignored/build/notes.md");
        fx.file("Documents/Empty/settings.gradle.kts");
        fs::create_dir_all(fx.home().join("Documents/Empty/build")).unwrap();

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        let targets: Vec<_> = found.projects.iter().map(|p| p.items[0].target).collect();
        assert_eq!(targets, ["purge.build", "purge.build"]);
    }

    #[test]
    fn a_gitignore_that_is_a_link_proves_nothing() {
        let fx = Fixture::new();
        fx.file("Documents/App/settings.gradle.kts");
        fx.file("Documents/App/build/notes.md");
        fs::write(fx.outside.join("ignore"), "build\n").unwrap();
        symlink(
            fx.outside.join("ignore"),
            fx.home().join("Documents/App/.gitignore"),
        )
        .unwrap();

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(target_of(&found, "build"), "purge.build-unsure");
    }

    #[test]
    fn an_ios_app_that_is_also_a_module_offers_its_build_folder_once() {
        let fx = Fixture::new();
        fx.file("Documents/App/settings.gradle.kts");
        fx.file("Documents/App/iosApp/build.gradle.kts");
        fx.dir("Documents/App/iosApp/Foo.xcodeproj");
        fx.dir("Documents/App/iosApp/build/classes");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(names(&found.projects[0]), ["iosApp/build"]);
    }

    #[test]
    fn a_composite_build_belongs_to_the_project_that_includes_it() {
        let fx = Fixture::new();
        fx.file("Documents/App/settings.gradle.kts");
        fx.file("Documents/App/build-logic/settings.gradle.kts");
        fx.file("Documents/App/build-logic/build.gradle.kts");
        fx.dir("Documents/App/build-logic/build/classes");
        fx.dir("Documents/App/build-logic/.gradle");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.projects.len(), 1);
        assert_eq!(names(&found.projects[0]), ["build-logic/build"]);
    }

    #[test]
    fn a_project_reached_twice_is_listed_once() {
        let fx = Fixture::new();
        kmp(&fx, "Documents/App");
        let docs = fx.home().join("Documents");

        let found = find_projects(&fx.env, &[docs.clone(), docs.join("App")]);

        assert_eq!(found.projects.len(), 1);
    }

    #[test]
    fn a_project_with_nothing_to_remove_is_not_listed() {
        let fx = Fixture::new();
        fx.file("Documents/Clean/settings.gradle.kts");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.projects, vec![]);
    }

    #[test]
    fn modules_belong_to_the_project_with_the_settings_file() {
        let fx = Fixture::new();
        fx.file("Projects/Big/settings.gradle.kts");
        fx.file("Projects/Big/feature/auth/build.gradle.kts");
        fx.dir("Projects/Big/feature/auth/build");
        fx.file("Projects/Lone/build.gradle");
        fx.dir("Projects/Lone/build");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        let summary: Vec<_> = found
            .projects
            .iter()
            .map(|p| (p.name.as_str(), names(p)))
            .collect();
        assert_eq!(
            summary,
            [("Big", vec!["feature/auth/build"]), ("Lone", vec!["build"])]
        );
    }

    #[test]
    fn hidden_folders_build_output_and_node_modules_are_not_searched() {
        let fx = Fixture::new();
        fx.file("Documents/.hidden/settings.gradle.kts");
        fx.dir("Documents/.hidden/build");
        fx.file("Documents/node_modules/x/settings.gradle.kts");
        fx.dir("Documents/node_modules/x/build");
        fx.file("Documents/App/build/inner/settings.gradle.kts");
        fx.dir("Documents/App/build/inner/build");

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.projects, vec![], "{found:?}");
    }

    #[test]
    fn a_symlinked_project_is_not_followed() {
        let fx = Fixture::new();
        kmp(&fx, "elsewhere/App");
        fx.dir("Documents");
        symlink(
            fx.home().join("elsewhere/App"),
            fx.home().join("Documents/Linked"),
        )
        .unwrap();

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.projects, vec![], "{found:?}");
    }

    #[test]
    fn a_build_folder_that_is_a_link_is_offered_as_the_link() {
        let fx = Fixture::new();
        fx.file("Documents/App/settings.gradle.kts");
        symlink(&fx.outside, fx.home().join("Documents/App/build")).unwrap();

        let found = find_projects(&fx.env, &default_folders(&fx.env));

        assert_eq!(found.projects[0].items[0].size_bytes, 0);
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn paths_must_be_folders_inside_home() {
        let fx = Fixture::new();
        fx.dir("Code");
        let outside = fx.outside.display().to_string();
        let ok = |text: &str| folders_from(&fx.env, &[text.to_string()]);

        assert_eq!(
            ok("~/Code").unwrap(),
            [fx.home().join("Code").canonicalize().unwrap()]
        );
        assert!(ok("Code").unwrap_err().contains("must start with"));
        assert!(ok("~/missing").unwrap_err().contains("cannot be searched"));
        assert!(ok(&outside).unwrap_err().contains("inside your home"));
        assert!(ok(&fx.home().display().to_string()).is_err());
        assert!(ok("~/Code/../..").is_err());
    }
}
