//! A temporary home folder for tests, with an `outside` folder that must never be touched.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

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
