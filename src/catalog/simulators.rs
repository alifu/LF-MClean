//! Simulators whose runtime is gone. `simctl` says which ones, and removes them; this module only
//! asks, and checks the answer before any folder is measured or any id is remembered.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::command::CommandRunner;
use crate::env::Env;

const DEVICES_DIR: &str = "Library/Developer/CoreSimulator/Devices";
/// A udid is a UUID: groups of 8, 4, 4, 4 and 12 hex digits.
const UDID_GROUPS: [usize; 5] = [8, 4, 4, 4, 12];

#[derive(Deserialize)]
struct SimctlList {
    devices: HashMap<String, Vec<Device>>,
}

#[derive(Deserialize)]
struct Device {
    udid: String,
    #[serde(rename = "dataPath")]
    data_path: Option<PathBuf>,
}

/// A simulator `simctl` reports as unavailable, after its id and folder were checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simulator {
    /// The id `simctl` uses, a UUID (checked by `is_udid`).
    pub udid: String,
    /// Its folder: exactly `<udid>` inside `CoreSimulator/Devices` in the home folder.
    pub folder: PathBuf,
}

/// Why the answer from `simctl` cannot be used. Nothing is used from an answer that fails.
#[derive(Debug, Error)]
pub enum SimctlError {
    /// `xcrun simctl` did not run or did not succeed.
    #[error("simctl could not be run: {0}")]
    CannotRun(#[source] io::Error),
    /// The answer is not the JSON that was asked for.
    #[error("the answer from simctl could not be read: {0}")]
    Unreadable(#[source] serde_json::Error),
    /// A device has no `dataPath`.
    #[error("simctl gave no data folder for {0}")]
    NoDataFolder(String),
    /// A device id could be used to point outside its folder.
    #[error("simctl gave the unusable id {0:?}; nothing was used")]
    BadUdid(String),
    /// The folder is not `<udid>` inside the simulators folder.
    #[error(
        "simctl named {folder}, which is not the folder of {udid} (outside the simulators folder); nothing was used"
    )]
    WrongFolder {
        /// The device.
        udid: String,
        /// The folder `simctl` named.
        folder: PathBuf,
    },
}

/// The simulators `simctl` reports as unavailable.
///
/// # Errors
/// A `SimctlError` when `simctl` cannot be run, answers something unreadable, or names an id or
/// folder that is not what a simulator has.
pub fn unavailable_simulators(
    env: &Env,
    runner: &dyn CommandRunner,
) -> Result<Vec<Simulator>, SimctlError> {
    let output = runner
        .run("xcrun", &["simctl", "list", "devices", "unavailable", "-j"])
        .map_err(SimctlError::CannotRun)?;
    let listing: SimctlList = serde_json::from_str(&output).map_err(SimctlError::Unreadable)?;
    let devices_dir = env.home().join(DEVICES_DIR);
    listing
        .devices
        .into_values()
        .flatten()
        .map(|device| checked(device, &devices_dir))
        .collect()
}

/// `<devices_dir>/<udid>`, the folder above the reported `data` folder, if that is what was named.
fn checked(device: Device, devices_dir: &Path) -> Result<Simulator, SimctlError> {
    if !is_udid(&device.udid) {
        return Err(SimctlError::BadUdid(device.udid));
    }
    let folder = device
        .data_path
        .as_deref()
        .and_then(Path::parent)
        .ok_or_else(|| SimctlError::NoDataFolder(device.udid.clone()))?;
    if folder != devices_dir.join(&device.udid) {
        return Err(SimctlError::WrongFolder {
            folder: folder.to_path_buf(),
            udid: device.udid,
        });
    }
    Ok(Simulator {
        folder: folder.to_path_buf(),
        udid: device.udid,
    })
}

/// True for a UUID such as `0F3B2C1A-9D4E-4B5F-8A7C-1E2D3C4B5A69`, and nothing else: no path
/// separator, no `..`, and none of the words `simctl` treats specially (`all`, `booted`,
/// `unavailable`) or an option starting with `-`.
#[must_use]
pub fn is_udid(udid: &str) -> bool {
    let groups: Vec<&str> = udid.split('-').collect();
    groups.len() == UDID_GROUPS.len()
        && groups
            .iter()
            .zip(UDID_GROUPS)
            .all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeRunner, Fixture};

    fn listing(fx: &Fixture, udids: &[&str]) -> String {
        let devices: Vec<String> = udids
            .iter()
            .map(|udid| {
                let data = fx.home().join(DEVICES_DIR).join(udid).join("data");
                format!(
                    r#"{{"udid": "{udid}", "dataPath": "{}", "name": "iPhone", "isAvailable": false}}"#,
                    data.display()
                )
            })
            .collect();
        format!(
            r#"{{"devices": {{"com.apple.CoreSimulator.SimRuntime.iOS-17-0": [{}], "com.apple.CoreSimulator.SimRuntime.iOS-18-0": []}}}}"#,
            devices.join(",")
        )
    }

    #[test]
    fn asks_simctl_for_unavailable_devices_as_json() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(&listing(&fx, &[]));

        unavailable_simulators(&fx.env, &runner).unwrap();

        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [["xcrun", "simctl", "list", "devices", "unavailable", "-j"]]
        );
    }

    #[test]
    fn returns_the_device_folder_not_its_data_folder() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(&listing(
            &fx,
            &[
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
            ],
        ));

        let mut found = unavailable_simulators(&fx.env, &runner).unwrap();

        found.sort_by(|a, b| a.udid.cmp(&b.udid));
        let simulator = |udid: &str| Simulator {
            udid: udid.to_string(),
            folder: fx.home().join(DEVICES_DIR).join(udid),
        };
        assert_eq!(
            found,
            [
                simulator("11111111-1111-4111-8111-111111111111"),
                simulator("22222222-2222-4222-8222-222222222222")
            ]
        );
    }

    #[test]
    fn no_unavailable_devices_is_an_empty_list() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(&listing(&fx, &[]));

        assert_eq!(unavailable_simulators(&fx.env, &runner).unwrap(), vec![]);
    }

    #[test]
    fn a_program_that_cannot_run_is_explained() {
        let fx = Fixture::new();
        let runner = FakeRunner::failing("xcrun: error: unable to find utility");

        let error = unavailable_simulators(&fx.env, &runner).unwrap_err();

        assert!(matches!(error, SimctlError::CannotRun(_)));
        let reason = error.to_string();
        assert!(reason.contains("simctl"), "{reason}");
        assert!(reason.contains("unable to find utility"), "{reason}");
    }

    #[test]
    fn an_answer_that_is_not_json_is_refused() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering("not json");

        let error = unavailable_simulators(&fx.env, &runner).unwrap_err();

        assert!(matches!(error, SimctlError::Unreadable(_)));
        assert!(error.to_string().contains("could not be read"));
    }

    #[test]
    fn a_folder_outside_the_devices_folder_refuses_the_whole_answer() {
        let fx = Fixture::new();
        let bad = format!(
            r#"{{"devices": {{"rt": [
                {{"udid": "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA", "dataPath": "{good}/AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA/data"}},
                {{"udid": "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB", "dataPath": "{outside}/BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB/data"}}
            ]}}}}"#,
            good = fx.home().join(DEVICES_DIR).display(),
            outside = fx.outside.display(),
        );
        let runner = FakeRunner::answering(&bad);

        let error = unavailable_simulators(&fx.env, &runner).unwrap_err();

        assert!(matches!(error, SimctlError::WrongFolder { .. }));
        assert!(error.to_string().contains("outside"));
    }

    #[test]
    fn a_folder_that_is_not_named_after_the_udid_is_refused() {
        let fx = Fixture::new();
        let wrong = format!(
            r#"{{"devices": {{"rt": [{{"udid": "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA", "dataPath": "{}/OTHER/data"}}]}}}}"#,
            fx.home().join(DEVICES_DIR).display()
        );
        let runner = FakeRunner::answering(&wrong);

        assert!(matches!(
            unavailable_simulators(&fx.env, &runner),
            Err(SimctlError::WrongFolder { .. })
        ));
    }

    #[test]
    fn ids_that_could_point_elsewhere_are_refused() {
        let fx = Fixture::new();
        let devices = fx.home().join(DEVICES_DIR);
        // (udid, dataPath) pairs whose parent folder would equal devices_dir.join(udid).
        let attacks = [
            ("", devices.join("data")),
            ("..", devices.join("../data")),
            ("../..", devices.join("../../data")),
            ("/", PathBuf::from("/data")),
            ("/Users", PathBuf::from("/Users/data")),
            ("a/b", devices.join("a/b/data")),
            ("a b", devices.join("a b/data")),
            ("a\nb", devices.join("a\nb/data")),
        ];
        for (udid, data_path) in attacks {
            let json = serde_json::json!({"devices": {"rt": [
                {"udid": udid, "dataPath": data_path}
            ]}})
            .to_string();

            let result = unavailable_simulators(&fx.env, &FakeRunner::answering(&json));

            assert!(
                matches!(result, Err(SimctlError::BadUdid(_))),
                "{udid:?} must be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn only_uuids_are_udids() {
        assert!(is_udid("0F3B2C1A-9D4E-4B5F-8A7C-1E2D3C4B5A69"));
        assert!(is_udid("0f3b2c1a-9d4e-4b5f-8a7c-1e2d3c4b5a69"));
        for bad in [
            "all",
            "booted",
            "unavailable",
            "-h",
            "--set",
            "0F3B2C1A-9D4E-4B5F-8A7C",
            "0F3B2C1A-9D4E-4B5F-8A7C-1E2D3C4B5A69-0000",
            "0F3B2C1A9D4E4B5F8A7C1E2D3C4B5A69",
            "0F3B2C1A-9D4E-4B5F-8A7C-1E2D3C4B5A6Z",
            " 0F3B2C1A-9D4E-4B5F-8A7C-1E2D3C4B5A69",
        ] {
            assert!(!is_udid(bad), "{bad:?}");
        }
    }

    #[test]
    fn simctl_keywords_given_as_ids_are_refused() {
        let fx = Fixture::new();
        for word in ["all", "booted", "unavailable"] {
            let json = format!(
                r#"{{"devices": {{"rt": [{{"udid": "{word}", "dataPath": "{}/{word}/data"}}]}}}}"#,
                fx.home().join(DEVICES_DIR).display()
            );

            let result = unavailable_simulators(&fx.env, &FakeRunner::answering(&json));

            assert!(matches!(result, Err(SimctlError::BadUdid(_))), "{word}");
        }
    }

    #[test]
    fn a_device_without_a_data_path_is_refused() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(
            r#"{"devices": {"rt": [{"udid": "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"}]}}"#,
        );

        assert!(matches!(
            unavailable_simulators(&fx.env, &runner),
            Err(SimctlError::NoDataFolder(_))
        ));
    }
}
