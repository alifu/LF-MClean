//! Turning a scan or the package browser into list sections, marking what the keep list keeps.

use chrono::{DateTime, FixedOffset};

use super::list::{Entry, Group, KeepRef, Section};
use crate::catalog::{self, Safety};
use crate::format::{age, printable};
use crate::keep::{KeepList, KeptPackage};
use crate::packages::{Browse, Manager, Package, Usage, Version};
use crate::scan::{Item, Removal, Report};

const CHECKOUTS_KEY: &str = "checkouts";

/// One section per category, one group per target that has items.
#[must_use]
pub fn clean_sections(
    report: &Report,
    keep: &KeepList,
    now: DateTime<FixedOffset>,
) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for target in report.targets.iter().filter(|t| !t.items.is_empty()) {
        let count = target.items.len();
        let group = Group {
            key: target.target.id.to_string(),
            label: format!("{} ({count})", target.target.name),
            detail: String::new(),
            safety: target.target.safety,
            keep_ref: KeepRef::Target(target.target.id),
            entries: target
                .items
                .iter()
                .map(|item| entry(item, printable(&item.name), aged(item, now), keep))
                .collect(),
            unused: false,
        };
        let title = target.target.category.title();
        match sections.last_mut() {
            Some(section) if section.title == title => section.groups.push(group),
            _ => sections.push(Section {
                title: title.to_string(),
                groups: vec![group],
            }),
        }
    }
    sections
}

/// `CocoaPods`, Swift packages, then the package checkouts of each Xcode project.
#[must_use]
pub fn package_sections(
    browse: &Browse,
    usage: &Usage,
    keep: &KeepList,
    now: DateTime<FixedOffset>,
) -> Vec<Section> {
    let mut sections = Vec::new();
    for (manager, title) in [
        (Manager::Pods, "CocoaPods"),
        (Manager::Spm, "Swift packages"),
    ] {
        let groups: Vec<Group> = browse
            .packages
            .iter()
            .filter(|p| p.manager == manager)
            .map(|p| package_group(p, usage, keep, now))
            .collect();
        if !groups.is_empty() {
            sections.push(Section {
                title: title.to_string(),
                groups,
            });
        }
    }
    if !browse.project_checkouts.is_empty() {
        let count = browse.project_checkouts.len();
        sections.push(Section {
            title: "Project checkouts".to_string(),
            groups: vec![Group {
                key: CHECKOUTS_KEY.to_string(),
                label: format!("Swift packages of Xcode projects ({count})"),
                detail: "inside DerivedData".to_string(),
                safety: catalog::SOURCE_PACKAGES.safety,
                keep_ref: KeepRef::Target(catalog::SOURCE_PACKAGES.id),
                entries: browse
                    .project_checkouts
                    .iter()
                    .map(|item| entry(item, printable(&item.name), aged(item, now), keep))
                    .collect(),
                unused: false,
            }],
        });
    }
    sections
}

fn package_group(
    package: &Package,
    usage: &Usage,
    keep: &KeepList,
    now: DateTime<FixedOffset>,
) -> Group {
    let count = package.versions.len();
    let copies = if count == 1 {
        "1 copy".to_string()
    } else {
        format!("{count} copies")
    };
    let users = if package.used_by.is_empty() {
        if usage.is_complete() {
            "not used by any project".to_string()
        } else {
            "no project found using it".to_string()
        }
    } else {
        format!("used by: {}", printable(&package.used_by.join(", ")))
    };
    let name = printable(&package.name);
    let all_versions = KeptPackage::all_versions(package.manager, &package.name).ok();
    let first_item = &package.versions[0].item;
    Group {
        key: format!("{}:{}", package.manager.key(), package.name.to_lowercase()),
        label: name,
        detail: format!("{copies} · {users}"),
        safety: Safety::Safe,
        keep_ref: all_versions.map_or_else(|| path_ref(first_item), KeepRef::Package),
        entries: package
            .versions
            .iter()
            .map(|version| version_entry(package, version, keep, now))
            .collect(),
        unused: package.used_by.is_empty(),
    }
}

fn version_entry(
    package: &Package,
    version: &Version,
    keep: &KeepList,
    now: DateTime<FixedOffset>,
) -> Entry {
    let detail = if version.in_use {
        "in use".to_string()
    } else {
        aged(&version.item, now)
    };
    let keep_ref = crate::packages::identify(&version.item)
        .and_then(|id| match id.version {
            Some(v) => KeptPackage::version(package.manager, &package.name, &v).ok(),
            None => KeptPackage::all_versions(package.manager, &package.name).ok(),
        })
        .map_or_else(|| path_ref(&version.item), KeepRef::Package);
    Entry {
        kept: keep.keeps(&version.item),
        in_use: version.in_use,
        item: version.item.clone(),
        label: printable(&version.label),
        detail,
        keep_ref,
    }
}

fn entry(item: &Item, label: String, detail: String, keep: &KeepList) -> Entry {
    Entry {
        kept: keep.keeps(item),
        in_use: false,
        keep_ref: path_ref(item),
        item: item.clone(),
        label,
        detail,
    }
}

/// An item is kept by its main folder; simulators, which have no single folder, by their target.
fn path_ref(item: &Item) -> KeepRef {
    match &item.removal {
        Removal::Paths { main, .. } => KeepRef::Path(main.path().to_path_buf()),
        Removal::SimctlDelete { .. } => KeepRef::Target(item.target),
    }
}

fn aged(item: &Item, now: DateTime<FixedOffset>) -> String {
    item.modified
        .map_or_else(String::new, |time| age(time, now))
}
