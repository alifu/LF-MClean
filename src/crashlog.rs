//! What a background thread's panic said. The screens cannot print it without corrupting the
//! terminal, so it is appended to `crash.log` beside the removal log.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use chrono::Local;
use rustix::fs::OFlags;

use crate::env::Env;

/// Past this size the log is left as it is: the first crashes are the useful ones.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Panic text may name paths of the home folder; only the user reads it.
const LOG_MODE: u32 = 0o600;

/// Appends one entry: when, which thread, and the panic message with its place in the code.
///
/// # Errors
/// When the folder cannot be made or the file cannot be written.
pub fn record(env: &Env, thread: &str, panic: &str) -> io::Result<()> {
    let file = env.crash_log();
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    write_entry(&file, thread, panic)
}

/// Never through a symlink: the log is opened as the file itself or not at all.
fn write_entry(file: &Path, thread: &str, panic: &str) -> io::Result<()> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(LOG_MODE)
        .custom_flags(OFlags::NOFOLLOW.bits().cast_signed())
        .open(file)?;
    if log.metadata()?.len() >= MAX_LOG_BYTES {
        return Ok(());
    }
    let entry = format!(
        "{} thread '{thread}' {panic}\n\n",
        Local::now().to_rfc3339()
    );
    log.write_all(entry.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fixture;

    #[test]
    fn a_panic_is_appended_with_its_thread() {
        let fx = Fixture::new();

        record(&fx.env, "worker", "panicked at src/x.rs:1:2:\nboom").unwrap();
        record(&fx.env, "other", "panicked at src/y.rs:3:4:\nbang").unwrap();

        let text = fs::read_to_string(fx.env.crash_log()).unwrap();
        assert!(text.contains("thread 'worker' panicked at src/x.rs:1:2:\nboom\n"));
        assert!(text.contains("thread 'other' panicked at src/y.rs:3:4:\nbang\n"));
    }

    #[test]
    fn the_log_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();

        record(&fx.env, "worker", "boom").unwrap();

        let mode = fs::metadata(fx.env.crash_log())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_log_that_is_a_symlink_is_not_written_through() {
        let fx = Fixture::new();
        let elsewhere = fx.outside.join("elsewhere.log");
        fs::write(&elsewhere, "").unwrap();
        fs::create_dir_all(fx.env.log_dir()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, fx.env.crash_log()).unwrap();

        assert!(record(&fx.env, "worker", "boom").is_err());
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "");
    }

    #[test]
    fn a_full_log_is_not_written_to() {
        let fx = Fixture::new();
        fs::create_dir_all(fx.env.log_dir()).unwrap();
        let full = "x".repeat(usize::try_from(MAX_LOG_BYTES).unwrap());
        fs::write(fx.env.crash_log(), &full).unwrap();

        record(&fx.env, "worker", "boom").unwrap();

        assert_eq!(fs::read_to_string(fx.env.crash_log()).unwrap(), full);
    }

    #[test]
    fn a_log_folder_that_cannot_be_made_is_an_error() {
        let fx = Fixture::new();
        fx.file("Library/Logs");

        assert!(record(&fx.env, "worker", "boom").is_err());
    }
}
