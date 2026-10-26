//! The command line: arguments and what each command does.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use chrono::Local;
use clap::{CommandFactory, Parser, Subcommand};

use crate::catalog::Target;
use crate::command::SystemRunner;
use crate::env::Env;
use crate::mode::Mode;
use crate::scan::{Scanner, render};
use crate::tui;

const CLEAR_LINE: &str = "\r\x1b[K";

/// Interactive cleaner for the caches of Xcode, Android, Gradle and Kotlin.
#[derive(Debug, Parser)]
#[command(name = "lfmclean", version, about)]
pub struct Cli {
    /// What to do; with none, the menu opens (or, outside a terminal, the help is shown).
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Show what would be removed, and remove nothing.
    #[arg(long, global = true, conflicts_with = "permanent")]
    pub dry_run: bool,
    /// Remove for good instead of moving to the Trash.
    #[arg(long, global = true)]
    pub permanent: bool,
}

impl Cli {
    /// How removal works for these arguments: the Trash unless asked otherwise.
    #[must_use]
    pub fn mode(&self) -> Mode {
        if self.dry_run {
            Mode::DryRun
        } else if self.permanent {
            Mode::Permanent
        } else {
            Mode::Trash
        }
    }
}

/// The commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the caches found and how much space they take. Nothing is removed.
    Scan {
        /// Print everything as JSON, with the paths of every item.
        #[arg(long, conflicts_with = "items")]
        json: bool,
        /// List each item under its cache.
        #[arg(long)]
        items: bool,
    },
}

/// Runs the command, writing its result to `out`.
///
/// # Errors
/// When the environment is unusable (running as root, no home folder) or the output fails.
pub fn run(cli: &Cli, out: &mut impl Write) -> Result<()> {
    match &cli.command {
        Some(Command::Scan { json, items }) => scan(*json, *items, out),
        None if io::stdout().is_terminal() && io::stdin().is_terminal() => {
            tui::run(&Env::from_system()?, cli.mode())
        }
        None => {
            Cli::command().write_help(out)?;
            writeln!(out)?;
            Ok(())
        }
    }
}

fn scan(json: bool, items: bool, out: &mut impl Write) -> Result<()> {
    let env = Env::from_system()?;
    let runner = SystemRunner::default();
    let progress = io::stderr().is_terminal();
    let report = Scanner::new(&env, &runner).scan_all(&mut |target: &Target| {
        if progress {
            eprint!("{CLEAR_LINE}Scanning {}...", target.name);
        }
    });
    if progress {
        eprint!("{CLEAR_LINE}");
    }
    if json {
        writeln!(out, "{}", render::json(&report)?)?;
    } else {
        write!(
            out,
            "{}",
            render::table(&report, Local::now().fixed_offset(), items)
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trash_is_the_default_and_the_flags_change_it() {
        let mode = |args: &[&str]| Cli::try_parse_from(args).unwrap().mode();

        assert_eq!(mode(&["lfmclean"]), Mode::Trash);
        assert_eq!(mode(&["lfmclean", "--dry-run"]), Mode::DryRun);
        assert_eq!(mode(&["lfmclean", "--permanent"]), Mode::Permanent);
        assert_eq!(mode(&["lfmclean", "scan", "--dry-run"]), Mode::DryRun);
    }

    #[test]
    fn dry_run_and_permanent_cannot_be_combined() {
        assert!(Cli::try_parse_from(["lfmclean", "--dry-run", "--permanent"]).is_err());
    }
}
