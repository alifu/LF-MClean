//! A temporary home folder for tests, with an `outside` folder that must never be touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use tempfile::TempDir;

use crate::command::CommandRunner;
use crate::env::{Env, current_uid};

pub const PRECIOUS_FILE: &str = "precious.txt";

pub struct Fixture {
    _tmp: TempDir,
    pub outside: PathBuf,
    pub env: Env,
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join(PRECIOUS_FILE), "keep me").unwrap();
        Self {
            env: Env::new(&home, current_uid()).unwrap(),
            outside,
            _tmp: tmp,
        }
    }

    /// The same home, seen as a different user.
    pub fn other_user_env(&self) -> Env {
        Env::unchecked(self.home(), current_uid() + 1)
    }

    pub fn home(&self) -> &Path {
        self.env.home()
    }

    /// Creates `rel` (a folder with one file) under home and returns its path.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.home().join(rel);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("data.bin"), "cache").unwrap();
        path
    }

    pub fn file(&self, rel: &str) -> PathBuf {
        let path = self.home().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "secret").unwrap();
        path
    }

    pub fn outside_is_intact(&self) -> bool {
        fs::read_to_string(self.outside.join(PRECIOUS_FILE)).is_ok_and(|text| text == "keep me")
    }
}

/// A small APFS disk image mounted at a chosen folder; detached again when dropped.
pub struct MountedImage {
    mount: PathBuf,
}

impl MountedImage {
    /// Creates `image` (4 MB) and mounts it at `mount`, which must exist. Needs `hdiutil`.
    pub fn attach(image: &Path, mount: &Path) -> Self {
        hdiutil(&[
            "create",
            "-size",
            "4m",
            "-fs",
            "APFS",
            "-volname",
            "lfmclean-test",
            "-quiet",
            &image.to_string_lossy(),
        ]);
        hdiutil(&[
            "attach",
            &image.to_string_lossy(),
            "-mountpoint",
            &mount.to_string_lossy(),
            "-nobrowse",
            "-quiet",
        ]);
        Self {
            mount: mount.to_path_buf(),
        }
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-force", "-quiet"])
            .arg(&self.mount)
            .status();
    }
}

fn hdiutil(args: &[&str]) {
    let status = Command::new("hdiutil").args(args).status().unwrap();
    assert!(status.success(), "hdiutil {args:?} failed");
}

/// A stand-in for other programs: always gives the same answer, and remembers how it was called.
pub struct FakeRunner {
    answer: io::Result<String>,
    /// Each call as `[program, args...]`.
    pub calls: Mutex<Vec<Vec<String>>>,
}

impl FakeRunner {
    pub fn answering(text: &str) -> Self {
        Self {
            answer: Ok(text.to_string()),
            calls: Mutex::default(),
        }
    }

    pub fn failing(message: &str) -> Self {
        Self {
            answer: Err(io::Error::other(message.to_string())),
            calls: Mutex::default(),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<String> {
        let mut call = vec![program.to_string()];
        call.extend(args.iter().map(ToString::to_string));
        self.calls.lock().unwrap().push(call);
        match &self.answer {
            Ok(text) => Ok(text.clone()),
            Err(err) => Err(io::Error::new(err.kind(), err.to_string())),
        }
    }
}
