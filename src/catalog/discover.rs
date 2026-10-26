//! Listing what a source holds. Discovery only reads folder names; the guard decides what may be
//! removed, and sizes come later.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Source;
use crate::env::Env;

/// File names that are never offered as items.
const IGNORED_NAMES: [&str; 2] = ["CACHEDIR.TAG", ".DS_Store"];

/// One thing that can be removed: its name, and the paths that go together (the main entry first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The path below the source, such as `App-abc` or `2026-10-26/App.xcarchive`.
    pub name: String,
    /// The entry itself, then anything removed along with it.
    pub paths: Vec<PathBuf>,
    /// The cache folder the entry is removed inside.
    pub root: PathBuf,
}

/// A path that could not be listed or was refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    /// The path concerned.
    pub path: PathBuf,
    /// Why it is not offered.
    pub reason: String,
}

/// What a source holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Entries found, sorted by name.
    pub candidates: Vec<Candidate>,
    /// Folders that could not be read.
    pub skipped: Vec<Skip>,
}

/// Lists the entries of `source`. A folder that does not exist is simply empty.
#[must_use]
pub fn discover(env: &Env, source: &Source) -> Discovery {
    let root = env.home().join(source.root);
    let base = root.join(source.sub);
    let mut found = Discovery::default();
    list(&base, &base, &root, source, source.depth, &mut found);
    found
        .candidates
        .sort_by_key(|candidate| candidate.name.to_lowercase());
    found
}

/// Lists `dir`; at the last level the filter picks items, above it only real folders are
/// entered (a symlink is never followed on the way down).
fn list(
    dir: &Path,
    base: &Path,
    root: &Path,
    source: &Source,
    levels_left: usize,
    found: &mut Discovery,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            found.skipped.push(unreadable(dir, &err));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                found.skipped.push(unreadable(dir, &err));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if IGNORED_NAMES.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                found.skipped.push(unreadable(&path, &err));
                continue;
            }
        };
        if levels_left > 1 {
            if file_type.is_dir() {
                list(&path, base, root, source, levels_left - 1, found);
            }
        } else if source.filter.matches(&name, file_type.is_dir()) {
            found.candidates.push(Candidate {
                name: relative_name(&path, base),
                paths: vec![path],
                root: root.to_path_buf(),
            });
        }
    }
}

fn relative_name(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn unreadable(path: &Path, err: &io::Error) -> Skip {
    Skip {
        path: path.to_path_buf(),
        reason: format!("cannot read it: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;
    use crate::catalog::Filter;
    use crate::testing::Fixture;

    const ROOT: &str = ".gradle/caches";

    fn names(found: &Discovery) -> Vec<&str> {
        found.candidates.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn lists_the_children_sorted_by_name_ignoring_case() {
        let fx = Fixture::new();
        fx.dir(".gradle/caches/modules-2");
        fx.dir(".gradle/caches/Beta");
        fx.dir(".gradle/caches/alpha");

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        assert_eq!(names(&found), ["alpha", "Beta", "modules-2"]);
        assert_eq!(found.skipped, vec![]);
    }

    #[test]
    fn a_candidate_knows_its_path_and_root() {
        let fx = Fixture::new();
        let item = fx.dir(".gradle/caches/modules-2");

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        assert_eq!(found.candidates[0].paths, vec![item]);
        assert_eq!(found.candidates[0].root, fx.home().join(ROOT));
    }

    #[test]
    fn skips_marker_files() {
        let fx = Fixture::new();
        fx.dir(".gradle/caches/modules-2");
        fx.file(".gradle/caches/CACHEDIR.TAG");
        fx.file(".gradle/caches/.DS_Store");

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        assert_eq!(names(&found), ["modules-2"]);
    }

    #[test]
    fn a_missing_folder_is_empty_and_not_an_error() {
        let fx = Fixture::new();

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        assert_eq!(found, Discovery::default());
    }

    #[test]
    fn deeper_items_are_named_by_their_path_below_the_source() {
        let fx = Fixture::new();
        fx.dir("Library/Developer/Xcode/Archives/2026-10-26/App.xcarchive");
        fx.dir("Library/Developer/Xcode/Archives/2026-10-26/notes");
        let source = Source::nested(
            "Library/Developer/Xcode/Archives",
            "",
            2,
            Filter::Suffix(".xcarchive"),
        );

        let found = discover(&fx.env, &source);

        assert_eq!(names(&found), ["2026-10-26/App.xcarchive"]);
    }

    #[test]
    fn a_sub_folder_moves_the_start() {
        let fx = Fixture::new();
        fx.dir("Library/Caches/org.swift.swiftpm/repositories/SnapKit-abc");
        fx.dir("Library/Caches/org.swift.swiftpm/manifests");
        let source = Source::nested(
            "Library/Caches/org.swift.swiftpm",
            "repositories",
            1,
            Filter::DirsOnly,
        );

        assert_eq!(names(&discover(&fx.env, &source)), ["SnapKit-abc"]);
    }

    #[test]
    fn symlinks_are_not_entered_on_the_way_down() {
        let fx = Fixture::new();
        fx.dir(".gradle/caches/real/item");
        symlink(&fx.outside, fx.home().join(".gradle/caches/link")).unwrap();

        let found = discover(&fx.env, &Source::nested(ROOT, "", 2, Filter::Any));

        assert_eq!(names(&found), ["real/item"]);
    }

    #[test]
    fn a_symlink_at_the_item_level_is_listed_as_an_item() {
        let fx = Fixture::new();
        fs::create_dir_all(fx.home().join(ROOT)).unwrap();
        symlink(&fx.outside, fx.home().join(".gradle/caches/link")).unwrap();

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        assert_eq!(names(&found), ["link"]);
    }

    #[test]
    fn filters_pick_by_name_and_kind() {
        let fx = Fixture::new();
        fx.dir(".konan/kotlin-native-prebuilt-macos-aarch64-2.0.0");
        fx.dir(".konan/dependencies");
        fx.file(".konan/notes.txt");
        let root = ".konan";
        let by = |filter| names_of(&fx, Source::children(root, filter));

        assert_eq!(by(Filter::Prefix("kotlin-native-prebuilt-")).len(), 1);
        assert_eq!(by(Filter::Named(&["dependencies"])), ["dependencies"]);
        assert_eq!(by(Filter::Suffix(".txt")), ["notes.txt"]);
        assert_eq!(by(Filter::DirsOnly).len(), 2);
        assert_eq!(by(Filter::Any).len(), 3);
    }

    fn names_of(fx: &Fixture, source: Source) -> Vec<String> {
        discover(&fx.env, &source)
            .candidates
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn an_unreadable_folder_is_reported() {
        let fx = Fixture::new();
        let root = fx.dir(ROOT);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();

        let found = discover(&fx.env, &Source::children(ROOT, Filter::Any));

        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(found.candidates, vec![]);
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.skipped[0].path, root);
        assert!(found.skipped[0].reason.contains("cannot read it"));
    }
}
