//! How much disk space a file or folder takes: allocated blocks, the way `du` and Finder count.
//! Symlinks are never followed, a file with several hard links counts once, and a folder on
//! another volume (a mount point) is not entered.

use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use jwalk::WalkDirGeneric;

const BLOCK_BYTES: u64 = 512;

/// What measuring a path found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Allocated bytes of everything that could be read.
    pub bytes: u64,
    /// Entries that could not be read; when above zero `bytes` is a lower bound.
    pub unreadable: u64,
}

/// Measures `path`, which may be a file, a symlink (the link itself) or a folder.
#[must_use]
pub fn disk_usage(path: &Path) -> Usage {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Usage {
            bytes: 0,
            unreadable: 1,
        };
    };
    if meta.is_dir() {
        return folder_usage(path, &meta);
    }
    Usage {
        bytes: allocated(&meta),
        unreadable: 0,
    }
}

/// What the parallel reader records about each entry, so counting needs no second `lstat`.
#[derive(Debug, Clone, Copy)]
struct Seen {
    is_dir: bool,
    bytes: u64,
    links: u64,
    dev: u64,
    ino: u64,
}

impl Seen {
    fn of(meta: &Metadata) -> Self {
        Self {
            is_dir: meta.is_dir(),
            bytes: allocated(meta),
            links: meta.nlink(),
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

type Walk = WalkDirGeneric<((), Option<Seen>)>;

fn folder_usage(path: &Path, root: &Metadata) -> Usage {
    let root_dev = root.dev();
    let walk = Walk::new(path)
        .follow_links(false)
        .skip_hidden(false)
        .process_read_dir(move |_, _, (), children| {
            for child in children.iter_mut().flatten() {
                let Ok(meta) = child.metadata() else { continue };
                if meta.is_dir() && meta.dev() != root_dev {
                    child.read_children = None;
                }
                child.client_state = Some(Seen::of(&meta));
            }
        });
    let mut usage = Usage {
        bytes: allocated(root),
        unreadable: 0,
    };
    let mut counted_links = HashSet::new();
    for entry in walk {
        let Ok(entry) = entry else {
            usage.unreadable += 1;
            continue;
        };
        // A folder that could not be opened is not an `Err` item; its error is kept on the entry.
        // The item folder itself (depth 0) can be one, so this runs before it is skipped.
        let cannot_open = entry
            .read_children
            .as_ref()
            .is_some_and(|children| children.error().is_some());
        usage.unreadable += u64::from(cannot_open);
        if entry.depth() == 0 {
            continue;
        }
        let Some(seen) = entry.client_state else {
            usage.unreadable += 1;
            continue;
        };
        // Folders are never hard-linked, however many links APFS reports for them.
        let first_sight =
            seen.is_dir || seen.links <= 1 || counted_links.insert((seen.dev, seen.ino));
        if first_sight {
            usage.bytes += seen.bytes;
        }
    }
    usage
}

fn allocated(meta: &Metadata) -> u64 {
    meta.blocks() * BLOCK_BYTES
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;
    use crate::testing::{Fixture, MountedImage};

    const BLOCK: u64 = 512;
    const LOCKED: u32 = 0o000;
    const OPEN: u32 = 0o755;

    fn allocated(path: &Path) -> u64 {
        fs::symlink_metadata(path).unwrap().blocks() * BLOCK
    }

    fn write_file(path: &Path, len: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7_u8; len]).unwrap();
    }

    #[test]
    fn a_file_is_its_allocated_size() {
        let fx = Fixture::new();
        let file = fx.home().join("big.bin");
        write_file(&file, 100_000);

        let usage = disk_usage(&file);

        assert_eq!(usage.bytes, allocated(&file));
        assert!(usage.bytes >= 100_000);
        assert_eq!(usage.unreadable, 0);
    }

    #[test]
    fn a_folder_adds_up_everything_inside() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        write_file(&root.join("a.bin"), 50_000);
        write_file(&root.join("deep/er/b.bin"), 70_000);
        write_file(&root.join(".hidden"), 10_000);
        let expected = [
            root.clone(),
            root.join("a.bin"),
            root.join("deep"),
            root.join("deep/er"),
            root.join("deep/er/b.bin"),
            root.join(".hidden"),
        ]
        .iter()
        .map(|path| allocated(path))
        .sum::<u64>();

        assert_eq!(disk_usage(&root).bytes, expected);
    }

    #[test]
    fn a_missing_path_is_one_unreadable_entry() {
        let fx = Fixture::new();

        let usage = disk_usage(&fx.home().join("gone"));

        assert_eq!(
            usage,
            Usage {
                bytes: 0,
                unreadable: 1
            }
        );
    }

    #[test]
    fn a_symlink_counts_as_the_link_not_its_target() {
        let fx = Fixture::new();
        let big = fx.outside.join("big.bin");
        write_file(&big, 500_000);
        let root = fx.home().join("cache");
        fs::create_dir_all(&root).unwrap();
        symlink(&fx.outside, root.join("dir-link")).unwrap();
        symlink(&big, root.join("file-link")).unwrap();

        let usage = disk_usage(&root);

        assert!(
            usage.bytes < 100_000,
            "followed a link: {} bytes",
            usage.bytes
        );
        assert_eq!(
            disk_usage(&root.join("file-link")).bytes,
            allocated(&root.join("file-link"))
        );
    }

    #[test]
    fn hard_links_count_once() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        write_file(&root.join("one.bin"), 80_000);
        fs::hard_link(root.join("one.bin"), root.join("two.bin")).unwrap();
        let once = allocated(&root) + allocated(&root.join("one.bin"));

        assert_eq!(disk_usage(&root).bytes, once);
    }

    #[test]
    fn an_unreadable_folder_is_counted_and_reported() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        write_file(&root.join("readable.bin"), 40_000);
        write_file(&root.join("locked/inside.bin"), 40_000);
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(LOCKED)).unwrap();

        let expected = allocated(&root)
            + allocated(&root.join("readable.bin"))
            + allocated(&root.join("locked"));

        let usage = disk_usage(&root);

        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(OPEN)).unwrap();
        assert_eq!(usage.unreadable, 1);
        assert_eq!(usage.bytes, expected);
    }

    #[test]
    fn an_item_folder_that_cannot_be_opened_is_reported_not_zero() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        write_file(&root.join("inside.bin"), 1_000_000);
        fs::set_permissions(&root, fs::Permissions::from_mode(LOCKED)).unwrap();
        let expected = allocated(&root);

        let usage = disk_usage(&root);

        fs::set_permissions(&root, fs::Permissions::from_mode(OPEN)).unwrap();
        assert_eq!(
            usage,
            Usage {
                bytes: expected,
                unreadable: 1
            }
        );
    }

    #[test]
    fn folders_do_not_fill_the_hard_link_set() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        write_file(&root.join("a/b/c/file.bin"), 10_000);
        let expected = [
            root.clone(),
            root.join("a"),
            root.join("a/b"),
            root.join("a/b/c"),
            root.join("a/b/c/file.bin"),
        ]
        .iter()
        .map(|path| allocated(path))
        .sum::<u64>();

        assert_eq!(disk_usage(&root).bytes, expected);
    }

    #[test]
    #[ignore = "mounts a small disk image with hdiutil"]
    fn another_volume_is_not_entered() {
        let fx = Fixture::new();
        let root = fx.home().join("cache");
        let mount = root.join("mounted");
        fs::create_dir_all(&mount).unwrap();
        let _volume = MountedImage::attach(&fx.outside.join("volume.dmg"), &mount);
        write_file(&mount.join("other-volume.bin"), 1_000_000);

        let usage = disk_usage(&root);

        assert!(
            usage.bytes < 1_000_000,
            "counted the other volume: {}",
            usage.bytes
        );
    }
}
