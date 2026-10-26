//! `lfmclean scan` run as a real program, against a fake home folder and a fake `xcrun`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const NO_SIMULATORS: &str = r#"{"devices": {}}"#;
const EXECUTABLE: u32 = 0o755;

struct Sandbox {
    _tmp: TempDir,
    home: PathBuf,
    bin: PathBuf,
}

impl Sandbox {
    /// A home folder, and a `bin` folder whose `xcrun` prints `simctl_json` (or fails if `None`).
    fn new(simctl_json: Option<&str>) -> Self {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let script = match simctl_json {
            Some(json) => format!("#!/bin/sh\necho '{json}'\n"),
            None => "#!/bin/sh\necho 'xcrun: error: no developer tools' >&2\nexit 1\n".to_string(),
        };
        let xcrun = bin.join("xcrun");
        fs::write(&xcrun, script).unwrap();
        fs::set_permissions(&xcrun, fs::Permissions::from_mode(EXECUTABLE)).unwrap();
        Self {
            _tmp: tmp,
            home,
            bin,
        }
    }

    fn plant(&self, rel: &str) -> PathBuf {
        let dir = self.home.join(rel);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("data.bin"), vec![1_u8; 50_000]).unwrap();
        dir
    }

    fn lfmclean(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("lfmclean").unwrap();
        command
            .args(args)
            .env("HOME", &self.home)
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin.display()));
        command
    }
}

fn stdout_of(command: &mut Command) -> String {
    let output = command.assert().success().get_output().stdout.clone();
    String::from_utf8(output).unwrap()
}

fn target<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["id"] == id)
        .unwrap_or_else(|| panic!("no target {id}"))
}

#[test]
fn json_lists_what_was_found() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    sb.plant("Library/Developer/Xcode/DerivedData/App-abc");
    sb.plant(".gradle/caches/modules-2");

    let report: Value =
        serde_json::from_str(&stdout_of(&mut sb.lfmclean(&["scan", "--json"]))).unwrap();

    let derived = target(&report, "xcode.derived-data");
    assert_eq!(derived["items"][0]["name"], "App-abc");
    assert_eq!(derived["items"][0]["removal"], "paths");
    assert!(derived["items"][0]["size_bytes"].as_u64().unwrap() >= 50_000);
    assert_eq!(
        target(&report, "gradle.caches")["items"][0]["name"],
        "modules-2"
    );
    assert!(report["total_bytes"].as_u64().unwrap() >= 100_000);
}

#[test]
fn the_table_shows_each_cache_and_the_total() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    sb.plant("Library/Developer/Xcode/DerivedData/App-abc");

    let text = stdout_of(&mut sb.lfmclean(&["scan"]));

    assert!(text.starts_with("Xcode\n  DerivedData"), "{text}");
    assert!(text.contains("safe     1 item"), "{text}");
    assert!(text.contains("\nTotal: "), "{text}");
    assert!(
        !text.contains("App-abc"),
        "items are only listed with --items"
    );
}

#[test]
fn items_lists_each_item() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    sb.plant("Library/Developer/Xcode/DerivedData/App-abc");

    let text = stdout_of(&mut sb.lfmclean(&["scan", "--items"]));

    assert!(text.contains("    App-abc"), "{text}");
}

#[test]
fn an_empty_home_finds_nothing() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    assert_eq!(
        stdout_of(&mut sb.lfmclean(&["scan"])),
        "Nothing found to clean.\n"
    );
}

#[test]
fn scanning_never_removes_anything() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let item = sb.plant("Library/Developer/Xcode/DerivedData/App-abc");

    stdout_of(&mut sb.lfmclean(&["scan", "--items"]));

    assert!(item.join("data.bin").is_file());
    assert!(
        !sb.home.join("Library/Logs/lfmclean").exists(),
        "a scan writes no log"
    );
}

#[test]
fn a_broken_xcrun_is_reported_but_the_scan_still_works() {
    let sb = Sandbox::new(None);
    sb.plant(".gradle/caches/modules-2");

    let text = stdout_of(&mut sb.lfmclean(&["scan"]));

    assert!(text.contains("Gradle caches"), "{text}");
    assert!(text.contains("Not offered (1):"), "{text}");
    assert!(
        text.contains("Unavailable simulators: simctl could not be run"),
        "{text}"
    );
}

#[test]
fn a_home_that_is_not_a_folder_is_an_error() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    sb.lfmclean(&["scan"])
        .env("HOME", Path::new("/nonexistent/lfmclean"))
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cannot be your home folder"));
}

#[test]
fn json_and_items_cannot_be_combined() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    sb.lfmclean(&["scan", "--json", "--items"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn without_a_command_the_help_is_shown() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let text = stdout_of(&mut sb.lfmclean(&[]));

    assert!(text.contains("Usage: lfmclean"), "{text}");
    assert!(text.contains("scan"), "{text}");
}

#[test]
fn the_version_is_the_crate_version() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let text = stdout_of(&mut sb.lfmclean(&["--version"]));

    assert_eq!(text.trim(), concat!("lfmclean ", env!("CARGO_PKG_VERSION")));
}
