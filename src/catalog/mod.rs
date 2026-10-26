//! The fixed list of caches lfmclean knows about. Nothing outside this list is ever offered, and
//! every source below sits inside one of the cache folders `guard` allows.

mod android;
mod discover;
mod emulators;
mod gradle;
mod simulators;
mod xcode;

use serde::Serialize;

pub use discover::{Candidate, Discovery, Skip, discover};
pub use emulators::attach_ini_files;
pub use simulators::{SimctlError, Simulator, unavailable_simulators};

/// The group a target is shown under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Xcode, simulators, Swift Package Manager and `CocoaPods`.
    Xcode,
    /// Android Studio, emulators and system images.
    Android,
    /// Gradle and Kotlin/Native.
    GradleKotlin,
}

impl Category {
    /// The heading shown for the group.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Xcode => "Xcode",
            Self::Android => "Android",
            Self::GradleKotlin => "Gradle & Kotlin",
        }
    }
}

/// How careful to be with a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Safety {
    /// Rebuilt or downloaded again with no loss.
    Safe,
    /// Rebuilt or downloaded again, but it takes a while or needs a network.
    Caution,
    /// Holds something that cannot be made again.
    Risky,
}

impl Safety {
    /// The word shown next to a target.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Caution => "caution",
            Self::Risky => "risky",
        }
    }
}

/// Which entries at the end of a source count.
#[derive(Debug, Clone, Copy)]
pub enum Filter {
    /// Every entry.
    Any,
    /// Folders only.
    DirsOnly,
    /// Names starting with the text.
    Prefix(&'static str),
    /// Names ending with the text.
    Suffix(&'static str),
    /// Exactly these names.
    Named(&'static [&'static str]),
}

impl Filter {
    pub(crate) fn matches(self, name: &str, is_dir: bool) -> bool {
        match self {
            Self::Any => true,
            Self::DirsOnly => is_dir,
            Self::Prefix(prefix) => name.starts_with(prefix),
            Self::Suffix(suffix) => name.ends_with(suffix),
            Self::Named(names) => names.contains(&name),
        }
    }
}

/// Where a target's items are: the entries `depth` levels below `root/sub`.
#[derive(Debug, Clone, Copy)]
pub struct Source {
    /// A cache folder `guard` allows, relative to home. Items are removed inside it.
    pub root: &'static str,
    /// A folder inside `root` to look in, or empty to look in `root` itself.
    pub sub: &'static str,
    /// How many levels below `root/sub` an item is; at least 1.
    pub depth: usize,
    /// Which entries at that depth count.
    pub filter: Filter,
}

impl Source {
    /// The entries directly inside `root`.
    #[must_use]
    pub const fn children(root: &'static str, filter: Filter) -> Self {
        Self::nested(root, "", 1, filter)
    }

    /// The entries `depth` levels below `root/sub`.
    #[must_use]
    pub const fn nested(
        root: &'static str,
        sub: &'static str,
        depth: usize,
        filter: Filter,
    ) -> Self {
        Self {
            root,
            sub,
            depth,
            filter,
        }
    }
}

/// How a target's items are found and removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Plain files and folders, from the sources.
    Paths,
    /// Android emulators: each folder goes with the `.ini` file that points to it.
    Emulators,
    /// Simulators `simctl` reports as unavailable, removed with `simctl`, not by path.
    UnavailableSimulators,
}

/// One kind of cache: what it is, how careful to be, and where it lives.
#[derive(Debug)]
pub struct Target {
    /// A stable id such as `xcode.derived-data`, used by the keep list and `clean --ids`.
    pub id: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    /// The group it is shown under.
    pub category: Category,
    /// How careful to be.
    pub safety: Safety,
    /// What happens after it is removed.
    pub effect: &'static str,
    /// How its items are found and removed.
    pub kind: Kind,
    /// Where its items are; empty for `UnavailableSimulators`.
    pub sources: &'static [Source],
}

/// Every target, grouped as shown: Xcode, Android, then Gradle and Kotlin.
pub fn all_targets() -> impl Iterator<Item = &'static Target> {
    xcode::TARGETS
        .iter()
        .chain(android::TARGETS)
        .chain(gradle::TARGETS)
}

/// Project package checkouts inside `DerivedData`, browsed package by package but not part of a
/// whole scan, which already counts them as `DerivedData`.
pub static SOURCE_PACKAGES: &Target = &xcode::SOURCE_PACKAGES;

/// The target with this id, including `SOURCE_PACKAGES`.
#[must_use]
pub fn find(id: &str) -> Option<&'static Target> {
    all_targets()
        .chain(std::iter::once(SOURCE_PACKAGES))
        .find(|target| target.id == id)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = all_targets().map(|target| target.id).collect();
        assert_eq!(ids.len(), all_targets().count());
    }

    #[test]
    fn every_source_looks_at_least_one_level_down_inside_home() {
        for target in all_targets() {
            for source in target.sources {
                assert!(source.depth >= 1, "{}: depth", target.id);
                assert!(!source.root.starts_with('/'), "{}: root", target.id);
                assert!(!source.root.contains(".."), "{}: root", target.id);
                assert!(!source.sub.starts_with('/'), "{}: sub", target.id);
                assert!(!source.sub.contains(".."), "{}: sub", target.id);
            }
        }
    }

    #[test]
    fn only_simulators_have_no_sources() {
        for target in all_targets() {
            let none = target.sources.is_empty();
            assert_eq!(
                none,
                target.kind == Kind::UnavailableSimulators,
                "{}",
                target.id
            );
        }
    }

    #[test]
    fn find_returns_the_target_with_that_id() {
        assert_eq!(find("gradle.caches").unwrap().name, "Gradle caches");
        assert!(find("nope").is_none());
    }

    #[test]
    fn risky_targets_are_the_ones_that_cannot_be_made_again() {
        let risky: Vec<_> = all_targets()
            .filter(|target| target.safety == Safety::Risky)
            .map(|target| target.id)
            .collect();
        assert_eq!(
            risky,
            ["xcode.archives", "android.avd", "android.system-images"]
        );
    }
}
