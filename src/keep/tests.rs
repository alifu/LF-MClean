use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;
use crate::catalog::Simulator;
use crate::scan::{Removal, Scanner};
use crate::testing::{FakeRunner, Fixture};

const NO_SIMULATORS: &str = r#"{"devices": {}}"#;
const PODS: &str = "Library/Caches/CocoaPods/Pods";
const DERIVED_DATA: &str = "Library/Developer/Xcode/DerivedData";

fn write_keep(fx: &Fixture, text: &str) -> PathBuf {
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}

fn loaded(fx: &Fixture, text: &str) -> KeepList {
    write_keep(fx, text);
    KeepList::load(&fx.env).unwrap()
}

fn items_of(fx: &Fixture, target: &'static Target) -> Vec<Item> {
    let runner = FakeRunner::answering(NO_SIMULATORS);
    Scanner::new(&fx.env, &runner).scan_target(target).items
}

fn items(fx: &Fixture, target: &str) -> Vec<Item> {
    items_of(fx, catalog::find(target).unwrap())
}

fn invalid_reason(fx: &Fixture, text: &str) -> String {
    write_keep(fx, text);
    match KeepList::load(&fx.env) {
        Err(KeepError::Invalid { reason, .. }) => reason,
        other => panic!("{text:?} should be invalid, got {other:?}"),
    }
}

fn pod(name: &str) -> KeptPackage {
    KeptPackage::all_versions(Manager::Pods, name).unwrap()
}

fn pod_version(name: &str, version: &str) -> KeptPackage {
    KeptPackage::version(Manager::Pods, name, version).unwrap()
}

fn spm(name: &str) -> KeptPackage {
    KeptPackage::all_versions(Manager::Spm, name).unwrap()
}

// Reading the file: only a file that is truly absent is an empty list.

#[test]
fn a_missing_file_is_an_empty_list() {
    let fx = Fixture::new();

    let keep = KeepList::load(&fx.env).unwrap();

    assert!(keep.is_empty());
    assert_eq!(keep, KeepList::empty(&fx.env));
}

#[test]
fn the_file_is_in_the_config_folder() {
    let fx = Fixture::new();

    assert_eq!(
        KeepList::file(&fx.env),
        fx.home().join(".config/lfmclean/keep.toml")
    );
}

#[test]
fn a_full_file_is_read() {
    let fx = Fixture::new();

    let keep = loaded(
        &fx,
        r#"
targets  = ["xcode.archives"]
packages = ["pods:GoogleMaps", "pods:Firebase@11.0.0", "spm:SnapKit"]
paths    = ["~/.android/avd/Pixel_8_API_35.avd", "/Volumes/Work/keep"]
"#,
    );

    assert_eq!(keep.targets().collect::<Vec<_>>(), ["xcode.archives"]);
    let packages: Vec<String> = keep.packages().map(ToString::to_string).collect();
    assert_eq!(
        packages,
        ["pods:Firebase@11.0.0", "pods:GoogleMaps", "spm:SnapKit"]
    );
    let paths: Vec<&Path> = keep.paths().collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&Path::new("/Volumes/Work/keep")));
    assert!(
        paths.contains(
            &fx.home()
                .canonicalize()
                .unwrap()
                .join(".android/avd/Pixel_8_API_35.avd")
                .as_path()
        )
    );
}

#[test]
fn an_empty_file_is_an_empty_list() {
    let fx = Fixture::new();

    assert!(loaded(&fx, "").is_empty());
}

#[test]
fn a_file_that_is_not_toml_stops_everything_and_says_where() {
    let fx = Fixture::new();
    let path = write_keep(&fx, "targets = [\"xcode.archives\"");

    let error = KeepList::load(&fx.env).unwrap_err();

    assert!(matches!(error, KeepError::Invalid { .. }));
    let message = error.to_string();
    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(message.contains("nothing is removed"), "{message}");
    assert!(
        message.contains("line 1"),
        "the line of the mistake is shown: {message}"
    );
}

#[test]
fn a_file_that_cannot_be_read_stops_everything() {
    let fx = Fixture::new();
    let path = write_keep(&fx, "targets = []");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let result = KeepList::load(&fx.env);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(result, Err(KeepError::Unreadable { .. })),
        "{result:?}"
    );
}

#[test]
fn a_folder_where_the_file_should_be_stops_everything() {
    let fx = Fixture::new();
    fs::create_dir_all(KeepList::file(&fx.env)).unwrap();

    assert!(matches!(
        KeepList::load(&fx.env),
        Err(KeepError::Unreadable { .. })
    ));
}

#[test]
fn a_keep_file_that_is_a_symlink_to_nothing_stops_everything() {
    let fx = Fixture::new();
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(fx.outside.join("dotfiles/keep.toml"), &path).unwrap();

    let error = KeepList::load(&fx.env).unwrap_err();

    assert!(matches!(error, KeepError::Unreadable { .. }), "{error:?}");
    assert!(error.to_string().contains("symlink"), "{error}");
}

#[test]
fn a_config_folder_that_is_a_symlink_to_nothing_stops_everything() {
    let fx = Fixture::new();
    symlink(fx.outside.join("gone"), fx.home().join(".config")).unwrap();

    assert!(matches!(
        KeepList::load(&fx.env),
        Err(KeepError::Unreadable { .. })
    ));
}

#[test]
fn a_keep_file_reached_through_a_working_symlink_is_read() {
    let fx = Fixture::new();
    fs::write(
        fx.outside.join("keep.toml"),
        "targets = [\"xcode.archives\"]",
    )
    .unwrap();
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(fx.outside.join("keep.toml"), &path).unwrap();

    let keep = KeepList::load(&fx.env).unwrap();

    assert_eq!(keep.targets().collect::<Vec<_>>(), ["xcode.archives"]);
}

#[test]
fn a_config_folder_that_is_a_working_symlink_without_the_file_is_empty() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.outside.join("config")).unwrap();
    symlink(fx.outside.join("config"), fx.home().join(".config")).unwrap();

    assert!(KeepList::load(&fx.env).unwrap().is_empty());
}

#[test]
fn a_pipe_where_the_file_should_be_stops_everything_without_waiting() {
    let fx = Fixture::new();
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );

    assert!(matches!(
        KeepList::load(&fx.env),
        Err(KeepError::Unreadable { .. })
    ));
}

#[test]
fn a_file_that_is_not_utf8_stops_everything() {
    let fx = Fixture::new();
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();

    assert!(KeepList::load(&fx.env).is_err());
}

#[test]
fn a_huge_file_stops_everything() {
    let fx = Fixture::new();
    let padding = format!(
        "# {}\n",
        "x".repeat(usize::try_from(MAX_FILE_BYTES).unwrap())
    );
    write_keep(&fx, &padding);

    assert!(matches!(
        KeepList::load(&fx.env),
        Err(KeepError::Unreadable { .. })
    ));
}

#[test]
fn unknown_keys_and_wrong_types_stop_everything() {
    let fx = Fixture::new();

    invalid_reason(&fx, "target = [\"xcode.archives\"]");
    invalid_reason(&fx, "targets = \"xcode.archives\"");
    invalid_reason(&fx, "packages = [1]");
}

#[test]
fn an_unknown_target_stops_everything() {
    let fx = Fixture::new();

    let reason = invalid_reason(&fx, "targets = [\"xcode.archive\"]");

    assert!(reason.contains("xcode.archive"), "{reason}");
}

#[test]
fn badly_written_packages_stop_everything() {
    let fx = Fixture::new();

    for bad in [
        "Firebase",
        "npm:left-pad",
        "pods:",
        "pods:Firebase@",
        "pods: Firebase",
        "pods:Fire base",
        "pods:a@b@c",
        "spm:a:b",
    ] {
        invalid_reason(&fx, &format!("packages = [{bad:?}]"));
    }
}

#[test]
fn relative_or_dotted_paths_stop_everything() {
    let fx = Fixture::new();

    for bad in ["Library/keep", "~/a/../b", "/a/./b", "~user/x", "/", ""] {
        invalid_reason(&fx, &format!("paths = [{bad:?}]"));
    }
}

// What a keep protects.

#[test]
fn a_kept_target_keeps_all_its_items() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/App-abc"));
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.derived-data")
        .unwrap();

    assert!(keep.keeps(&items(&fx, "xcode.derived-data")[0]));
}

#[test]
fn a_kept_target_inside_another_targets_item_keeps_that_item() {
    let fx = Fixture::new();
    fx.dir(&format!(
        "{DERIVED_DATA}/App-abc/SourcePackages/checkouts/SnapKit"
    ));
    fx.dir(&format!("{DERIVED_DATA}/Plain-def/Build"));
    let keep = KeepList::empty(&fx.env)
        .with_target("spm.source-packages")
        .unwrap();

    let found = items(&fx, "xcode.derived-data");

    let kept: Vec<_> = found
        .iter()
        .map(|i| (i.name.as_str(), keep.keeps(i)))
        .collect();
    assert_eq!(kept, [("App-abc", true), ("Plain-def", false)]);
}

#[test]
fn a_folder_that_cannot_be_read_might_hold_a_kept_target_so_it_is_kept() {
    let fx = Fixture::new();
    let locked = fx.dir(&format!("{DERIVED_DATA}/Locked-abc"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let keep = KeepList::empty(&fx.env)
        .with_target("spm.source-packages")
        .unwrap();

    let found = items(&fx, "xcode.derived-data");
    let kept = keep.keeps(&found[0]);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(kept);
}

#[test]
fn keeping_the_whole_home_folder_keeps_everything() {
    let fx = Fixture::new();
    fx.dir(".gradle/caches/modules-2");

    assert!(loaded(&fx, "paths = [\"~/\"]").keeps(&items(&fx, "gradle.caches")[0]));
}

#[test]
fn a_kept_target_keeps_the_items_of_targets_inside_it() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/App-abc/SourcePackages"));
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.derived-data")
        .unwrap();

    let checkouts = items_of(&fx, catalog::SOURCE_PACKAGES);

    assert!(keep.keeps(&checkouts[0]));
}

#[test]
fn a_kept_target_does_not_keep_unrelated_items() {
    let fx = Fixture::new();
    fx.dir(".gradle/caches/modules-2");
    fx.dir("Library/Caches/org.swift.swiftpm/manifests");
    fx.dir("Library/Caches/org.swift.swiftpm/repositories/SnapKit-a5691e5b");
    let keep = KeepList::empty(&fx.env)
        .with_target("spm.source-packages")
        .and_then(|k| k.with_target("spm.manifests"))
        .unwrap();

    assert!(!keep.keeps(&items(&fx, "gradle.caches")[0]));
    assert!(!keep.keeps(&items(&fx, "spm.repositories")[0]));
    assert!(keep.keeps(&items(&fx, "spm.manifests")[0]));
}

#[test]
fn a_kept_package_keeps_every_version_and_a_kept_version_only_itself() {
    let fx = Fixture::new();
    fx.dir(&format!("{PODS}/Release/Firebase/10.25.0-aaaaa"));
    fx.dir(&format!("{PODS}/Release/Firebase/11.0.0-bbbbb"));
    fx.dir(&format!("{PODS}/Release/Alamofire/5.8.0-ccccc"));
    let found = items(&fx, "pods.cache");
    let (alamofire, old, new) = (&found[0], &found[1], &found[2]);

    let one_version = KeepList::empty(&fx.env).with_package(pod_version("Firebase", "11.0.0"));
    let every_version = KeepList::empty(&fx.env).with_package(pod("firebase"));

    assert_eq!(
        [alamofire, old, new].map(|item| one_version.keeps(item)),
        [false, false, true]
    );
    assert_eq!(
        [alamofire, old, new].map(|item| every_version.keeps(item)),
        [false, true, true]
    );
}

#[test]
fn a_kept_version_keeps_copies_whose_version_is_unknown() {
    let fx = Fixture::new();
    fx.dir(&format!("{PODS}/External/MyFork/9f8e7d6c5b4a"));
    fx.dir(&format!("{PODS}/Release/Odd/1.0.0"));
    fx.dir("Library/Caches/org.swift.swiftpm/repositories/SnapKit-a5691e5b");
    let keep = KeepList::empty(&fx.env)
        .with_package(pod_version("MyFork", "1.0.0"))
        .with_package(pod_version("Odd", "2.0.0"))
        .with_package(KeptPackage::version(Manager::Spm, "SnapKit", "5.7.1").unwrap());

    let mut found = items(&fx, "pods.cache");
    found.extend(items(&fx, "spm.repositories"));

    assert_eq!(found.len(), 3);
    assert!(found.iter().all(|item| keep.keeps(item)));
}

#[test]
fn a_kept_swift_package_keeps_its_repository_but_not_a_pod_of_that_name() {
    let fx = Fixture::new();
    fx.dir("Library/Caches/org.swift.swiftpm/repositories/SnapKit-a5691e5b");

    let found = items(&fx, "spm.repositories");

    assert!(
        KeepList::empty(&fx.env)
            .with_package(spm("SnapKit"))
            .keeps(&found[0])
    );
    assert!(
        !KeepList::empty(&fx.env)
            .with_package(pod("SnapKit"))
            .keeps(&found[0])
    );
}

#[test]
fn a_kept_package_does_not_reach_into_project_checkouts() {
    // Documented: a package keep covers the shared caches; a project's own checkouts go with it.
    let fx = Fixture::new();
    fx.dir(&format!(
        "{DERIVED_DATA}/App-abc/SourcePackages/checkouts/SnapKit"
    ));
    let keep = KeepList::empty(&fx.env).with_package(spm("SnapKit"));

    assert!(!keep.keeps(&items_of(&fx, catalog::SOURCE_PACKAGES)[0]));
}

#[test]
fn a_kept_path_keeps_the_item_it_is_in_and_the_item_that_is_it() {
    let fx = Fixture::new();
    fx.dir(".android/avd/Pixel.avd");
    fs::write(
        fx.home().join(".android/avd/Pixel.ini"),
        "path.rel=avd/Pixel.avd\n",
    )
    .unwrap();
    fx.dir(".android/avd/Other.avd");
    let inside = loaded(&fx, "paths = [\"~/.android/avd/Pixel.avd/sdcard.img\"]");
    let exact = KeepList::empty(&fx.env)
        .with_path(
            &fx.home()
                .join(".android/avd/Pixel.avd")
                .display()
                .to_string(),
        )
        .unwrap();

    let found = items(&fx, "android.avd");

    let names: Vec<_> = found.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Other", "Pixel"]);
    assert!(!inside.keeps(&found[0]));
    assert!(
        inside.keeps(&found[1]),
        "removing the avd would remove the kept file"
    );
    assert!(
        exact.keeps(&found[1]),
        "the home folder is resolved like the guard's paths"
    );
}

#[test]
fn a_kept_folder_keeps_every_item_inside_it() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/App-abc"));
    let keep = loaded(&fx, "paths = [\"~/Library/Developer/Xcode\"]");

    assert!(keep.keeps(&items(&fx, "xcode.derived-data")[0]));
}

#[test]
fn a_kept_path_whose_folders_do_not_exist_yet_still_matches_later() {
    let fx = Fixture::new();
    let keep = loaded(
        &fx,
        "paths = [\"~/Library/Developer/Xcode/DerivedData/App-abc\"]",
    );
    fx.dir(&format!("{DERIVED_DATA}/App-abc"));

    assert!(keep.keeps(&items(&fx, "xcode.derived-data")[0]));
}

#[test]
fn paths_are_compared_without_regard_to_case() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/App-abc"));
    let keep = loaded(
        &fx,
        "paths = [\"~/library/developer/xcode/deriveddata/APP-ABC\"]",
    );

    assert!(keep.keeps(&items(&fx, "xcode.derived-data")[0]));
}

#[test]
fn paths_are_compared_without_regard_to_unicode_normalization() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/Caf\u{e9}-abc"));
    let keep = loaded(
        &fx,
        "paths = [\"~/Library/Developer/Xcode/DerivedData/Cafe\u{301}-abc\"]",
    );

    assert!(keep.keeps(&items(&fx, "xcode.derived-data")[0]));
}

#[test]
fn a_kept_entry_in_one_item_does_not_keep_its_neighbour() {
    let fx = Fixture::new();
    fx.dir(&format!("{DERIVED_DATA}/App"));
    fx.dir(&format!("{DERIVED_DATA}/App-abc"));
    let keep = loaded(
        &fx,
        "paths = [\"~/Library/Developer/Xcode/DerivedData/App\"]",
    );

    let found = items(&fx, "xcode.derived-data");

    assert_eq!(
        found.iter().map(|i| keep.keeps(i)).collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn simulators_are_kept_by_target_or_by_a_path_around_their_folders() {
    let fx = Fixture::new();
    let devices = fx
        .home()
        .canonicalize()
        .unwrap()
        .join("Library/Developer/CoreSimulator/Devices");
    let item = Item {
        target: "xcode.sim-unavailable",
        name: "Unavailable simulators (1)".to_string(),
        removal: Removal::SimctlDelete {
            simulators: vec![Simulator {
                udid: "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".to_string(),
                folder: devices.join("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"),
            }],
        },
        size_bytes: 0,
        partial: false,
        modified: None,
    };
    let path = |text: &str| KeepList::empty(&fx.env).with_path(text).unwrap();

    assert!(!KeepList::empty(&fx.env).keeps(&item));
    assert!(
        KeepList::empty(&fx.env)
            .with_target("xcode.sim-unavailable")
            .unwrap()
            .keeps(&item)
    );
    assert!(
        path("~/Library/Developer/CoreSimulator/Devices/AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA")
            .keeps(&item)
    );
    assert!(path("~/Library/Developer").keeps(&item));
    assert!(
        !path("~/Library/Developer/CoreSimulator/Devices/BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB")
            .keeps(&item)
    );
}

// Changing and saving the list.

#[test]
fn saving_and_loading_gives_the_same_list() {
    let fx = Fixture::new();
    fx.dir(".android/avd");
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .with_package(pod("GoogleMaps"))
        .with_package(pod_version("Firebase", "11.0.0"))
        .with_package(spm("SnapKit"))
        .with_path("~/.android/avd/Pixel.avd")
        .and_then(|k| k.with_path("/Volumes/Work/keep"))
        .unwrap();

    keep.save(&fx.env).unwrap();
    let path = KeepList::file(&fx.env);
    let text = fs::read_to_string(&path).unwrap();
    let loaded = KeepList::load(&fx.env).unwrap();

    assert_eq!(loaded, keep);
    assert!(text.contains("\"~/.android/avd/Pixel.avd\""), "{text}");
    assert!(text.contains("\"pods:Firebase@11.0.0\""), "{text}");
    let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["keep.toml"]);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        FILE_MODE
    );
}

#[test]
fn saving_through_a_symlinked_keep_file_replaces_its_target_and_keeps_the_link() {
    let fx = Fixture::new();
    let real = fx.outside.join("keep.toml");
    fs::write(&real, "").unwrap();
    let path = KeepList::file(&fx.env);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(&real, &path).unwrap();

    KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .save(&fx.env)
        .unwrap();

    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&real)
            .unwrap()
            .contains("xcode.archives")
    );
}

#[test]
fn saving_into_an_unwritable_folder_is_an_error() {
    let fx = Fixture::new();
    let folder = fx.home().join(".config");
    fs::create_dir_all(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();

    let result = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .save(&fx.env);

    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(result, Err(KeepError::NotSaved { .. })),
        "{result:?}"
    );
}

#[test]
fn values_that_could_not_be_read_back_are_refused_before_saving() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env);

    assert!(keep.with_target("xcode.archive").is_err());
    assert!(keep.with_path("/").is_err());
    assert!(keep.with_path("relative").is_err());
    assert!(keep.with_path("~/a/../b").is_err());
    assert!(KeptPackage::all_versions(Manager::Pods, "").is_err());
    assert!(KeptPackage::all_versions(Manager::Pods, "a@b").is_err());
    assert!(KeptPackage::version(Manager::Spm, "SnapKit", "").is_err());
    assert!(KeptPackage::version(Manager::Spm, "SnapKit", "1 0").is_err());
}

#[test]
fn a_saved_list_always_loads_again() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_package(pod("Name-With.Dots_and+plus"))
        .with_path("/Volumes/Some Disk/x y")
        .unwrap();

    keep.save(&fx.env).unwrap();

    assert_eq!(KeepList::load(&fx.env).unwrap(), keep);
}

#[test]
fn changes_return_a_new_list_and_leave_the_old_one() {
    let fx = Fixture::new();
    let empty = KeepList::empty(&fx.env);

    let with = empty
        .with_package(pod("Firebase"))
        .with_target("xcode.archives")
        .unwrap();
    let without = with
        .without_package(&pod("FIREBASE"))
        .without_target("xcode.archives");

    assert!(empty.is_empty());
    assert_eq!(with.packages().count(), 1);
    assert_eq!(with.targets().count(), 1);
    assert!(without.is_empty());
}

#[test]
fn keeping_one_version_and_every_version_are_separate_entries() {
    let fx = Fixture::new();
    let both = KeepList::empty(&fx.env)
        .with_package(pod("Firebase"))
        .with_package(pod_version("Firebase", "11.0.0"));

    assert_eq!(both.packages().count(), 2);
    assert_eq!(
        both.without_package(&pod("Firebase"))
            .packages()
            .collect::<Vec<_>>(),
        [&pod_version("Firebase", "11.0.0")]
    );
}

#[test]
fn adding_the_same_entry_twice_keeps_one() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_package(spm("SnapKit"))
        .with_package(spm("snapkit"))
        .with_path("/A/b")
        .and_then(|k| k.with_path("/a/B"))
        .unwrap();

    assert_eq!(keep.packages().count(), 1);
    assert_eq!(keep.paths().count(), 1);
}

#[test]
fn a_purge_target_cannot_be_kept_by_id() {
    let fx = Fixture::new();

    let refused = KeepList::empty(&fx.env).with_target("purge.build");

    assert!(refused.unwrap_err().contains("keep the project folder"));
}
