//! What a background thread's panic said. The screens cannot print it without corrupting the
//! terminal, so it is appended to `crash.log` beside the removal log.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use chrono::Local;

use crate::env::Env;

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

fn write_entry(file: &Path, thread: &str, panic: &str) -> io::Result<()> {
    let entry = format!(
        "{} thread '{thread}' {panic}\n\n",
        Local::now().to_rfc3339()
    );
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(entry.as_bytes())
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
    fn a_log_folder_that_cannot_be_made_is_an_error() {
        let fx = Fixture::new();
        fx.file("Library/Logs");

        assert!(record(&fx.env, "worker", "boom").is_err());
    }
}
