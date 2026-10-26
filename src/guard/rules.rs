//! Which folders may be cleaned inside, and which paths are never removed. Paths are relative to
//! home and compared without regard to letter case, as the default APFS volume does.

use std::path::{Path, PathBuf};

/// The only folders the tool cleans inside. An entry must be strictly inside one of them.
const CACHE_ROOTS: &[&str] = &[
    "Library/Developer/Xcode/DerivedData",
    "Library/Developer/Xcode/Archives",
    "Library/Developer/Xcode/iOS DeviceSupport",
    "Library/Developer/Xcode/watchOS DeviceSupport",
    "Library/Developer/Xcode/tvOS DeviceSupport",
    "Library/Developer/Xcode/visionOS DeviceSupport",
    "Library/Developer/Xcode/UserData/Previews",
    "Library/Developer/CoreSimulator/Caches",
    "Library/Caches/org.swift.swiftpm",
    "Library/Caches/CocoaPods",
    "Library/Caches/JetBrains",
    "Library/Caches/Google",
    "Library/Logs/Google",
    "Library/Android/sdk/system-images",
    ".android/avd",
    ".gradle/caches",
    ".gradle/daemon",
    ".gradle/wrapper/dists",
    ".konan",
    ".cocoapods/repos",
];

/// Never removed, nor any folder that contains them. The tool only ever cleans inside these.
const NEVER_REMOVED_OR_CONTAINED: &[&str] = &[
    "Library",
    "Library/Caches",
    "Library/Logs",
    "Library/Developer",
    "Library/Developer/Xcode",
    "Library/Developer/Xcode/UserData",
    "Library/Developer/CoreSimulator",
    "Library/Android",
    "Library/Android/sdk",
    "Documents",
    "Desktop",
    "Downloads",
    "Pictures",
    "Movies",
    "Music",
    ".android",
    ".gradle",
    ".konan",
    ".cocoapods",
];

/// Never removed, nor anything inside them: keys, signing files, settings and this tool's log.
const NEVER_REMOVED_OR_ENTERED: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".netrc",
    ".npmrc",
    ".gem/credentials",
    ".fastlane",
    ".appstoreconnect",
    "private_keys",
    ".private_keys",
    ".m2/settings.xml",
    ".m2/settings-security.xml",
    "Library/Keychains",
    "Library/MobileDevice/Provisioning Profiles",
    "Library/Developer/Xcode/UserData/Provisioning Profiles",
    "Library/Developer/Xcode/UserData/CodeSnippets",
    "Library/Developer/Xcode/UserData/FontAndColorThemes",
    "Library/Developer/Xcode/UserData/KeyBindings",
    "Library/Logs/lfmclean",
    ".android/debug.keystore",
    ".android/adbkey",
    ".android/adbkey.pub",
    ".gradle/gradle.properties",
    ".gradle/init.d",
    ".gradle/init.gradle",
    ".gradle/init.gradle.kts",
    "Library/Android/sdk/licenses",
    "Library/Android/sdk/platforms",
    "Library/Android/sdk/build-tools",
    "Library/Android/sdk/platform-tools",
];

pub(super) fn is_cache_root(root: &Path, home: &Path) -> bool {
    let root = lowercase(root);
    CACHE_ROOTS
        .iter()
        .any(|rel| lowercase(&home.join(rel)) == root)
}

/// True for a cache folder, anything inside one, and the folders of tools whose files are not
/// projects (`.m2`, `.android`): a project is never looked for there.
pub(super) fn is_in_tool_folder(path: &Path, home: &Path) -> bool {
    let path = lowercase(path);
    CACHE_ROOTS
        .iter()
        .chain(&[".m2", ".android"])
        .any(|rel| path.starts_with(lowercase(&home.join(rel))))
}

pub(super) fn is_protected(path: &Path, home: &Path) -> bool {
    let path = lowercase(path);
    let contains_protected = NEVER_REMOVED_OR_CONTAINED
        .iter()
        .chain(NEVER_REMOVED_OR_ENTERED)
        .any(|rel| lowercase(&home.join(rel)).starts_with(&path));
    let inside_protected = NEVER_REMOVED_OR_ENTERED
        .iter()
        .any(|rel| path.starts_with(lowercase(&home.join(rel))));
    contains_protected || inside_protected
}

/// The protected names are ASCII, so lowercasing is enough; Unicode normalization is not needed.
fn lowercase(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/someone";

    fn at(rel: &str) -> PathBuf {
        Path::new(HOME).join(rel)
    }

    #[test]
    fn cache_roots_match_in_any_letter_case() {
        let home = Path::new(HOME);
        assert!(is_cache_root(
            &at("Library/Developer/Xcode/DerivedData"),
            home
        ));
        assert!(is_cache_root(
            &at("library/developer/xcode/deriveddata"),
            home
        ));
        assert!(is_cache_root(&at(".GRADLE/Caches"), home));
    }

    #[test]
    fn broad_folders_are_not_cache_roots() {
        let home = Path::new(HOME);
        for rel in [
            "",
            "Documents",
            "Library",
            "Library/Caches",
            ".gradle",
            ".android",
        ] {
            assert!(!is_cache_root(&at(rel), home), "{rel:?} must not be a root");
        }
        assert!(!is_cache_root(&at(".gradle/caches/modules-2"), home));
    }

    #[test]
    fn secrets_are_protected_in_any_letter_case() {
        let home = Path::new(HOME);
        for rel in [
            ".gradle/GRADLE.PROPERTIES",
            ".gradle/Init.d/repos.gradle",
            ".android/Debug.Keystore",
            "Library/KEYCHAINS",
            "library/keychains/login.keychain-db",
            ".ssh/id_ed25519",
            "Library/Developer/Xcode/UserData/Provisioning Profiles/abc.mobileprovision",
            "Library/Logs/lfmclean/clean.log",
        ] {
            assert!(is_protected(&at(rel), home), "{rel} must be protected");
        }
    }

    #[test]
    fn folders_holding_protected_paths_are_protected() {
        let home = Path::new(HOME);
        for rel in [
            "",
            "Library",
            "LIBRARY/Developer",
            "Library/Developer/Xcode/UserData",
            ".gradle",
        ] {
            assert!(is_protected(&at(rel), home), "{rel:?} must be protected");
        }
    }

    #[test]
    fn cache_entries_are_not_protected() {
        let home = Path::new(HOME);
        for rel in [
            "Library/Developer/Xcode/DerivedData/App-abc",
            "Library/Developer/Xcode/UserData/Previews/Simulator Devices",
            ".gradle/caches/modules-2",
            ".konan/kotlin-native-prebuilt-macos-aarch64-2.0.0",
        ] {
            assert!(!is_protected(&at(rel), home), "{rel} must be removable");
        }
    }
}
