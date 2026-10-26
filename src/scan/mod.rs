//! Finding the caches on this Mac and measuring them.
//!
//! Every path an item offers has passed `guard::check`, so what is listed can be removed: a path
//! the guard refuses is reported as skipped with its reason, never offered.

pub mod render;
pub mod size;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, FixedOffset, Local};

use crate::catalog::{self, Candidate, Discovery, Kind, Simulator, Source, Target};
use crate::command::CommandRunner;
use crate::env::Env;
use crate::guard::{self, SafePath};

/// How an item is removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// Remove `main`; the `with` paths go with it and are removed only once `main` is gone.
    Paths {
        /// The main entry.
        main: SafePath,
        /// Entries that belong to it, such as the `.ini` file of an emulator.
        with: Vec<SafePath>,
    },
    /// Run `xcrun simctl delete` for each of these simulators.
    SimctlDelete {
        /// Simulators that passed `catalog::unavailable_simulators`.
        simulators: Vec<Simulator>,
    },
}

impl Removal {
    /// Every path this removal touches, main entry first.
    #[must_use]
    pub fn paths(&self) -> Vec<&SafePath> {
        match self {
            Self::Paths { main, with } => std::iter::once(main).chain(with).collect(),
            Self::SimctlDelete { .. } => Vec::new(),
        }
    }

    /// Every folder this removal deletes, including the simulator folders `simctl` deletes.
    #[must_use]
    pub fn folders(&self) -> Vec<&Path> {
        match self {
            Self::Paths { .. } => self.paths().into_iter().map(SafePath::path).collect(),
            Self::SimctlDelete { simulators } => {
                simulators.iter().map(|sim| sim.folder.as_path()).collect()
            }
        }
    }

    /// The simulator ids, for `SimctlDelete`.
    #[must_use]
    pub fn udids(&self) -> Vec<&str> {
        match self {
            Self::Paths { .. } => Vec::new(),
            Self::SimctlDelete { simulators } => {
                simulators.iter().map(|sim| sim.udid.as_str()).collect()
            }
        }
    }
}

/// One thing that can be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The id of the target it belongs to.
    pub target: &'static str,
    /// The name shown to the user.
    pub name: String,
    /// How it is removed.
    pub removal: Removal,
    /// Disk space it takes.
    pub size_bytes: u64,
    /// True when some folders could not be read, so `size_bytes` is a lower bound.
    pub partial: bool,
    /// When it was last changed.
    pub modified: Option<DateTime<FixedOffset>>,
}

impl Item {
    /// A key that tells this item from every other, even when two have the same name.
    #[must_use]
    pub fn key(&self) -> String {
        let what = match &self.removal {
            Removal::Paths { main, .. } => main.path().display().to_string(),
            Removal::SimctlDelete { .. } => self.removal.udids().join(","),
        };
        format!("{}:{what}", self.target)
    }
}

/// Something that was found but is not offered, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The id of the target it was found under.
    pub target: &'static str,
    /// The path concerned, when there is one.
    pub path: Option<PathBuf>,
    /// Why it is not offered.
    pub reason: String,
}

/// What one target holds.
#[derive(Debug)]
pub struct TargetReport {
    /// The target.
    pub target: &'static Target,
    /// Items that can be removed, sorted by name.
    pub items: Vec<Item>,
    /// Things that are not offered.
    pub skipped: Vec<Skipped>,
}

impl TargetReport {
    /// Total size of the items.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|item| item.size_bytes).sum()
    }
}

/// What a whole scan found, one entry per target in catalog order.
#[derive(Debug)]
pub struct Report {
    /// Every target, including those with nothing to remove.
    pub targets: Vec<TargetReport>,
}

impl Report {
    /// Total size of everything that can be removed.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.targets.iter().map(TargetReport::total_bytes).sum()
    }
}

/// Scans targets for one user.
pub struct Scanner<'a> {
    env: &'a Env,
    runner: &'a dyn CommandRunner,
}

impl<'a> Scanner<'a> {
    /// A scanner for `env` that runs helper programs through `runner`.
    #[must_use]
    pub fn new(env: &'a Env, runner: &'a dyn CommandRunner) -> Self {
        Self { env, runner }
    }

    /// Scans every target in the catalog, calling `on_target` just before each one.
    pub fn scan_all(&self, on_target: &mut dyn FnMut(&'static Target)) -> Report {
        let targets = catalog::all_targets()
            .map(|target| {
                on_target(target);
                self.scan_target(target)
            })
            .collect();
        Report { targets }
    }

    /// Scans one target.
    #[must_use]
    pub fn scan_target(&self, target: &'static Target) -> TargetReport {
        let mut items = Vec::new();
        let mut skipped = Vec::new();
        match target.kind {
            Kind::UnavailableSimulators => match self.simulators(target) {
                Ok(item) => items.extend(item),
                Err(skip) => skipped.push(skip),
            },
            Kind::Paths | Kind::Emulators => {
                for source in target.sources {
                    let found = self.discover(target, source);
                    skipped.extend(found.skipped.into_iter().map(|skip| Skipped {
                        target: target.id,
                        path: Some(skip.path),
                        reason: skip.reason,
                    }));
                    for candidate in found.candidates {
                        match self.item_for(target, candidate) {
                            Ok(item) => items.push(item),
                            Err(skip) => skipped.push(skip),
                        }
                    }
                }
            }
        }
        items.sort_by_key(|item| item.name.to_lowercase());
        TargetReport {
            target,
            items,
            skipped,
        }
    }

    fn discover(&self, target: &Target, source: &Source) -> Discovery {
        let found = catalog::discover(self.env, source);
        if target.kind == Kind::Emulators {
            return catalog::attach_ini_files(found);
        }
        found
    }

    /// Checks every path of the candidate with the guard, then measures them together.
    fn item_for(&self, target: &Target, candidate: Candidate) -> Result<Item, Skipped> {
        let safe = candidate
            .paths
            .iter()
            .map(|path| {
                guard::check(path, &candidate.root, self.env).map_err(|err| Skipped {
                    target: target.id,
                    path: Some(path.clone()),
                    reason: err.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let usages: Vec<_> = safe.iter().map(|s| size::disk_usage(s.path())).collect();
        let modified = safe.iter().filter_map(|s| modified(s.path())).max();
        let mut safe = safe.into_iter();
        let main = safe.next().expect("a candidate has at least one path");
        Ok(Item {
            target: target.id,
            name: candidate.name,
            size_bytes: usages.iter().map(|usage| usage.bytes).sum(),
            partial: usages.iter().any(|usage| usage.unreadable > 0),
            modified,
            removal: Removal::Paths {
                main,
                with: safe.collect(),
            },
        })
    }

    /// One item for all unavailable simulators, which `simctl` removes together.
    fn simulators(&self, target: &Target) -> Result<Option<Item>, Skipped> {
        let simulators =
            catalog::unavailable_simulators(self.env, self.runner).map_err(|err| Skipped {
                target: target.id,
                path: None,
                reason: err.to_string(),
            })?;
        if simulators.is_empty() {
            return Ok(None);
        }
        let usages: Vec<_> = simulators
            .iter()
            .filter(|sim| fs::symlink_metadata(&sim.folder).is_ok())
            .map(|sim| size::disk_usage(&sim.folder))
            .collect();
        Ok(Some(Item {
            target: target.id,
            name: format!("Unavailable simulators ({})", simulators.len()),
            removal: Removal::SimctlDelete { simulators },
            size_bytes: usages.iter().map(|usage| usage.bytes).sum(),
            partial: usages.iter().any(|usage| usage.unreadable > 0),
            modified: None,
        }))
    }
}

fn modified(path: &Path) -> Option<DateTime<FixedOffset>> {
    let time: SystemTime = fs::symlink_metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Local>::from(time).fixed_offset())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::*;
    use crate::catalog::{Filter, Source};
    use crate::testing::{FakeRunner, Fixture};

    const NO_SIMULATORS: &str = r#"{"devices": {}}"#;

    fn scan(fx: &Fixture, id: &str) -> TargetReport {
        scan_with(fx, id, &FakeRunner::answering(NO_SIMULATORS))
    }

    fn scan_with(fx: &Fixture, id: &str, runner: &FakeRunner) -> TargetReport {
        Scanner::new(&fx.env, runner).scan_target(catalog::find(id).unwrap())
    }

    fn names(report: &TargetReport) -> Vec<&str> {
        report.items.iter().map(|item| item.name.as_str()).collect()
    }

    fn paths(item: &Item) -> Vec<&Path> {
        item.removal
            .paths()
            .into_iter()
            .map(SafePath::path)
            .collect()
    }

    #[test]
    fn a_target_with_nothing_on_disk_has_no_items_and_no_noise() {
        let fx = Fixture::new();

        let report = scan(&fx, "gradle.caches");

        assert_eq!(report.items, vec![]);
        assert_eq!(report.skipped, vec![]);
    }

    #[test]
    fn items_are_measured_and_sorted_by_name() {
        let fx = Fixture::new();
        let modules = fx.dir(".gradle/caches/modules-2");
        fx.dir(".gradle/caches/Build-cache-1");

        let report = scan(&fx, "gradle.caches");

        assert_eq!(names(&report), ["Build-cache-1", "modules-2"]);
        let item = &report.items[1];
        assert_eq!(item.target, "gradle.caches");
        assert_eq!(item.size_bytes, size::disk_usage(&modules).bytes);
        assert!(item.size_bytes > 0);
        assert!(!item.partial);
        assert!(item.modified.is_some());
        assert_eq!(paths(item), [modules.canonicalize().unwrap()]);
        assert_eq!(
            report.total_bytes(),
            report.items.iter().map(|i| i.size_bytes).sum::<u64>()
        );
    }

    #[test]
    fn several_sources_make_one_list() {
        let fx = Fixture::new();
        fx.dir("Library/Developer/Xcode/iOS DeviceSupport/iPhone 26.5");
        fx.dir("Library/Developer/Xcode/watchOS DeviceSupport/Watch 11.0");

        let report = scan(&fx, "xcode.device-support");

        assert_eq!(names(&report), ["iPhone 26.5", "Watch 11.0"]);
    }

    #[test]
    fn an_unreadable_folder_inside_an_item_marks_it_partial() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let locked = fx.dir(".gradle/caches/modules-2/locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let report = scan(&fx, "gradle.caches");

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(report.items[0].partial);
    }

    #[test]
    fn a_path_the_guard_refuses_is_skipped_with_the_reason() {
        let fx = Fixture::new();
        fx.dir(".gradle/caches/modules-2");
        let runner = FakeRunner::answering(NO_SIMULATORS);
        let other_user = fx.other_user_env();

        let report =
            Scanner::new(&other_user, &runner).scan_target(catalog::find("gradle.caches").unwrap());

        assert_eq!(report.items, vec![]);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].target, "gradle.caches");
        assert!(report.skipped[0].reason.contains("belongs to another user"));
        assert!(
            report.skipped[0]
                .path
                .as_ref()
                .unwrap()
                .ends_with("modules-2")
        );
    }

    #[test]
    fn a_cache_folder_that_is_a_symlink_to_elsewhere_offers_nothing() {
        let fx = Fixture::new();
        fs::create_dir_all(fx.home().join(".gradle")).unwrap();
        symlink(&fx.outside, fx.home().join(".gradle/caches")).unwrap();

        let report = scan(&fx, "gradle.caches");

        assert_eq!(report.items, vec![]);
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0]
                .reason
                .contains("not one of the cache folders")
        );
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn an_emulator_is_one_item_with_its_ini_and_the_size_of_both() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/Medium_Phone.avd");
        let ini = fx.home().join(".android/avd/Medium_Phone_API_36.ini");
        fs::write(&ini, "path.rel=avd/Medium_Phone.avd\n").unwrap();

        let report = scan(&fx, "android.avd");

        assert_eq!(names(&report), ["Medium_Phone"]);
        let item = &report.items[0];
        assert_eq!(
            paths(item),
            [avd.canonicalize().unwrap(), ini.canonicalize().unwrap()]
        );
        assert_eq!(
            item.size_bytes,
            size::disk_usage(&avd).bytes + size::disk_usage(&ini).bytes
        );
    }

    #[test]
    fn unavailable_simulators_are_one_item_removed_with_simctl() {
        let fx = Fixture::new();
        let devices = "Library/Developer/CoreSimulator/Devices";
        let first = fx.dir(&format!("{devices}/AAAA/data"));
        let second = fx.dir(&format!("{devices}/BBBB/data"));
        let listing = format!(
            r#"{{"devices": {{"rt": [
                {{"udid": "AAAA", "dataPath": "{}"}},
                {{"udid": "BBBB", "dataPath": "{}"}},
                {{"udid": "CCCC", "dataPath": "{}/CCCC/data"}}
            ]}}}}"#,
            first.display(),
            second.display(),
            fx.home().join(devices).display(),
        );

        let report = scan_with(
            &fx,
            "xcode.sim-unavailable",
            &FakeRunner::answering(&listing),
        );

        assert_eq!(names(&report), ["Unavailable simulators (3)"]);
        let item = &report.items[0];
        assert!(matches!(item.removal, Removal::SimctlDelete { .. }));
        let mut udids = item.removal.udids();
        udids.sort_unstable();
        assert_eq!(udids, ["AAAA", "BBBB", "CCCC"]);
        let expected = size::disk_usage(first.parent().unwrap()).bytes
            + size::disk_usage(second.parent().unwrap()).bytes;
        assert_eq!(item.size_bytes, expected);
        assert!(!item.partial);
    }

    #[test]
    fn no_unavailable_simulators_means_no_item() {
        let fx = Fixture::new();

        let report = scan(&fx, "xcode.sim-unavailable");

        assert_eq!(report.items, vec![]);
        assert_eq!(report.skipped, vec![]);
    }

    #[test]
    fn simctl_failing_is_a_skip_not_an_error() {
        let fx = Fixture::new();

        let report = scan_with(
            &fx,
            "xcode.sim-unavailable",
            &FakeRunner::failing("no Xcode"),
        );

        assert_eq!(report.items, vec![]);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("no Xcode"));
        assert_eq!(report.skipped[0].path, None);
    }

    /// Makes one entry that `source` would find, with a file inside, and returns its path.
    fn plant(fx: &Fixture, source: &Source) -> PathBuf {
        let mut path = fx.home().join(source.root).join(source.sub);
        for _ in 1..source.depth {
            path.push("level");
        }
        path.push(match source.filter {
            Filter::Any | Filter::DirsOnly => "entry".to_string(),
            Filter::Prefix(prefix) => format!("{prefix}x"),
            Filter::Suffix(suffix) => format!("entry{suffix}"),
            Filter::Named(names) => names[0].to_string(),
        });
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("data.bin"), "cache").unwrap();
        path
    }

    #[test]
    fn every_source_in_the_catalog_finds_its_entry_and_the_guard_accepts_it() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(NO_SIMULATORS);
        let scanner = Scanner::new(&fx.env, &runner);

        for target in catalog::all_targets().filter(|t| t.kind != Kind::UnavailableSimulators) {
            let planted: Vec<_> = target.sources.iter().map(|s| plant(&fx, s)).collect();
            let report = scanner.scan_target(target);
            assert_eq!(report.skipped, vec![], "{}", target.id);
            assert_eq!(report.items.len(), planted.len(), "{}", target.id);
        }
    }

    #[test]
    fn no_item_contains_another_item() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(NO_SIMULATORS);
        for target in catalog::all_targets() {
            target.sources.iter().for_each(|s| {
                plant(&fx, s);
            });
        }

        let report = Scanner::new(&fx.env, &runner).scan_all(&mut |_| {});

        let all: Vec<&Path> = report
            .targets
            .iter()
            .flat_map(|t| &t.items)
            .flat_map(paths)
            .collect();
        assert!(all.len() > 10);
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert!(
                    i == j || !b.starts_with(a),
                    "{} contains {}",
                    a.display(),
                    b.display()
                );
            }
        }
    }

    #[test]
    fn scan_all_reports_every_target_in_catalog_order_and_announces_each() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(NO_SIMULATORS);
        let mut announced = Vec::new();

        let report = Scanner::new(&fx.env, &runner).scan_all(&mut |t| announced.push(t.id));

        let ids: Vec<_> = report.targets.iter().map(|t| t.target.id).collect();
        assert_eq!(
            ids,
            catalog::all_targets().map(|t| t.id).collect::<Vec<_>>()
        );
        assert_eq!(announced, ids);
        assert_eq!(report.total_bytes(), 0);
    }

    #[test]
    fn items_with_the_same_name_in_two_sources_have_different_keys() {
        let fx = Fixture::new();
        fx.dir("Library/Developer/Xcode/iOS DeviceSupport/Same 1.0");
        fx.dir("Library/Developer/Xcode/watchOS DeviceSupport/Same 1.0");

        let report = scan(&fx, "xcode.device-support");

        assert_eq!(names(&report), ["Same 1.0", "Same 1.0"]);
        assert_ne!(report.items[0].key(), report.items[1].key());
    }
}
