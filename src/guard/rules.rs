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
    "Library/Caches/com.apple.dt.Instruments",
    "Library/Caches/com.apple.dt.InstrumentsCLI",
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

/// The only folders cleaned inside the private cache folder (`/private/var/folders/…/C`).
const USER_CACHE_ROOTS: &[&str] = &["com.apple.dt.Instruments", "com.apple.dt.InstrumentsCLI"];

/// The only entries removed from the private temporary folder (`/private/var/folders/…/T`), which
/// holds every app's temporary files: what an Instruments recording leaves behind, by name.
const USER_TEMP_PREFIX: &str = "instruments";
const USER_TEMP_SUFFIX: &str = ".ktrace";

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

/// True when `root` is one of the folders cleaned inside the private cache folder.
pub(super) fn is_user_cache_root(root: &Path, user_cache: Option<&Path>) -> bool {
    let Some(user_cache) = user_cache else {
        return false;
    };
    let root = lowercase(root);
    USER_CACHE_ROOTS
        .iter()
        .any(|rel| lowercase(&user_cache.join(rel)) == root)
}

/// True when `root` is the private temporary folder itself.
pub(super) fn is_user_temp_root(root: &Path, user_temp: Option<&Path>) -> bool {
    user_temp.is_some_and(|temp| lowercase(temp) == lowercase(root))
}

/// True when `name` is one an Instruments leftover in the private temporary folder has.
pub(super) fn is_user_temp_leftover(name: &str) -> bool {
    let name = name.to_lowercase();
    name.len() > USER_TEMP_PREFIX.len() + USER_TEMP_SUFFIX.len()
        && name.starts_with(USER_TEMP_PREFIX)
        && name.ends_with(USER_TEMP_SUFFIX)
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
