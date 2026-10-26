//! An append-only record of every removal, one JSON object per line, read by `lfmclean history`.
//! Each removal writes a `started` line before it begins and a `finished` line after, so an
//! interrupted removal still leaves a trace.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::mode::Mode;

const LOG_FILE: &str = "clean.log";

/// Which of the two lines of a removal an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Written before the removal begins.
    Started,
    /// Written after it ends, with its error if it failed.
    Finished,
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// When the line was written, with the local UTC offset.
    pub time: DateTime<FixedOffset>,
    /// Before or after the removal.
    pub stage: Stage,
    /// Trash or permanent.
    pub mode: Mode,
    /// The removed entry. APFS only allows UTF-8 names, so nothing is lost as a string.
    pub path: String,
    /// The size the user was shown.
    pub size_bytes: u64,
    /// Set on a `finished` line when the removal failed, possibly part way.
    pub error: Option<String>,
}

impl LogEntry {
    /// The line written before a removal.
    #[must_use]
    pub fn started(time: DateTime<FixedOffset>, path: &Path, mode: Mode, size_bytes: u64) -> Self {
        Self {
            time,
            stage: Stage::Started,
            mode,
            path: path.to_string_lossy().into_owned(),
            size_bytes,
            error: None,
        }
    }

    /// The line written after a removal; `error` is `None` when it worked.
    #[must_use]
    pub fn finished(
        time: DateTime<FixedOffset>,
        path: &Path,
        mode: Mode,
        size_bytes: u64,
        error: Option<String>,
    ) -> Self {
        Self {
            stage: Stage::Finished,
            error,
            ..Self::started(time, path, mode, size_bytes)
        }
    }
}

/// The readable entries, oldest first, and the line numbers that could not be read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LogContents {
    /// Entries that could be read, oldest first.
    pub entries: Vec<LogEntry>,
    /// 1-based numbers of lines that could not be read.
    pub damaged_lines: Vec<usize>,
}

/// The removal log at `~/Library/Logs/lfmclean/clean.log`.
pub struct OpLog {
    file: PathBuf,
}

impl OpLog {
    /// Creates the log folder and checks the log can be written.
    ///
    /// # Errors
    /// When the folder cannot be made or the file cannot be opened for appending.
    pub fn open(env: &Env) -> io::Result<Self> {
        let dir = env.log_dir();
        fs::create_dir_all(&dir)?;
        let log = Self {
            file: dir.join(LOG_FILE),
        };
        log.open_for_append()?;
        Ok(log)
    }

    /// Where the log file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Writes the entry as one line in a single write, so parallel runs do not mix lines.
    ///
    /// # Errors
    /// When the file cannot be opened or written.
    pub fn append(&self, entry: &LogEntry) -> io::Result<()> {
        let mut line = serde_json::to_string(entry).map_err(io::Error::other)?;
        line.push('\n');
        self.open_for_append()?.write_all(line.as_bytes())
    }

    /// A missing log is empty; damaged lines are skipped and reported by number.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn read_all(&self) -> io::Result<LogContents> {
        let text = match fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(LogContents::default()),
            Err(err) => return Err(err),
        };
        let (entries, damaged_lines) = text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .fold(
                (Vec::new(), Vec::new()),
                |(mut entries, mut damaged), (index, line)| {
                    match serde_json::from_str(line) {
                        Ok(entry) => entries.push(entry),
                        Err(_) => damaged.push(index + 1),
                    }
                    (entries, damaged)
                },
            );
        Ok(LogContents {
            entries,
            damaged_lines,
        })
    }

    fn open_for_append(&self) -> io::Result<fs::File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::testing::Fixture;

    fn noon() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(7 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 26, 12, 0, 0)
            .unwrap()
    }

    #[test]
    fn open_creates_the_log_folder() {
        let fx = Fixture::new();

        let log = OpLog::open(&fx.env).unwrap();

        assert!(log.path().starts_with(fx.env.log_dir()));
        assert!(fx.env.log_dir().is_dir());
    }

    #[test]
    fn open_fails_when_the_log_folder_cannot_be_made() {
        let fx = Fixture::new();
        fx.file("Library/Logs");

        assert!(OpLog::open(&fx.env).is_err());
    }

    #[test]
    fn entries_read_back_in_order() {
        let fx = Fixture::new();
        let log = OpLog::open(&fx.env).unwrap();
        let first = LogEntry::started(noon(), Path::new("/a"), Mode::Trash, 10);
        let second = LogEntry::finished(
            noon(),
            Path::new("/b with space"),
            Mode::Permanent,
            20,
            Some("busy".into()),
        );

        log.append(&first).unwrap();
        log.append(&second).unwrap();

        let contents = log.read_all().unwrap();
        assert_eq!(contents.entries, vec![first, second]);
        assert_eq!(contents.damaged_lines, Vec::<usize>::new());
    }

    #[test]
    fn missing_log_reads_as_empty() {
        let fx = Fixture::new();
        let log = OpLog::open(&fx.env).unwrap();
        fs::remove_file(log.path()).unwrap();

        assert_eq!(log.read_all().unwrap(), LogContents::default());
    }

    #[test]
    fn damaged_lines_are_skipped_and_reported() {
        let fx = Fixture::new();
        let log = OpLog::open(&fx.env).unwrap();
        let entry = LogEntry::started(noon(), Path::new("/a"), Mode::Trash, 10);
        log.append(&entry).unwrap();
        fs::write(
            log.path(),
            format!("{}not json\n", fs::read_to_string(log.path()).unwrap()),
        )
        .unwrap();
        log.append(&entry).unwrap();

        let contents = log.read_all().unwrap();

        assert_eq!(contents.entries.len(), 2);
        assert_eq!(contents.damaged_lines, vec![2]);
    }
}
