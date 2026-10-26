//! Commands that run without the screens: `clean --ids` and `history`.

use std::io::Write;

use anyhow::{Context, Result, anyhow, bail};

use crate::catalog::{self, Safety, Target};
use crate::clean::{Cleaned, Cleaner, Outcome};
use crate::command::CommandRunner;
use crate::env::Env;
use crate::format::{human_size, items, printable};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::oplog::{LogContents, OpLog, Stage};
use crate::scan::{Item, Removal, Scanner};
use crate::system::{running_tools, tools_using};

/// What a `clean --ids` run was told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanOptions {
    /// The person agrees to the removal.
    pub yes: bool,
    /// The person agrees to remove risky targets too.
    pub allow_risky: bool,
}

/// Removes everything found for the targets `ids`, after checking them all.
///
/// Nothing is removed unless every id is a known target that is not kept, the keep list can be
/// read, and `--yes` was given (and `--allow-risky` for a risky target). A dry run needs no `--yes`.
///
/// # Errors
/// When a check fails, the log cannot be opened, the output fails, or any item failed to remove.
pub fn clean_ids(
    env: &Env,
    runner: &dyn CommandRunner,
    mode: Mode,
    ids: &[String],
    options: CleanOptions,
    out: &mut impl Write,
) -> Result<()> {
    let keep = KeepList::load(env).context("the keep list cannot be used; nothing was removed")?;
    let targets = known_targets(ids)?;
    refuse_kept(&targets, &keep)?;
    let scanner = Scanner::new(env, runner);
    let reports: Vec<_> = targets.iter().map(|t| scanner.scan_target(t)).collect();
    for skipped in reports.iter().flat_map(|r| &r.skipped) {
        eprintln!("not offered: {}", printable(&skipped.reason));
    }
    let found = distinct(reports.into_iter().flat_map(|r| r.items).collect());
    // Read again after the scan: a keep added meanwhile, in another window, counts.
    let keep = KeepList::load(env).context("the keep list cannot be used; nothing was removed")?;
    refuse_kept(&targets, &keep)?;
    if let Some(kept) = found.iter().find(|item| keep.keeps(item)) {
        bail!(
            "{} is kept by the keep list; nothing was removed",
            printable(&kept.name)
        );
    }
    print_plan(&found, mode, out)?;
    require_consent(&targets, mode, options)?;
    require_permanent_for_simulators(&found, mode)?;
    warn_about_running_tools(runner, &targets);
    if found.is_empty() {
        return Ok(());
    }
    let log = OpLog::open(env)?;
    let done = Cleaner::new(env, &log, mode, runner).clean(&found, &keep, &mut |_| {});
    print_outcomes(&done, mode, out)?;
    let failed = done
        .iter()
        .filter(|d| matches!(d.outcome, Outcome::Failed(_)))
        .count();
    if failed > 0 {
        bail!("{} failed", items(failed));
    }
    Ok(())
}

/// Items once each: overlapping targets (`DerivedData` holds `SourcePackages`) can find the same one.
fn distinct(found: Vec<Item>) -> Vec<Item> {
    let mut seen = std::collections::BTreeSet::new();
    found
        .into_iter()
        .filter(|item| seen.insert(item.key()))
        .collect()
}

/// `simctl` deletes simulators for good, so the Trash cannot be promised for them.
fn require_permanent_for_simulators(found: &[Item], mode: Mode) -> Result<()> {
    let simulators = found
        .iter()
        .any(|item| matches!(item.removal, Removal::SimctlDelete { .. }));
    if simulators && mode == Mode::Trash {
        bail!("simulators are deleted for good, not moved to the Trash; add --permanent");
    }
    Ok(())
}

/// The same warning the screens give, on stderr; it does not stop the run.
fn warn_about_running_tools(runner: &dyn CommandRunner, targets: &[&Target]) {
    let Ok(running) = running_tools(runner) else {
        return;
    };
    for target in targets {
        for tool in tools_using(&running, target.id) {
            eprintln!(
                "warning: {} is running and uses {}: {}.",
                tool.name(),
                target.name,
                tool.advice()
            );
        }
    }
}

fn known_targets(ids: &[String]) -> Result<Vec<&'static Target>> {
    let mut targets: Vec<&'static Target> = Vec::new();
    for id in ids {
        let target = catalog::find(id).ok_or_else(|| {
            let known: Vec<&str> = catalog::all_targets().map(|t| t.id).collect();
            anyhow!(
                "unknown cache {:?}; known: {}",
                printable(id),
                known.join(", ")
            )
        })?;
        if !targets.iter().any(|t| t.id == target.id) {
            targets.push(target);
        }
    }
    Ok(targets)
}

fn refuse_kept(targets: &[&Target], keep: &KeepList) -> Result<()> {
    let kept: Vec<&str> = targets
        .iter()
        .map(|t| t.id)
        .filter(|id| keep.targets().any(|kept| kept == *id))
        .collect();
    if kept.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} is on the keep list; nothing was removed",
            kept.join(", ")
        )
    }
}

fn require_consent(targets: &[&Target], mode: Mode, options: CleanOptions) -> Result<()> {
    if mode == Mode::DryRun {
        return Ok(());
    }
    if !options.yes {
        bail!("nothing was removed; add --yes to remove the items above");
    }
    let risky: Vec<&str> = targets
        .iter()
        .filter(|t| t.safety == Safety::Risky)
        .map(|t| t.id)
        .collect();
    if !risky.is_empty() && !options.allow_risky {
        bail!(
            "{} is risky; add --allow-risky as well as --yes",
            risky.join(", ")
        );
    }
    Ok(())
}

fn print_plan(found: &[Item], mode: Mode, out: &mut impl Write) -> Result<()> {
    if found.is_empty() {
        writeln!(out, "Nothing found to remove.")?;
        return Ok(());
    }
    let total: u64 = found.iter().map(|i| i.size_bytes).sum();
    writeln!(
        out,
        "{} ({}) {}:",
        items(found.len()),
        human_size(total),
        mode_phrase(mode)
    )?;
    for item in found {
        writeln!(
            out,
            "  {:>9}  {}",
            human_size(item.size_bytes),
            printable(&item.name)
        )?;
    }
    Ok(())
}

fn mode_phrase(mode: Mode) -> &'static str {
    match mode {
        Mode::DryRun => "found (dry run, nothing is removed)",
        Mode::Trash => "to move to the Trash",
        Mode::Permanent => "to remove for good",
    }
}

fn print_outcomes(done: &[Cleaned], mode: Mode, out: &mut impl Write) -> Result<()> {
    for d in done {
        let name = printable(&d.item.name);
        match &d.outcome {
            Outcome::Removed(bytes) => writeln!(out, "removed {name} ({})", human_size(*bytes))?,
            Outcome::WouldRemove(bytes) => {
                writeln!(out, "would remove {name} ({})", human_size(*bytes))?;
            }
            Outcome::AlreadyGone => writeln!(out, "already gone {name}")?,
            Outcome::Kept => writeln!(out, "kept {name}")?,
            Outcome::Failed(reason) => writeln!(out, "failed {name}: {}", printable(reason))?,
        }
    }
    let freed: u64 = done.iter().map(|d| d.outcome.freed_bytes()).sum();
    let verb = if mode == Mode::DryRun {
        "Would free"
    } else {
        "Freed"
    };
    writeln!(out, "{verb} {}", human_size(freed))?;
    Ok(())
}

/// Prints the removal log: one line per finished removal, newest first, or everything as JSON.
///
/// # Errors
/// When the log cannot be read or the output fails.
pub fn history(env: &Env, json: bool, out: &mut impl Write) -> Result<()> {
    let contents = OpLog::open(env)?.read_all()?;
    if json {
        writeln!(out, "{}", history_json(&contents)?)?;
    } else {
        write!(out, "{}", history_text(&contents))?;
    }
    Ok(())
}

fn history_json(contents: &LogContents) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "entries": contents.entries,
        "damaged_lines": contents.damaged_lines,
    }))?)
}

fn history_text(contents: &LogContents) -> String {
    let mut lines: Vec<String> = contents
        .entries
        .iter()
        .rev()
        .filter(|e| e.stage == Stage::Finished)
        .map(|entry| {
            let failure = entry
                .error
                .as_ref()
                .map_or_else(String::new, |err| format!("  FAILED: {}", printable(err)));
            format!(
                "{}  {:<9}  {:>9}  {}{failure}",
                entry.time.format("%Y-%m-%d %H:%M"),
                format!("{:?}", entry.mode).to_lowercase(),
                human_size(entry.size_bytes),
                printable(&entry.path)
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push("Nothing has been removed yet.".to_string());
    }
    match contents.damaged_lines.len() {
        0 => {}
        1 => lines.push("1 line of the log could not be read.".to_string()),
        n => lines.push(format!("{n} lines of the log could not be read.")),
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::oplog::LogEntry;

    fn finished(error: Option<&str>) -> LogEntry {
        let time = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 26, 9, 5, 0)
            .unwrap();
        LogEntry {
            stage: Stage::Finished,
            error: error.map(str::to_string),
            ..LogEntry::started(time, std::path::Path::new("/x/y"), Mode::Trash, 2_000_000)
        }
    }

    #[test]
    fn history_text_shows_failures_and_damaged_lines() {
        let contents = LogContents {
            entries: vec![finished(None), finished(Some("busy"))],
            damaged_lines: vec![3],
        };

        let text = history_text(&contents);

        assert_eq!(
            text,
            "2026-10-26 09:05  trash         2.0 MB  /x/y  FAILED: busy\n\
             2026-10-26 09:05  trash         2.0 MB  /x/y\n\
             1 line of the log could not be read.\n"
        );
    }

    #[test]
    fn several_damaged_lines_are_counted() {
        let contents = LogContents {
            entries: vec![],
            damaged_lines: vec![1, 2],
        };

        assert!(history_text(&contents).contains("2 lines of the log could not be read."));
    }
}
