//! CocoaPods and Swift package caches, grouped package by package, with the projects that use each
//! one. Every version is a scan `Item`, so removing one goes through the same guard checks as any
//! other cache.

mod pods;
mod spm;
mod usage;

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::Serialize;

pub use usage::{DEFAULT_PROJECT_FOLDERS, Usage, Use, find_usage};

use crate::catalog;
use crate::scan::{Item, Scanner, Skipped};

const PODS_TARGET: &str = "pods.cache";
const SPM_TARGET: &str = "spm.repositories";
/// Xcode names a `DerivedData` folder `<Project>-<28 letters>`.
const DERIVED_DATA_HASH_LEN: usize = 28;

/// The package manager a package belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Manager {
    /// `CocoaPods`.
    Pods,
    /// Swift Package Manager.
    Spm,
}

impl Manager {
    /// The word used in the keep list: `pods` or `spm`.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Pods => "pods",
            Self::Spm => "spm",
        }
    }

    /// The manager written as `key()`.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "pods" => Some(Self::Pods),
            "spm" => Some(Self::Spm),
            _ => None,
        }
    }
}

/// Which package, and which version of it, a cache item holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageId {
    /// Its package manager.
    pub manager: Manager,
    /// The package name as the cache folder writes it.
    pub name: String,
    /// The version, when the cache keeps versions apart.
    pub version: Option<String>,
}

/// The package an item of the `CocoaPods` or `SwiftPM` cache holds; `None` for other items.
#[must_use]
pub fn identify(item: &Item) -> Option<PackageId> {
    let paths = item.removal.paths();
    let main = paths.first()?.path();
    match item.target {
        PODS_TARGET => pods::identify(main),
        SPM_TARGET => spm::identify(main),
        _ => None,
    }
}

/// One cached copy of a package.
#[derive(Debug, Clone)]
pub struct Version {
    /// The version, or what kind of copy it is (`external`, `repository`).
    pub label: String,
    /// The cache entry, as the scan found it.
    pub item: Item,
    /// True when a project's lock file names this copy.
    pub in_use: bool,
}

/// A package with every cached copy of it.
#[derive(Debug, Clone)]
pub struct Package {
    /// Its package manager.
    pub manager: Manager,
    /// Its name.
    pub name: String,
    /// The cached copies, oldest version first.
    pub versions: Vec<Version>,
    /// The projects whose lock files name it, sorted.
    pub used_by: Vec<String>,
}

impl Package {
    /// Disk space of every copy.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.versions.iter().map(|v| v.item.size_bytes).sum()
    }
}

/// Everything the package browser shows.
#[derive(Debug)]
pub struct Browse {
    /// `CocoaPods` packages, then Swift packages, each sorted by name.
    pub packages: Vec<Package>,
    /// The package checkouts of each Xcode project in `DerivedData`.
    pub project_checkouts: Vec<Item>,
    /// What was found but is not offered.
    pub skipped: Vec<Skipped>,
}

/// Scans the package caches and groups them.
#[must_use]
pub fn browse(scanner: &Scanner<'_>, usage: &Usage) -> Browse {
    let mut skipped = Vec::new();
    let mut items = Vec::new();
    for target in [PODS_TARGET, SPM_TARGET]
        .into_iter()
        .filter_map(catalog::find)
    {
        let report = scanner.scan_target(target);
        items.extend(report.items);
        skipped.extend(report.skipped);
    }
    let checkouts = scanner.scan_target(catalog::SOURCE_PACKAGES);
    skipped.extend(checkouts.skipped);
    Browse {
        packages: group(items, usage),
        project_checkouts: checkouts
            .items
            .into_iter()
            .map(|item| Item {
                name: project_of_checkout(&item.name).to_string(),
                ..item
            })
            .collect(),
        skipped,
    }
}

/// Groups cache items by manager and package name (ignoring case), versions oldest first.
#[must_use]
pub fn group(items: Vec<Item>, usage: &Usage) -> Vec<Package> {
    let mut groups: BTreeMap<(Manager, String), Package> = BTreeMap::new();
    for item in items {
        let Some(id) = identify(&item) else { continue };
        let uses = usage.of(id.manager, &id.name);
        let in_use = match &id.version {
            Some(version) => uses
                .iter()
                .any(|u| u.version.as_deref() == Some(version.as_str())),
            None => !uses.is_empty(),
        };
        let label = id.version.clone().unwrap_or_else(|| match id.manager {
            Manager::Pods => item.removal.folders().first().map_or_else(
                || "external".to_string(),
                |main| pods::unversioned_label(main),
            ),
            Manager::Spm => "repository".to_string(),
        });
        let package = groups
            .entry((id.manager, id.name.to_lowercase()))
            .or_insert_with(|| Package {
                manager: id.manager,
                name: id.name.clone(),
                versions: Vec::new(),
                used_by: projects(uses),
            });
        package.versions.push(Version {
            label,
            item,
            in_use,
        });
    }
    groups
        .into_values()
        .map(|mut package| {
            package
                .versions
                .sort_by(|a, b| compare_versions(&a.label, &b.label));
            package
        })
        .collect()
}

/// `CatWalk-facwgxvjllxdxihkfbenhhwheeai/SourcePackages` is the checkout of `CatWalk`.
fn project_of_checkout(name: &str) -> &str {
    let folder = name.split('/').next().unwrap_or(name);
    match folder.rsplit_once('-') {
        Some((project, hash))
            if !project.is_empty()
                && hash.len() == DERIVED_DATA_HASH_LEN
                && hash.chars().all(|c| c.is_ascii_lowercase()) =>
        {
            project
        }
        _ => folder,
    }
}

fn projects(uses: &[Use]) -> Vec<String> {
    let mut names: Vec<String> = uses.iter().map(|u| u.project.clone()).collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// Orders versions as people read them: `9.0` before `10.0`, numbers compared as numbers.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_ascii_alphanumeric())
            .map(str::to_string)
            .collect()
    };
    let (a_parts, b_parts) = (parts(a), parts(b));
    for (x, y) in a_parts.iter().zip(&b_parts) {
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Greater,
            (Err(_), Ok(_)) => Ordering::Less,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a_parts.len().cmp(&b_parts.len())
}

#[cfg(test)]
mod tests;
