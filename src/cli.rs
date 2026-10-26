//! The command line: arguments and what each command does.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use chrono::Local;
use clap::{CommandFactory, Parser, Subcommand};

use crate::catalog::Target;
use crate::command::SystemRunner;
use crate::env::Env;
use crate::scan::{Scanner, render};

const CLEAR_LINE: &str = "\r\x1b[K";

/// Interactive cleaner for the caches of Xcode, Android, Gradle and Kotlin.
#[derive(Debug, Parser)]
#[command(name = "lfmclean", version, about)]
pub struct Cli {
    /// What to do; with none, the help is shown.
    #[command(subcommand)]
    pub command: Option<Command>,
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
