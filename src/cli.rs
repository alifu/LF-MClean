//! The command line: arguments and what each command does.

use std::io::{self, IsTerminal, Write};

use anyhow::{Result, bail};
use chrono::Local;
use clap::{CommandFactory, Parser, Subcommand};

use crate::catalog::Target;
use crate::command::SystemRunner;
use crate::env::Env;
use crate::headless::{self, CleanOptions};
use crate::mode::Mode;
use crate::purge;
use crate::scan::{Scanner, render};
use crate::tui::{self, Screen};

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
    /// Pick caches to remove. With `--ids`, remove them without the screens.
    Clean {
        /// Remove these caches (ids from `lfmclean scan --json`), separated by commas.
        #[arg(long, value_delimiter = ',', value_name = "ID,ID")]
        ids: Vec<String>,
        /// Agree to the removal; without it `--ids` only shows what it would remove.
        #[arg(long, requires = "ids")]
        yes: bool,
        /// Also agree to remove risky caches (emulators, system images, archives).
        #[arg(long, requires = "yes")]
        allow_risky: bool,
    },
    /// Browse `CocoaPods` and Swift packages to delete or keep.
    Packages,
    /// Find and remove the build folders of your projects.
    Purge {
        /// Search these folders (inside your home folder) instead of ~/Documents, ~/Developer
        /// and ~/Projects, separated by commas.
        #[arg(long, value_delimiter = ',', value_name = "FOLDER,FOLDER")]
        paths: Vec<String>,
        /// Print the projects found as JSON and remove nothing.
        #[arg(long)]
        json: bool,
    },
    /// Show and edit what is never removed.
    Keep,
    /// Show what was removed.
    History {
        /// Print the log as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Runs the command, writing its result to `out`.
///
/// # Errors
/// When the environment is unusable (running as root, no home folder) or the output fails.
pub fn run(cli: &Cli, out: &mut impl Write) -> Result<()> {
    match &cli.command {
        Some(Command::Scan { json, items }) => scan(*json, *items, out),
        Some(Command::Clean {
            ids,
            yes,
            allow_risky,
        }) if !ids.is_empty() => {
            let options = CleanOptions {
                yes: *yes,
                allow_risky: *allow_risky,
            };
            let runner = SystemRunner::default();
            headless::clean_ids(&Env::from_system()?, &runner, cli.mode(), ids, options, out)
        }
        Some(Command::Purge { paths, json: true }) => {
            let env = Env::from_system()?;
            headless::purge_json(&env, &folders_from(&env, paths)?, out)
        }
        Some(Command::History { json }) if *json || !is_interactive() => {
            headless::history(&Env::from_system()?, *json, out)
        }
        Some(command) => screen(command, cli.mode()),
        None if is_interactive() => tui::run(&Env::from_system()?, cli.mode(), Screen::Menu, None),
        None => {
            Cli::command().write_help(out)?;
            writeln!(out)?;
            Ok(())
        }
    }
}

fn folders_from(env: &Env, paths: &[String]) -> Result<Vec<std::path::PathBuf>> {
    if paths.is_empty() {
        return Ok(purge::default_folders(env));
    }
    purge::folders_from(env, paths).map_err(|reason| anyhow::anyhow!(reason))
}

fn is_interactive() -> bool {
    io::stdout().is_terminal() && io::stdin().is_terminal()
}

/// Opens the screen a command is named after; the screens need a terminal.
fn screen(command: &Command, mode: Mode) -> Result<()> {
    let screen = match command {
        Command::Scan { .. } => bail!("scan has no screen"),
        Command::Clean { .. } => Screen::Clean,
        Command::Packages => Screen::Packages,
        Command::Purge { .. } => Screen::Purge,
        Command::Keep => Screen::Keep,
        Command::History { .. } => Screen::History,
    };
    if !is_interactive() {
        bail!(
            "this command needs a terminal; for scripts use `scan --json`, `clean --ids`, `purge --json` or `history --json`"
        );
    }
    let env = Env::from_system()?;
    let folders = match command {
        Command::Purge { paths, .. } if !paths.is_empty() => Some(folders_from(&env, paths)?),
        _ => None,
    };
    tui::run(&env, mode, screen, folders)
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

    #[test]
    fn clean_ids_split_on_commas_and_need_yes_for_allow_risky() {
        let cli = Cli::try_parse_from(["lfmclean", "clean", "--ids", "a.b,c.d", "--yes"]).unwrap();

        let Some(Command::Clean {
            ids,
            yes,
            allow_risky,
        }) = cli.command
        else {
            panic!("{cli:?}")
        };
        assert_eq!(
            (ids, yes, allow_risky),
            (vec!["a.b".into(), "c.d".into()], true, false)
        );
        assert!(Cli::try_parse_from(["lfmclean", "clean", "--allow-risky"]).is_err());
        assert!(Cli::try_parse_from(["lfmclean", "clean", "--yes"]).is_err());
    }

    #[test]
    fn every_screen_has_a_subcommand() {
        for name in ["clean", "packages", "purge", "keep", "history"] {
            assert!(Cli::try_parse_from(["lfmclean", name]).is_ok(), "{name}");
        }
    }
}
