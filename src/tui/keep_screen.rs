//! The keep list as a screen: every entry, marked with space to stop keeping it, saved with enter.

use std::collections::BTreeSet;

use super::app::Key;
use crate::keep::KeepList;

/// One entry of the keep list as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepLine {
    /// `target`, `package` or `path`.
    pub kind: &'static str,
    /// The entry as written in the file.
    pub text: String,
}

/// Every entry of `keep`, targets first.
#[must_use]
pub fn lines(keep: &KeepList) -> Vec<KeepLine> {
    let targets = keep.targets().map(|id| KeepLine {
        kind: "target",
        text: id.to_string(),
    });
    let packages = keep.packages().map(|p| KeepLine {
        kind: "package",
        text: p.to_string(),
    });
    let paths = keep.paths().map(|p| KeepLine {
        kind: "path",
        text: p.display().to_string(),
    });
    targets.chain(packages).chain(paths).collect()
}

/// The cursor and the entries marked to stop keeping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeepScreen {
    /// The line under the cursor.
    pub cursor: usize,
    /// Lines marked with space.
    pub marked: BTreeSet<usize>,
}

/// What a key on the keep screen means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Still editing.
    Wait(KeepScreen),
    /// Leave without changes.
    Back,
    /// Save this list.
    Save(KeepList),
}

impl KeepScreen {
    /// What a key means for `keep`.
    #[must_use]
    pub fn on_key(&self, key: Key, keep: &KeepList) -> Answer {
        let count = lines(keep).len();
        let mut next = self.clone();
        match key {
            Key::Up => next.cursor = next.cursor.saturating_sub(1),
            Key::Down => next.cursor = (next.cursor + 1).min(count.saturating_sub(1)),
            Key::Char(' ') if count > 0 => {
                if !next.marked.remove(&next.cursor) {
                    next.marked.insert(next.cursor);
                }
            }
            Key::Enter if self.marked.is_empty() => return Answer::Back,
            Key::Enter => return Answer::Save(without(keep, &self.marked)),
            Key::Esc | Key::Char('q') => return Answer::Back,
            _ => {}
        }
        Answer::Wait(next)
    }
}

/// `keep` without the marked lines.
fn without(keep: &KeepList, marked: &BTreeSet<usize>) -> KeepList {
    let targets: Vec<&str> = keep.targets().collect();
    let packages: Vec<_> = keep.packages().cloned().collect();
    let paths: Vec<_> = keep.paths().map(std::path::Path::to_path_buf).collect();
    let mut next = keep.clone();
    for &line in marked {
        let package_at = line.checked_sub(targets.len());
        let path_at = package_at.and_then(|at| at.checked_sub(packages.len()));
        next = if let Some(id) = targets.get(line) {
            next.without_target(id)
        } else if let Some(path) = path_at.and_then(|at| paths.get(at)) {
            next.without_path(path)
        } else if let Some(package) = package_at.and_then(|at| packages.get(at)) {
            next.without_package(package)
        } else {
            next
        };
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keep::KeptPackage;
    use crate::testing::Fixture;

    #[test]
    fn marked_lines_of_every_kind_are_removed_and_the_rest_stay() {
        let fx = Fixture::new();
        let keep = KeepList::empty(&fx.env)
            .with_target("xcode.archives")
            .unwrap()
            .with_target("android.avd")
            .unwrap()
            .with_package("pods:Firebase@10.20.0".parse::<KeptPackage>().unwrap())
            .with_package("spm:SnapKit".parse::<KeptPackage>().unwrap())
            .with_path("~/Projects/App")
            .unwrap()
            .with_path("~/Projects/Lib")
            .unwrap();
        let shown: Vec<String> = lines(&keep).into_iter().map(|line| line.text).collect();
        let at = |text: &str| shown.iter().position(|s| s.contains(text)).unwrap();
        let marked = BTreeSet::from([at("android.avd"), at("Firebase"), at("Lib")]);

        let left = without(&keep, &marked);

        let left: Vec<String> = lines(&left).into_iter().map(|line| line.text).collect();
        assert_eq!(left.len(), 3, "{left:?}");
        assert!(left.iter().any(|s| s.contains("xcode.archives")));
        assert!(left.iter().any(|s| s.contains("SnapKit")));
        assert!(left.iter().any(|s| s.contains("App")));
    }
}
