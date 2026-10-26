//! The `CocoaPods` download cache and `Podfile.lock`.
//!
//! The cache keeps released pods in `Pods/Release/<Name>/<version>-<checksum>` and pods from git
//! or a path in `Pods/External/<Name>/<hash>`.

use std::path::Path;

use super::{Manager, PackageId};

const RELEASE: &str = "Release";
const CHECKSUM_LEN: usize = 5;
const PODS_SECTION: &str = "PODS:";
const POD_LINE: &str = "  - ";

/// The pod and version a cache folder holds.
pub(super) fn identify(folder: &Path) -> Option<PackageId> {
    let entry = folder.file_name()?.to_str()?;
    let pod = folder.parent()?;
    let name = pod.file_name()?.to_str()?;
    let kind = pod.parent()?.file_name()?.to_str()?;
    Some(PackageId {
        manager: Manager::Pods,
        name: name.to_string(),
        version: if kind == RELEASE {
            version_of(entry)
        } else {
            None
        },
    })
}

/// What to call a copy whose version is unknown: `external` for a pod from git or a path, else
/// the folder name.
pub(super) fn unversioned_label(folder: &Path) -> String {
    let kind = folder
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name);
    match (kind.and_then(|k| k.to_str()), folder.file_name()) {
        (Some(RELEASE), Some(name)) => name.to_string_lossy().into_owned(),
        _ => "external".to_string(),
    }
}

/// `10.25.0-1a2b3` is version `10.25.0`. A name without the checksum is not a folder `CocoaPods`
/// made, so its version is unknown, and any keep entry for the pod covers it.
fn version_of(entry: &str) -> Option<String> {
    let (version, sum) = entry.rsplit_once('-')?;
    let is_checksum = sum.len() == CHECKSUM_LEN && sum.chars().all(|c| c.is_ascii_hexdigit());
    (!version.is_empty() && is_checksum).then(|| version.to_string())
}

/// The pods a `Podfile.lock` installs, as `(name, version)`; subspecs count as their pod.
///
/// # Errors
/// When there is no `PODS:` section or a pod line cannot be read.
pub(super) fn parse_lock(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim_end() != PODS_SECTION);
    if lines.next().is_none() {
        return Err("no PODS section".to_string());
    }
    let mut pods: Vec<(String, String)> = lines
        .take_while(|line| line.starts_with(' ') || line.is_empty())
        .filter(|line| line.starts_with(POD_LINE))
        .map(pod_line)
        .collect::<Result<_, _>>()?;
    pods.sort();
    pods.dedup();
    Ok(pods)
}

/// `  - "Firebase/Core (10.25.0)":` is `("Firebase", "10.25.0")`.
fn pod_line(line: &str) -> Result<(String, String), String> {
    let entry = line[POD_LINE.len()..]
        .trim_end()
        .trim_end_matches(':')
        .trim_matches('"');
    let (spec, version) = entry
        .rsplit_once(" (")
        .and_then(|(spec, rest)| Some((spec, rest.strip_suffix(')')?)))
        .ok_or_else(|| format!("cannot read the pod line {:?}", line.trim()))?;
    let name = spec.split('/').next().unwrap_or(spec);
    if name.is_empty() || version.is_empty() {
        return Err(format!("cannot read the pod line {:?}", line.trim()));
    }
    Ok((name.to_string(), version.to_string()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const CACHE: &str = "/Users/me/Library/Caches/CocoaPods/Pods";

    fn id(rel: &str) -> Option<PackageId> {
        identify(&PathBuf::from(CACHE).join(rel))
    }

    #[test]
    fn a_release_folder_is_a_pod_version_without_the_checksum() {
        let found = id("Release/Firebase/10.25.0-1a2b3").unwrap();

        assert_eq!(found.manager, Manager::Pods);
        assert_eq!(found.name, "Firebase");
        assert_eq!(found.version.as_deref(), Some("10.25.0"));
    }

    #[test]
    fn a_prerelease_version_keeps_its_own_dashes() {
        let found = id("Release/Lottie/4.0.0-beta.1-abcde").unwrap();

        assert_eq!(found.version.as_deref(), Some("4.0.0-beta.1"));
    }

    #[test]
    fn a_folder_without_a_checksum_has_an_unknown_version() {
        assert_eq!(id("Release/Odd/1.0.0-final").unwrap().version, None);
        assert_eq!(id("Release/Odd/1.0.0").unwrap().version, None);
        assert_eq!(id("Release/Odd/-abcde").unwrap().version, None);
    }

    #[test]
    fn unknown_versions_are_labelled_by_folder_or_as_external() {
        let path = |rel: &str| PathBuf::from(CACHE).join(rel);

        assert_eq!(unversioned_label(&path("Release/Odd/1.0.0")), "1.0.0");
        assert_eq!(unversioned_label(&path("External/Fork/9f8e7d")), "external");
    }

    #[test]
    fn an_external_pod_has_no_version() {
        let found = id("External/MyFork/9f8e7d6c5b4a").unwrap();

        assert_eq!(found.name, "MyFork");
        assert_eq!(found.version, None);
    }

    #[test]
    fn a_lock_file_lists_each_pod_once_with_subspecs_folded_in() {
        let lock = "\
PODS:
  - Alamofire (5.4.4)
  - AppAuth (1.4.0):
    - AppAuth/Core (= 1.4.0)
  - AppAuth/Core (1.4.0)
  - \"GoogleUtilities/Environment (7.12.0)\":
    - PromisesObjC (< 3.0, >= 1.2)

DEPENDENCIES:
  - Alamofire
  - Ignored (9.9.9)
";

        let pods = parse_lock(lock).unwrap();

        assert_eq!(
            pods,
            [
                ("Alamofire".to_string(), "5.4.4".to_string()),
                ("AppAuth".to_string(), "1.4.0".to_string()),
                ("GoogleUtilities".to_string(), "7.12.0".to_string()),
            ]
        );
    }

    #[test]
    fn a_file_that_is_not_a_lock_file_is_an_error() {
        assert!(parse_lock("hello").is_err());
    }

    #[test]
    fn a_pod_line_without_a_version_is_an_error() {
        let error = parse_lock("PODS:\n  - Alamofire\n").unwrap_err();

        assert!(error.contains("Alamofire"), "{error}");
    }
}
