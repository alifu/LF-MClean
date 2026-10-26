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
const MAX_UDID_LEN: usize = 64;

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
    /// The id `simctl` uses, only letters, digits and `-`.
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
    if !is_plain_id(&device.udid) {
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

/// Letters, digits and `-` only, so the id is one path component and cannot be `..` or `/`.
fn is_plain_id(udid: &str) -> bool {
    !udid.is_empty()
        && udid.len() <= MAX_UDID_LEN
        && udid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
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
        let runner = FakeRunner::answering(&listing(&fx, &["AAAA-1111", "BBBB-2222"]));

        let mut found = unavailable_simulators(&fx.env, &runner).unwrap();

        found.sort_by(|a, b| a.udid.cmp(&b.udid));
        let simulator = |udid: &str| Simulator {
            udid: udid.to_string(),
            folder: fx.home().join(DEVICES_DIR).join(udid),
        };
        assert_eq!(found, [simulator("AAAA-1111"), simulator("BBBB-2222")]);
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
                {{"udid": "AAAA", "dataPath": "{good}/AAAA/data"}},
                {{"udid": "BBBB", "dataPath": "{outside}/BBBB/data"}}
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
            r#"{{"devices": {{"rt": [{{"udid": "AAAA", "dataPath": "{}/OTHER/data"}}]}}}}"#,
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
    fn a_very_long_id_is_refused() {
        let fx = Fixture::new();
        let udid = "A".repeat(MAX_UDID_LEN + 1);
        let json = format!(
            r#"{{"devices": {{"rt": [{{"udid": "{udid}", "dataPath": "{}/{udid}/data"}}]}}}}"#,
            fx.home().join(DEVICES_DIR).display()
        );

        let result = unavailable_simulators(&fx.env, &FakeRunner::answering(&json));

        assert!(matches!(result, Err(SimctlError::BadUdid(_))));
    }

    #[test]
    fn a_device_without_a_data_path_is_refused() {
        let fx = Fixture::new();
        let runner = FakeRunner::answering(r#"{"devices": {"rt": [{"udid": "AAAA"}]}}"#);

        assert!(matches!(
            unavailable_simulators(&fx.env, &runner),
            Err(SimctlError::NoDataFolder(_))
        ));
    }
}
