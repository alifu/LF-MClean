use std::fs;
use std::path::PathBuf;

use super::*;
use crate::scan::Removal;
use crate::testing::{FakeRunner, Fixture};

const PODS: &str = "Library/Caches/CocoaPods/Pods";
const REPOS: &str = "Library/Caches/org.swift.swiftpm/repositories";
const NO_SIMULATORS: &str = r#"{"devices": {}}"#;

fn cache(fx: &Fixture, rel: &str, bytes: usize) -> PathBuf {
    let dir = fx.home().join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("data.bin"), vec![1_u8; bytes]).unwrap();
    dir
}

fn lock(fx: &Fixture, rel: &str, text: &str) {
    let path = fx.home().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn browse_home(fx: &Fixture) -> Browse {
    let runner = FakeRunner::answering(NO_SIMULATORS);
    browse(&Scanner::new(&fx.env, &runner), &find_usage(&fx.env))
}

fn labels(package: &Package) -> Vec<&str> {
    package.versions.iter().map(|v| v.label.as_str()).collect()
}

#[test]
fn pod_versions_are_grouped_under_one_package_oldest_first() {
    let fx = Fixture::new();
    cache(
        &fx,
        &format!("{PODS}/Release/Firebase/10.25.0-aaaaa"),
        20_000,
    );
    cache(&fx, &format!("{PODS}/Release/Firebase/9.6.0-bbbbb"), 10_000);
    cache(
        &fx,
        &format!("{PODS}/Release/Firebase/11.0.0-ccccc"),
        30_000,
    );
    cache(&fx, &format!("{PODS}/Release/Alamofire/5.8.0-ddddd"), 5_000);

    let found = browse_home(&fx);

    let names: Vec<_> = found.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Alamofire", "Firebase"]);
    let firebase = &found.packages[1];
    assert_eq!(firebase.manager, Manager::Pods);
    assert_eq!(labels(firebase), ["9.6.0", "10.25.0", "11.0.0"]);
    assert_eq!(
        firebase.total_bytes(),
        firebase
            .versions
            .iter()
            .map(|v| v.item.size_bytes)
            .sum::<u64>()
    );
    assert!(firebase.total_bytes() >= 60_000);
    assert_eq!(found.skipped, vec![]);
}

#[test]
fn used_by_and_in_use_come_from_the_lock_files() {
    let fx = Fixture::new();
    cache(&fx, &format!("{PODS}/Release/Firebase/10.20.0-aaaaa"), 100);
    cache(&fx, &format!("{PODS}/Release/Firebase/10.25.0-bbbbb"), 100);
    cache(&fx, &format!("{PODS}/Release/Alamofire/5.8.0-ccccc"), 100);
    lock(
        &fx,
        "Documents/MyCMPApp/iosApp/Podfile.lock",
        "PODS:\n  - Firebase/Core (10.25.0)\n",
    );

    let found = browse_home(&fx);

    let (alamofire, firebase) = (&found.packages[0], &found.packages[1]);
    assert_eq!(alamofire.used_by, Vec::<String>::new());
    assert!(!alamofire.versions[0].in_use);
    assert_eq!(firebase.used_by, ["iosApp"]);
    assert!(
        !firebase.versions[0].in_use,
        "10.20.0 is not in any lock file"
    );
    assert!(firebase.versions[1].in_use, "10.25.0 is");
}

#[test]
fn external_pods_and_swift_repositories_are_single_copies() {
    let fx = Fixture::new();
    cache(&fx, &format!("{PODS}/External/MyFork/9f8e7d6c5b4a"), 100);
    cache(&fx, &format!("{REPOS}/SnapKit-a5691e5b"), 100);
    lock(
        &fx,
        "Developer/CatWalk/Package.resolved",
        r#"{"pins": [{"identity": "snapkit", "location": "https://github.com/SnapKit/SnapKit.git",
            "state": {"version": "5.7.1"}}], "version": 2}"#,
    );

    let found = browse_home(&fx);

    let (fork, snapkit) = (&found.packages[0], &found.packages[1]);
    assert_eq!(
        (fork.manager, labels(fork)),
        (Manager::Pods, vec!["external"])
    );
    assert_eq!(
        (snapkit.manager, labels(snapkit)),
        (Manager::Spm, vec!["repository"])
    );
    assert_eq!(snapkit.used_by, ["CatWalk"]);
    assert!(snapkit.versions[0].in_use);
}

#[test]
fn project_checkouts_are_listed_apart_and_removed_by_path() {
    let fx = Fixture::new();
    let checkouts = cache(
        &fx,
        "Library/Developer/Xcode/DerivedData/CatWalk-facwgxvjllxdxihkfbenhhwheeai/SourcePackages",
        100,
    );
    cache(
        &fx,
        "Library/Developer/Xcode/DerivedData/CatWalk-facwgxvjllxdxihkfbenhhwheeai/Build",
        100,
    );
    cache(
        &fx,
        "Library/Developer/Xcode/DerivedData/Odd-name/SourcePackages",
        100,
    );

    let found = browse_home(&fx);

    assert!(found.packages.is_empty());
    let names: Vec<_> = found
        .project_checkouts
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(names, ["CatWalk", "Odd-name"]);
    let item = &found.project_checkouts[0];
    assert_eq!(item.target, "spm.source-packages");
    let Removal::Paths { main, with } = &item.removal else {
        panic!("checkouts are removed by path");
    };
    assert_eq!(main.path(), checkouts.canonicalize().unwrap());
    assert_eq!(with.len(), 0);
}

#[test]
fn identify_only_knows_package_cache_items() {
    let fx = Fixture::new();
    cache(&fx, ".gradle/caches/modules-2", 100);
    let runner = FakeRunner::answering(NO_SIMULATORS);
    let report =
        Scanner::new(&fx.env, &runner).scan_target(catalog::find("gradle.caches").unwrap());

    assert_eq!(identify(&report.items[0]), None);
}

#[test]
fn the_package_targets_are_in_the_catalog() {
    for id in [PODS_TARGET, SPM_TARGET] {
        assert!(catalog::find(id).is_some(), "{id}");
    }
}

#[test]
fn versions_compare_as_numbers() {
    let mut versions = vec!["10.0.0", "9.10.0", "9.9.1", "9.9", "1.0.0-beta", "1.0.0"];

    versions.sort_by(|a, b| compare_versions(a, b));

    assert_eq!(
        versions,
        ["1.0.0", "1.0.0-beta", "9.9", "9.9.1", "9.10.0", "10.0.0"]
    );
}

#[test]
fn manager_keys_go_both_ways() {
    for manager in [Manager::Pods, Manager::Spm] {
        assert_eq!(Manager::from_key(manager.key()), Some(manager));
    }
    assert_eq!(Manager::from_key("npm"), None);
}
