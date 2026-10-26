//! The `SwiftPM` repository cache and `Package.resolved`.
//!
//! The cache keeps one bare clone per package in `repositories/<Name>-<hash>`, named after the
//! last part of the package URL.

use std::path::Path;

use serde_json::Value;

use super::{Manager, PackageId};

const HASH_LEN: usize = 8;

/// The package a repository clone holds.
pub(super) fn identify(folder: &Path) -> Option<PackageId> {
    let entry = folder.file_name()?.to_str()?;
    Some(PackageId {
        manager: Manager::Spm,
        name: without_hash(entry).to_string(),
        version: None,
    })
}

fn without_hash(entry: &str) -> &str {
    match entry.rsplit_once('-') {
        Some((name, hash))
            if !name.is_empty()
                && hash.len() == HASH_LEN
                && hash.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            name
        }
        _ => entry,
    }
}

/// A package a `Package.resolved` pins: the names it can go by (lowercased) and its version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pin {
    pub names: Vec<String>,
    pub version: Option<String>,
}

/// The packages a `Package.resolved` pins, in any of its formats (version 1, 2 or 3).
///
/// # Errors
/// When the text is not JSON or has no `pins` list.
pub(super) fn parse_resolved(text: &str) -> Result<Vec<Pin>, String> {
    let json: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let pins = json
        .get("pins")
        .or_else(|| json.get("object").and_then(|o| o.get("pins")))
        .and_then(Value::as_array)
        .ok_or("no pins list")?;
    Ok(pins.iter().map(pin).collect())
}

fn pin(pin: &Value) -> Pin {
    let text = |key: &str| pin.get(key).and_then(Value::as_str);
    let from_url = ["location", "repositoryURL"]
        .into_iter()
        .filter_map(text)
        .filter_map(url_name);
    let mut names: Vec<String> = ["identity", "package"]
        .into_iter()
        .filter_map(text)
        .map(str::to_string)
        .chain(from_url)
        .map(|name| name.to_lowercase())
        .collect();
    names.sort();
    names.dedup();
    Pin {
        names,
        version: pin
            .get("state")
            .and_then(|state| state.get("version"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// `https://github.com/SnapKit/SnapKit.git` is `SnapKit`, the name the cache folder uses.
fn url_name(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit('/').next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_repository_folder_is_named_without_its_hash() {
        let found = identify(&PathBuf::from(
            "/c/org.swift.swiftpm/repositories/SnapKit-a5691e5b",
        ))
        .unwrap();

        assert_eq!(found.manager, Manager::Spm);
        assert_eq!(found.name, "SnapKit");
        assert_eq!(found.version, None);
    }

    #[test]
    fn a_name_with_dashes_keeps_them() {
        let found = identify(&PathBuf::from("/r/swift-argument-parser-0123abcd")).unwrap();

        assert_eq!(found.name, "swift-argument-parser");
        assert_eq!(
            identify(&PathBuf::from("/r/no-hash")).unwrap().name,
            "no-hash"
        );
    }

    #[test]
    fn version_2_and_3_files_give_identity_url_name_and_version() {
        let text = r#"{"originHash": "x", "pins": [{
            "identity": "snapkit", "kind": "remoteSourceControl",
            "location": "https://github.com/SnapKit/SnapKit.git",
            "state": {"revision": "abc", "version": "5.7.1"}
        }, {
            "identity": "local", "kind": "remoteSourceControl",
            "location": "https://example.com/Branch-Only/",
            "state": {"branch": "main", "revision": "def"}
        }], "version": 3}"#;

        let pins = parse_resolved(text).unwrap();

        assert_eq!(
            pins,
            [
                Pin {
                    names: vec!["snapkit".to_string()],
                    version: Some("5.7.1".to_string()),
                },
                Pin {
                    names: vec!["branch-only".to_string(), "local".to_string()],
                    version: None,
                },
            ]
        );
    }

    #[test]
    fn version_1_files_are_read_too() {
        let text = r#"{"object": {"pins": [{
            "package": "Kingfisher",
            "repositoryURL": "https://github.com/onevcat/Kingfisher",
            "state": {"version": "7.0.0"}
        }]}, "version": 1}"#;

        let pins = parse_resolved(text).unwrap();

        assert_eq!(pins[0].names, ["kingfisher"]);
        assert_eq!(pins[0].version.as_deref(), Some("7.0.0"));
    }

    #[test]
    fn text_that_is_not_a_resolved_file_is_an_error() {
        assert!(parse_resolved("nope").is_err());
        assert!(parse_resolved(r#"{"version": 2}"#).is_err());
    }
}
