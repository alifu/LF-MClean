//! Which folders of a project may be removed, for `purge`. The cache roots in `rules` do not apply
//! to projects, which live anywhere in the home folder, so the entry itself is what is checked: its
//! name, and the files beside it that show it is what the name suggests.

use std::fs;
use std::path::Path;

/// Gradle files that make a folder a Gradle project or module.
const GRADLE_FILES: &[&str] = &[
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
];
/// The folder of the iOS app of a Kotlin Multiplatform project.
const IOS_APP: &str = "iosApp";
const XCODE_SUFFIXES: &[&str] = &[".xcodeproj", ".xcworkspace"];
const PODFILE: &str = "Podfile";
/// A project is searched this many folders deep for modules.
pub(super) const MAX_DEPTH_IN_PROJECT: usize = 8;

/// True when `entry` (inside `root`) is a folder `purge` may remove.
///
/// - `build`: beside a Gradle file, or inside `iosApp` beside an Xcode project;
/// - `.gradle`, `.kotlin`: directly in the project folder, beside a Gradle file;
/// - `Pods`: in `iosApp`, beside a `Podfile`.
pub(super) fn is_removable_project_folder(entry: &Path, root: &Path) -> bool {
    let (Some(name), Some(parent)) = (entry.file_name().and_then(|n| n.to_str()), entry.parent())
    else {
        return false;
    };
    let Ok(inside) = entry.strip_prefix(root) else {
        return false;
    };
    if inside.components().count() > MAX_DEPTH_IN_PROJECT {
        return false;
    }
    // A folder (or a link to one, removed as a link); a plain file named `build` is not one.
    if !fs::symlink_metadata(entry).is_ok_and(|meta| !meta.is_file()) {
        return false;
    }
    match name {
        "build" => {
            has_any(parent, GRADLE_FILES) || (is_ios_app(parent) && has_xcode_project(parent))
        }
        ".gradle" | ".kotlin" => parent == root && has_any(parent, GRADLE_FILES),
        "Pods" => is_ios_app(parent) && has_file(parent, PODFILE),
        _ => false,
    }
}

fn is_ios_app(folder: &Path) -> bool {
    folder.file_name().is_some_and(|name| name == IOS_APP)
}

/// Files only, not links: a link named `build.gradle` proves nothing.
fn has_file(folder: &Path, name: &str) -> bool {
    fs::symlink_metadata(folder.join(name)).is_ok_and(|meta| meta.is_file())
}

fn has_any(folder: &Path, names: &[&str]) -> bool {
    names.iter().any(|name| has_file(folder, name))
}

fn has_xcode_project(folder: &Path) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|kind| kind.is_dir())
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| XCODE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fixture;

    fn removable(fx: &Fixture, entry: &str, root: &str) -> bool {
        is_removable_project_folder(&fx.home().join(entry), &fx.home().join(root))
    }

    #[test]
    fn build_folders_beside_a_gradle_file_are_removable() {
        let fx = Fixture::new();
        fx.file("p/build.gradle.kts");
        fx.dir("p/build");
        fx.file("p/composeApp/build.gradle.kts");
        fx.dir("p/composeApp/build");

        assert!(removable(&fx, "p/build", "p"));
        assert!(removable(&fx, "p/composeApp/build", "p"));
    }

    #[test]
    fn a_build_folder_without_a_gradle_file_is_not() {
        let fx = Fixture::new();
        fx.dir("p/build");
        fx.file("p/settings.gradle.kts");
        fx.dir("p/docs/build");

        assert!(removable(&fx, "p/build", "p"));
        assert!(!removable(&fx, "p/docs/build", "p"));
    }

    #[test]
    fn a_link_named_like_a_gradle_file_proves_nothing() {
        let fx = Fixture::new();
        fx.dir("p/build");
        std::os::unix::fs::symlink(
            fx.file("elsewhere.kts"),
            fx.home().join("p/build.gradle.kts"),
        )
        .unwrap();

        assert!(!removable(&fx, "p/build", "p"));
    }

    #[test]
    fn the_ios_app_build_needs_an_xcode_project() {
        let fx = Fixture::new();
        fx.dir("p/iosApp/build");
        assert!(!removable(&fx, "p/iosApp/build", "p"));

        fx.dir("p/iosApp/iosApp.xcodeproj");

        assert!(removable(&fx, "p/iosApp/build", "p"));
    }

    #[test]
    fn dot_gradle_and_dot_kotlin_only_at_the_project_folder() {
        let fx = Fixture::new();
        fx.file("p/settings.gradle.kts");
        fx.dir("p/.gradle");
        fx.dir("p/.kotlin");
        fx.file("p/app/build.gradle.kts");
        fx.dir("p/app/.gradle");

        assert!(removable(&fx, "p/.gradle", "p"));
        assert!(removable(&fx, "p/.kotlin", "p"));
        assert!(!removable(&fx, "p/app/.gradle", "p"));
    }

    #[test]
    fn pods_need_a_podfile() {
        let fx = Fixture::new();
        fx.dir("p/iosApp/Pods");
        assert!(!removable(&fx, "p/iosApp/Pods", "p"));

        fx.file("p/iosApp/Podfile");

        assert!(removable(&fx, "p/iosApp/Pods", "p"));
    }

    #[test]
    fn a_plain_file_named_build_is_not_removable() {
        let fx = Fixture::new();
        fx.file("p/build.gradle.kts");
        fx.file("p/build");

        assert!(!removable(&fx, "p/build", "p"));
    }

    #[test]
    fn other_names_are_never_removable() {
        let fx = Fixture::new();
        fx.file("p/build.gradle.kts");
        for name in ["src", "gradle", "Build", "local.properties"] {
            fx.dir(&format!("p/{name}"));

            assert!(!removable(&fx, &format!("p/{name}"), "p"), "{name}");
        }
    }
}
