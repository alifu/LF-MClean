//! `lfmclean clean --ids`, `history` and the screen commands run as a real program, against a fake
//! home folder and a fake `xcrun`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use assert_cmd::Command;
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

    /// The same sandbox with a `xcrun` that prints `json`.
    fn with_simctl(old: Self, json: &str) -> Self {
        let xcrun = old.bin.join("xcrun");
        fs::write(&xcrun, format!("#!/bin/sh\necho '{json}'\n")).unwrap();
        old
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

fn run(command: &mut Command) -> std::process::Output {
    command.output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn without_yes_nothing_is_removed_and_the_plan_is_shown() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");

    let output = run(&mut sb.lfmclean(&["clean", "--ids", "gradle.caches"]));

    assert!(!output.status.success());
    assert!(text(&output.stdout).contains("modules-2"));
    assert!(text(&output.stderr).contains("--yes"));
    assert!(folder.exists());
}

#[test]
fn yes_with_permanent_removes_and_logs() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");

    let output =
        run(&mut sb.lfmclean(&["--permanent", "clean", "--ids", "gradle.caches", "--yes"]));

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("removed modules-2"));
    assert!(!folder.exists());
    let history = text(&run(&mut sb.lfmclean(&["history"])).stdout);
    assert!(history.contains("permanent"), "{history}");
    assert!(history.contains("modules-2"), "{history}");
}

#[test]
fn the_default_mode_moves_to_the_trash() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");

    let output = run(&mut sb.lfmclean(&["clean", "--ids", "gradle.caches", "--yes"]));

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(!folder.exists());
    assert!(sb.home.join(".Trash").exists());
}

#[test]
fn a_dry_run_removes_nothing_and_needs_no_yes() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");

    let output = run(&mut sb.lfmclean(&["--dry-run", "clean", "--ids", "gradle.caches"]));

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("would remove modules-2"));
    assert!(folder.exists());
}

#[test]
fn a_risky_cache_needs_allow_risky_too() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let avd = sb.plant(".android/avd/Pixel.avd");

    let refused = run(&mut sb.lfmclean(&["--permanent", "clean", "--ids", "android.avd", "--yes"]));
    assert!(!refused.status.success());
    assert!(text(&refused.stderr).contains("--allow-risky"));
    assert!(avd.exists());

    let allowed = run(&mut sb.lfmclean(&[
        "--permanent",
        "clean",
        "--ids",
        "android.avd",
        "--yes",
        "--allow-risky",
    ]));
    assert!(allowed.status.success(), "{}", text(&allowed.stderr));
    assert!(!avd.exists());
}

#[test]
fn allow_risky_alone_is_not_enough() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let output = run(&mut sb.lfmclean(&["clean", "--ids", "android.avd", "--allow-risky"]));

    assert!(!output.status.success());
}

#[test]
fn an_unknown_id_removes_nothing_even_of_the_known_ones() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");

    let output = run(&mut sb.lfmclean(&[
        "--permanent",
        "clean",
        "--ids",
        "gradle.caches,no.such",
        "--yes",
    ]));

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("unknown cache"));
    assert!(folder.exists());
}

#[test]
fn a_kept_id_is_refused() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");
    let keep = sb.home.join(".config/lfmclean/keep.toml");
    fs::create_dir_all(keep.parent().unwrap()).unwrap();
    fs::write(&keep, "targets = [\"gradle.caches\"]\n").unwrap();

    let output =
        run(&mut sb.lfmclean(&["--permanent", "clean", "--ids", "gradle.caches", "--yes"]));

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("keep list"));
    assert!(folder.exists());
}

#[test]
fn a_broken_keep_file_removes_nothing() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let folder = sb.plant(".gradle/caches/modules-2");
    let keep = sb.home.join(".config/lfmclean/keep.toml");
    fs::create_dir_all(keep.parent().unwrap()).unwrap();
    fs::write(&keep, "targets = [").unwrap();

    let output =
        run(&mut sb.lfmclean(&["--permanent", "clean", "--ids", "gradle.caches", "--yes"]));

    assert!(!output.status.success());
    assert!(folder.exists());
}

#[test]
fn history_of_nothing_says_so_and_json_is_valid() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let plain = text(&run(&mut sb.lfmclean(&["history"])).stdout);
    let json = text(&run(&mut sb.lfmclean(&["history", "--json"])).stdout);

    assert_eq!(plain, "Nothing has been removed yet.\n");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["entries"], serde_json::json!([]));
}

#[test]
fn history_json_has_the_removals() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    sb.plant(".gradle/caches/modules-2");
    run(&mut sb.lfmclean(&["--permanent", "clean", "--ids", "gradle.caches", "--yes"]));

    let json = text(&run(&mut sb.lfmclean(&["history", "--json"])).stdout);

    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let entries = value["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1]["stage"], "finished");
    assert_eq!(entries[1]["mode"], "permanent");
}

#[test]
fn screen_commands_need_a_terminal() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    // `history` is left out: off a terminal it prints the log.
    for name in ["clean", "packages", "purge", "keep"] {
        let output = run(&mut sb.lfmclean(&[name]));

        assert!(!output.status.success(), "{name}");
        assert!(text(&output.stderr).contains("needs a terminal"), "{name}");
    }
}

#[test]
fn a_kept_path_inside_an_item_refuses_the_whole_run() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let keep_this = sb.plant(".gradle/caches/modules-2");
    let other = sb.plant("Library/Developer/Xcode/DerivedData/App-abc");
    let keep = sb.home.join(".config/lfmclean/keep.toml");
    fs::create_dir_all(keep.parent().unwrap()).unwrap();
    fs::write(&keep, "paths = [\"~/.gradle/caches/modules-2/data.bin\"]\n").unwrap();

    let output = run(&mut sb.lfmclean(&[
        "--permanent",
        "clean",
        "--ids",
        "xcode.derived-data,gradle.caches",
        "--yes",
    ]));

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("kept by the keep list"));
    assert!(keep_this.exists() && other.exists());
}

#[test]
fn a_risky_cache_in_a_dry_run_needs_no_flags() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let avd = sb.plant(".android/avd/Pixel.avd");

    let output = run(&mut sb.lfmclean(&["--dry-run", "clean", "--ids", "android.avd"]));

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(avd.exists());
}

#[test]
fn allow_risky_covers_a_mixed_list() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let avd = sb.plant(".android/avd/Pixel.avd");
    let gradle = sb.plant(".gradle/caches/modules-2");

    let refused = run(&mut sb.lfmclean(&[
        "--permanent",
        "clean",
        "--ids",
        "gradle.caches,android.avd",
        "--yes",
    ]));
    assert!(!refused.status.success());
    assert!(avd.exists() && gradle.exists());

    let allowed = run(&mut sb.lfmclean(&[
        "--permanent",
        "clean",
        "--ids",
        "gradle.caches,android.avd",
        "--yes",
        "--allow-risky",
    ]));
    assert!(allowed.status.success(), "{}", text(&allowed.stderr));
    assert!(!avd.exists() && !gradle.exists());
}

#[test]
fn dry_run_and_permanent_together_are_refused() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let output = run(&mut sb.lfmclean(&["--dry-run", "--permanent", "history"]));

    assert!(!output.status.success());
}

#[test]
fn overlapping_ids_list_each_item_once() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    sb.plant("Library/Developer/Xcode/DerivedData/SourcePackages");

    let output = run(&mut sb.lfmclean(&[
        "--dry-run",
        "clean",
        "--ids",
        "xcode.derived-data,spm.source-packages",
    ]));

    let out = text(&output.stdout);
    assert_eq!(out.matches("SourcePackages").count(), 2, "{out}"); // plan + outcome
}

#[test]
fn simulators_are_refused_in_trash_mode() {
    let udid = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let sb = Sandbox::new(None);
    let data = sb.home.join(format!(
        "Library/Developer/CoreSimulator/Devices/{udid}/data"
    ));
    fs::create_dir_all(&data).unwrap();
    let json = format!(
        r#"{{"devices": {{"rt": [{{"udid": "{udid}", "dataPath": "{}"}}]}}}}"#,
        data.display()
    );
    let sb = Sandbox::with_simctl(sb, &json);

    let output = run(&mut sb.lfmclean(&["clean", "--ids", "xcode.sim-unavailable", "--yes"]));

    assert!(!output.status.success());
    assert!(
        text(&output.stderr).contains("--permanent"),
        "{}",
        text(&output.stderr)
    );
    assert!(data.exists());
}

fn plant_project(sb: &Sandbox, rel: &str) -> PathBuf {
    let root = sb.home.join(rel);
    fs::create_dir_all(root.join("build/classes")).unwrap();
    fs::write(root.join("build/classes/out.bin"), vec![1_u8; 50_000]).unwrap();
    fs::write(root.join("settings.gradle.kts"), "x").unwrap();
    root
}

#[test]
fn purge_json_lists_projects_and_removes_nothing() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    let root = plant_project(&sb, "Projects/App");

    let output = run(&mut sb.lfmclean(&["purge", "--json"]));

    assert!(output.status.success(), "{}", text(&output.stderr));
    let value: serde_json::Value = serde_json::from_str(&text(&output.stdout)).unwrap();
    assert_eq!(value["projects"][0]["name"], "App");
    assert_eq!(value["projects"][0]["items"][0]["name"], "build");
    assert_eq!(value["projects"][0]["items"][0]["target"], "purge.build");
    assert!(value["total_bytes"].as_u64().unwrap() >= 50_000);
    assert!(root.join("build").exists());
}

#[test]
fn purge_paths_chooses_the_folders_and_must_be_inside_home() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));
    plant_project(&sb, "Code/Elsewhere");

    let found = run(&mut sb.lfmclean(&["purge", "--json", "--paths", "~/Code"]));
    let default = run(&mut sb.lfmclean(&["purge", "--json"]));
    let outside = run(&mut sb.lfmclean(&["purge", "--json", "--paths", "/usr"]));

    let names = |out: &std::process::Output| {
        let v: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
        v["projects"].as_array().unwrap().len()
    };
    assert_eq!(names(&found), 1);
    assert_eq!(names(&default), 0);
    assert!(!outside.status.success());
    assert!(text(&outside.stderr).contains("inside your home"));
}

#[test]
fn purge_ids_are_not_cleaned_by_id() {
    let sb = Sandbox::new(Some(NO_SIMULATORS));

    let output = run(&mut sb.lfmclean(&["clean", "--ids", "purge.build", "--yes"]));

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("lfmclean purge"));
}
