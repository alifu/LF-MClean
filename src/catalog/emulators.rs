//! Android emulators: a `.avd` folder and the `.ini` file in the same folder that points to it.
//! The `.ini` is named after the emulator's id, which can differ from its folder name.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::Discovery;

const AVD_SUFFIX: &str = ".avd";
const INI_SUFFIX: &str = ".ini";
const MAX_INI_BYTES: u64 = 64 * 1024;
const PATH_KEYS: [&str; 2] = ["path.rel=", "path="];

/// Names each emulator without `.avd` and adds the `.ini` files that point to its folder, after
/// the folder itself so a failed removal of the folder leaves the emulator listed.
#[must_use]
pub fn attach_ini_files(mut found: Discovery) -> Discovery {
    let Some(root) = found
        .candidates
        .first()
        .map(|candidate| candidate.root.clone())
    else {
        return found;
    };
    let pointers: Vec<(String, PathBuf)> = ini_files(&root)
        .into_iter()
        .filter_map(|ini| Some((target_folder(&ini)?, ini)))
        .collect();
    for candidate in &mut found.candidates {
        let folder = candidate.paths[0]
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let inis = pointers
            .iter()
            .filter(|(target, _)| *target == folder)
            .map(|(_, ini)| ini.clone());
        candidate.paths.extend(inis);
        candidate.name = candidate
            .name
            .strip_suffix(AVD_SUFFIX)
            .unwrap_or(&candidate.name)
            .to_string();
    }
    found
}

/// The folder name an `.ini` file points to, from its `path.rel` or `path` line.
fn target_folder(ini: &Path) -> Option<String> {
    let mut text = String::new();
    File::open(ini)
        .ok()?
        .take(MAX_INI_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    PATH_KEYS.iter().find_map(|key| {
        let value = text.lines().find_map(|line| line.strip_prefix(key))?;
        let name = Path::new(value.trim()).file_name()?;
        Some(name.to_string_lossy().into_owned())
    })
}

/// The regular `.ini` files directly in `dir`; a symlink is not followed.
fn ini_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(INI_SUFFIX))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Candidate, Filter, Source, discover};
    use crate::testing::Fixture;

    fn emulators(fx: &Fixture) -> Discovery {
        let source = Source::children(".android/avd", Filter::Suffix(".avd"));
        attach_ini_files(discover(&fx.env, &source))
    }

    fn write_ini(fx: &Fixture, name: &str, text: &str) -> PathBuf {
        let path = fx.home().join(".android/avd").join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn an_emulator_goes_with_the_ini_that_points_to_it() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/Medium_Phone.avd");
        let ini = write_ini(
            &fx,
            "Medium_Phone_API_36.ini",
            "avd.ini.encoding=UTF-8\npath=/Users/x/.android/avd/Medium_Phone.avd\npath.rel=avd/Medium_Phone.avd\ntarget=android-36\n",
        );

        let found = emulators(&fx);

        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].name, "Medium_Phone");
        assert_eq!(found.candidates[0].paths, vec![avd, ini]);
    }

    #[test]
    fn the_absolute_path_line_is_used_when_there_is_no_relative_one() {
        let fx = Fixture::new();
        fx.dir(".android/avd/Pixel.avd");
        let ini = write_ini(&fx, "Pixel.ini", "path=/Users/x/.android/avd/Pixel.avd\n");

        assert_eq!(emulators(&fx).candidates[0].paths[1], ini);
    }

    #[test]
    fn an_emulator_without_an_ini_is_listed_alone() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/Lonely.avd");

        assert_eq!(emulators(&fx).candidates[0].paths, vec![avd]);
    }

    #[test]
    fn an_ini_that_points_elsewhere_is_left_alone() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/A.avd");
        write_ini(&fx, "B.ini", "path.rel=avd/B.avd\n");

        assert_eq!(emulators(&fx).candidates[0].paths, vec![avd]);
    }

    #[test]
    fn an_orphan_ini_is_not_an_emulator() {
        let fx = Fixture::new();
        fx.dir(".android/avd");
        write_ini(&fx, "Gone.ini", "path.rel=avd/Gone.avd\n");

        assert_eq!(emulators(&fx).candidates, Vec::<Candidate>::new());
    }

    #[test]
    fn an_unreadable_or_odd_ini_is_ignored() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/A.avd");
        write_ini(&fx, "binary.ini", "\u{0}\u{1}no equals sign here");
        write_ini(&fx, "empty.ini", "");

        assert_eq!(emulators(&fx).candidates[0].paths, vec![avd]);
    }

    #[test]
    fn an_ini_that_is_a_symlink_is_not_followed() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/A.avd");
        let target = fx.outside.join("A.ini");
        std::fs::write(&target, "path.rel=avd/A.avd\n").unwrap();
        std::os::unix::fs::symlink(&target, fx.home().join(".android/avd/A.ini")).unwrap();

        assert_eq!(emulators(&fx).candidates[0].paths, vec![avd]);
    }
}
