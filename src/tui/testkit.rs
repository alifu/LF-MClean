//! Made-up scans and apps for the screen tests: fixed sizes, ages and paths, so drawings match.

use chrono::{DateTime, Duration, FixedOffset, TimeZone};

use super::app::{App, Event, Key};
use crate::catalog::{self, Simulator};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::packages::{Browse, Manager, Package, Usage, Version};
use crate::purge::{Found, Project};
use crate::scan::{Item, Removal, Report, TargetReport};
use crate::testing::Fixture;

pub const GB: u64 = 1_000_000_000;
pub const MB: u64 = 1_000_000;

pub fn now() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(7 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 10, 26, 12, 0, 0)
        .unwrap()
}

/// An item that needs no real folder: it is "removed" by id, under a made-up folder.
pub fn item(target: &'static str, name: &str, bytes: u64, days_old: i64) -> Item {
    Item {
        target,
        name: name.to_string(),
        removal: Removal::SimctlDelete {
            simulators: vec![Simulator {
                udid: format!("{target}-{name}"),
                folder: format!("/Users/me/{target}/{name}").into(),
            }],
        },
        size_bytes: bytes,
        partial: false,
        modified: Some(now() - Duration::days(days_old)),
    }
}

pub fn report() -> Report {
    let target = |id: &str, items: Vec<Item>| TargetReport {
        target: catalog::find(id).unwrap(),
        items,
        skipped: Vec::new(),
    };
    Report {
        targets: vec![
            target(
                "xcode.derived-data",
                vec![
                    item("xcode.derived-data", "App-abc", 1_500 * MB, 3),
                    item("xcode.derived-data", "Lib-def", 400 * MB, 210),
                ],
            ),
            target(
                "xcode.archives",
                vec![item(
                    "xcode.archives",
                    "2026-01-02/App.xcarchive",
                    300 * MB,
                    90,
                )],
            ),
            target(
                "android.avd",
                vec![item("android.avd", "Pixel_8", 8 * GB, 14)],
            ),
            target(
                "gradle.caches",
                vec![item("gradle.caches", "modules-2", 2 * GB, 1)],
            ),
            target(
                "konan.toolchains",
                vec![item("konan.toolchains", "2.0.0", 1_300 * MB, 40)],
            ),
        ],
    }
}

pub fn browse() -> Browse {
    let version = |name: &str, label: &str, bytes: u64, in_use: bool| Version {
        label: label.to_string(),
        item: item("pods.cache", &format!("{name}/{label}"), bytes, 30),
        in_use,
    };
    Browse {
        packages: vec![
            Package {
                manager: Manager::Pods,
                name: "Alamofire".to_string(),
                versions: vec![version("Alamofire", "5.8.0", 12 * MB, false)],
                used_by: Vec::new(),
            },
            Package {
                manager: Manager::Pods,
                name: "Firebase".to_string(),
                versions: vec![
                    version("Firebase", "10.20.0", 200 * MB, false),
                    version("Firebase", "10.25.0", 205 * MB, true),
                    version("Firebase", "11.0.0", 205 * MB, false),
                ],
                used_by: vec!["MyCMPApp".to_string()],
            },
            Package {
                manager: Manager::Spm,
                name: "SnapKit".to_string(),
                versions: vec![Version {
                    label: "repository".to_string(),
                    item: item("spm.repositories", "SnapKit-a5691e5b", 9 * MB, 5),
                    in_use: true,
                }],
                used_by: vec!["CatWalk".to_string()],
            },
        ],
        project_checkouts: vec![item("spm.source-packages", "CatWalk", 96 * MB, 2)],
        skipped: Vec::new(),
    }
}

pub fn app(fx: &Fixture, mode: Mode) -> App {
    App::new(
        mode,
        &fx.env,
        Ok(KeepList::empty(&fx.env)),
        Some(41_200 * MB),
        now(),
    )
}

/// The app on the Clean screen with the made-up scan in.
pub fn on_clean(fx: &Fixture) -> App {
    let (app, _) = app(fx, Mode::Trash).on_key(Key::Char('1'));
    app.on_event(Event::Scanned(app.scan_generation, report()))
        .0
}

/// The app on the Packages screen with the made-up packages in.
pub fn on_packages(fx: &Fixture, usage: Usage) -> App {
    let (app, _) = app(fx, Mode::Trash).on_key(Key::Char('2'));
    app.on_event(Event::PackagesScanned(
        app.packages_generation,
        browse(),
        usage,
    ))
    .0
}

/// Presses keys in order, returning the app and every effect asked for.
pub fn press(app: &App, keys: &[Key]) -> (App, Vec<super::app::Effect>) {
    keys.iter()
        .fold((app.clone(), Vec::new()), |(app, mut effects), key| {
            let (next, more) = app.on_key(*key);
            effects.extend(more);
            (next, effects)
        })
}

/// Two made-up projects: a Kotlin Multiplatform app with three folders and a small library.
pub fn found_projects() -> Found {
    let project = |name: &str, days: i64, items: Vec<Item>| Project {
        name: name.to_string(),
        folder: format!("/Users/me/Projects/{name}").into(),
        changed: Some(now() - Duration::days(days)),
        items,
    };
    Found {
        projects: vec![
            project(
                "Library",
                200,
                vec![item("purge.build", "build", 50 * MB, 200)],
            ),
            project(
                "ShopApp",
                3,
                vec![
                    item("purge.build", "composeApp/build", 900 * MB, 3),
                    item("purge.gradle", ".gradle", 120 * MB, 3),
                    item("purge.pods", "iosApp/Pods", 300 * MB, 30),
                ],
            ),
        ],
        problems: Vec::new(),
    }
}

/// The app on the Purge screen with the made-up projects in.
pub fn on_purge(fx: &Fixture) -> App {
    let (app, _) = app(fx, Mode::Trash).on_key(Key::Char('3'));
    app.on_event(Event::PurgeScanned(app.purge.generation, found_projects()))
        .0
}
