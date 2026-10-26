//! Running other programs, behind a trait so tests can stand in for them.

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Longest a helper program may run; `simctl` normally answers in well under a second.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs a program and returns what it printed.
pub trait CommandRunner: Send + Sync {
    /// Runs `program` with `args` and returns its standard output.
    ///
    /// # Errors
    /// When the program cannot start, exits with an error or takes too long.
    fn run(&self, program: &str, args: &[&str]) -> io::Result<String>;
}

/// Runs real programs, giving up after a time limit.
#[derive(Debug, Clone, Copy)]
pub struct SystemRunner {
    timeout: Duration,
}

impl SystemRunner {
    /// A runner that stops a program after `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT)
    }
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<String> {
        // Its own process group, so a timeout can stop the programs it started as well.
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        // Both pipes are drained on their own threads so a chatty program cannot fill one and stall.
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let outcome = wait_for_exit(&mut child, deadline).and_then(|status| {
            Ok((
                status,
                receive(&stdout, deadline)?,
                receive(&stderr, deadline)?,
            ))
        });
        let (status, stdout, stderr) = match outcome {
            Ok(done) => done,
            Err(err) => {
                stop(&mut child);
                return Err(timed_out_or(err, self.timeout));
            }
        };
        if !status.success() {
            return Err(io::Error::other(format!(
                "{program} failed ({status}): {}",
                stderr.trim()
            )));
        }
        Ok(stdout)
    }
}

/// Reads a pipe to its end on its own thread; the text arrives through the channel.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<io::Result<String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = match pipe {
            Some(mut pipe) => pipe.read_to_end(&mut bytes).map(|_| ()),
            None => Ok(()),
        };
        // The receiver is gone when the caller gave up; there is nobody to tell.
        let _ = sender.send(read.map(|()| String::from_utf8_lossy(&bytes).into_owned()));
    });
    receiver
}

/// The text of one pipe, or a time-out error when it is still open at the deadline.
fn receive(reader: &Receiver<io::Result<String>>, deadline: Instant) -> io::Result<String> {
    let left = deadline.saturating_duration_since(Instant::now());
    match reader.recv_timeout(left) {
        Ok(text) => text,
        Err(RecvTimeoutError::Timeout) => Err(io::ErrorKind::TimedOut.into()),
        Err(RecvTimeoutError::Disconnected) => Err(io::Error::other("output reader failed")),
    }
}

fn wait_for_exit(child: &mut Child, deadline: Instant) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Kills the program and everything it started, then reaps it so it is not left as a zombie.
fn stop(child: &mut Child) {
    let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
    if let Some(group) = group {
        // The group may already be gone; either way the direct kill below covers the child.
        let _ = kill_process_group(group, Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn timed_out_or(err: io::Error, timeout: Duration) -> io::Error {
    if err.kind() != io::ErrorKind::TimedOut {
        return err;
    }
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("stopped after {:.1} seconds", timeout.as_secs_f64()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(timeout: Duration, program: &str, args: &[&str]) -> io::Result<String> {
        SystemRunner::new(timeout).run(program, args)
    }

    #[test]
    fn returns_what_the_program_printed() {
        let out = run(DEFAULT_TIMEOUT, "echo", &["hello", "world"]).unwrap();

        assert_eq!(out, "hello world\n");
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        let err = run(DEFAULT_TIMEOUT, "lfmclean-no-such-program", &[]).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_failing_program_reports_its_exit_status_and_message() {
        let err = run(DEFAULT_TIMEOUT, "sh", &["-c", "echo oops >&2; exit 3"]).unwrap_err();

        let message = err.to_string();
        assert!(message.contains("exit status: 3"), "{message}");
        assert!(message.contains("oops"), "{message}");
    }

    #[test]
    fn a_program_that_takes_too_long_is_stopped() {
        let started = Instant::now();

        let err = run(Duration::from_millis(100), "sleep", &["5"]).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn the_timeout_message_shows_fractions_of_a_second() {
        let err = run(Duration::from_millis(200), "sleep", &["5"]).unwrap_err();

        assert!(err.to_string().contains("0.2 seconds"), "{err}");
    }

    #[test]
    fn a_grandchild_holding_the_pipe_open_cannot_outlive_the_timeout() {
        let started = Instant::now();

        let err = run(
            Duration::from_millis(300),
            "sh",
            &["-c", "sleep 4 & echo hi"],
        )
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn output_that_is_not_utf8_is_read_lossily() {
        let out = run(DEFAULT_TIMEOUT, "printf", &["a\\377b"]).unwrap();

        assert_eq!(out, "a\u{fffd}b");
    }

    #[test]
    fn output_larger_than_a_pipe_buffer_is_not_a_deadlock() {
        let out = run(
            DEFAULT_TIMEOUT,
            "sh",
            &["-c", "head -c 300000 /dev/zero | tr '\\0' 'x'"],
        )
        .unwrap();

        assert_eq!(out.len(), 300_000);
    }
}
