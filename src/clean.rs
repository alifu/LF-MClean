//! Removing the items a person picked, one by one, after checking the keep list again.
//!
//! One failure never stops the rest: each item gets its own outcome, which the summary shows.

use chrono::Local;

use crate::catalog::is_udid;
use crate::command::CommandRunner;
use crate::env::Env;
use crate::guard::{GuardError, SafePath};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::oplog::{LogEntry, OpLog};
use crate::remove::{RemoveError, Remover};
use crate::scan::{Item, Removal};

/// What happened to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Removed (or moved to the Trash); the space it took.
    Removed(u64),
    /// A dry run: it would have been removed.
    WouldRemove(u64),
    /// It was gone already, for instance inside a folder removed earlier in the same run.
    AlreadyGone,
    /// Not touched, because it is on the keep list.
    Kept,
    /// Removing it failed; part of it may be gone.
    Failed(String),
}

impl Outcome {
    /// The space this outcome freed (or, in a dry run, would free).
    #[must_use]
    pub fn freed_bytes(&self) -> u64 {
        match self {
            Self::Removed(bytes) | Self::WouldRemove(bytes) => *bytes,
            Self::AlreadyGone | Self::Kept | Self::Failed(_) => 0,
        }
    }
}

/// An item and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    /// The item.
    pub item: Item,
    /// What happened.
    pub outcome: Outcome,
}

/// Removes items in one `Mode`, logging every removal.
pub struct Cleaner<'a> {
    env: &'a Env,
    log: &'a OpLog,
    mode: Mode,
    runner: &'a dyn CommandRunner,
}

impl<'a> Cleaner<'a> {
    /// A cleaner for `env` that logs to `log` and runs `simctl` through `runner`.
    #[must_use]
    pub fn new(env: &'a Env, log: &'a OpLog, mode: Mode, runner: &'a dyn CommandRunner) -> Self {
        Self {
            env,
            log,
            mode,
            runner,
        }
    }

    /// Removes `items` in order, skipping every one `keep` keeps, and calls `on_done` after each.
    /// `keep` should be loaded just before: a keep added in another window then still counts.
    pub fn clean(
        &self,
        items: &[Item],
        keep: &KeepList,
        on_done: &mut dyn FnMut(&Cleaned),
    ) -> Vec<Cleaned> {
        items
            .iter()
            .map(|item| {
                let done = Cleaned {
                    item: item.clone(),
                    outcome: self.outcome(item, keep),
                };
                on_done(&done);
                done
            })
            .collect()
    }

    fn outcome(&self, item: &Item, keep: &KeepList) -> Outcome {
        if keep.keeps(item) {
            return Outcome::Kept;
        }
        let result = match &item.removal {
            Removal::Paths { main, with } => self.remove_paths(main, with, item.size_bytes),
            Removal::SimctlDelete { .. } => self.delete_simulators(item),
        };
        match result {
            Ok(Done::Removed) if self.mode == Mode::DryRun => Outcome::WouldRemove(item.size_bytes),
            Ok(Done::Removed) => Outcome::Removed(item.size_bytes),
            Ok(Done::AlreadyGone) => Outcome::AlreadyGone,
            Err(reason) => Outcome::Failed(reason),
        }
    }

    /// Removes the main entry, then what goes with it; a failed main entry leaves the rest.
    fn remove_paths(&self, main: &SafePath, with: &[SafePath], size: u64) -> Result<Done, String> {
        let remover = Remover::new(self.env, self.log, self.mode);
        let done = removed(remover.remove(main, size))?;
        for path in with {
            removed(remover.remove(path, 0))?;
        }
        Ok(done)
    }

    /// `simctl` removes each simulator for good, whatever the mode; a dry run runs nothing. Every
    /// simulator is tried even when one fails, and each id is checked again first: `simctl delete`
    /// also takes words such as `all`.
    fn delete_simulators(&self, item: &Item) -> Result<Done, String> {
        if self.mode == Mode::DryRun {
            return Ok(Done::Removed);
        }
        let failures: Vec<String> = item
            .removal
            .udids()
            .into_iter()
            .zip(item.removal.folders())
            .filter_map(|(udid, folder)| self.delete_simulator(udid, folder).err())
            .collect();
        if failures.is_empty() {
            Ok(Done::Removed)
        } else {
            Err(failures.join("; "))
        }
    }

    fn delete_simulator(&self, udid: &str, folder: &std::path::Path) -> Result<(), String> {
        if !is_udid(udid) {
            return Err(format!("{udid:?} is not a simulator id; nothing was run"));
        }
        let started = LogEntry::started(Local::now().fixed_offset(), folder, Mode::Permanent, 0);
        self.log.append(&started).map_err(|err| {
            format!("{udid} was not deleted: the log could not be written: {err}")
        })?;
        let result = self
            .runner
            .run("xcrun", &["simctl", "delete", udid])
            .map(drop)
            .map_err(|err| format!("simctl could not delete {udid}: {err}"));
        let finished = LogEntry::finished(
            Local::now().fixed_offset(),
            folder,
            Mode::Permanent,
            0,
            result.as_ref().err().cloned(),
        );
        let logged = self.log.append(&finished);
        result?;
        logged.map_err(|err| format!("{udid} was deleted, but the log could not be written: {err}"))
    }
}

enum Done {
    Removed,
    AlreadyGone,
}

/// A path that is gone already counts as done: a folder holding it may have been removed first.
fn removed(result: Result<(), RemoveError>) -> Result<Done, String> {
    match result {
        Ok(()) => Ok(Done::Removed),
        Err(RemoveError::Guard(GuardError::NotFound(_))) => Ok(Done::AlreadyGone),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::catalog::{self, Simulator};
    use crate::oplog::Stage;
    use crate::scan::Scanner;
    use crate::testing::{FakeRunner, Fixture};

    const NO_SIMULATORS: &str = r#"{"devices": {}}"#;

    fn scanned(fx: &Fixture, target: &str) -> Vec<Item> {
        let runner = FakeRunner::answering(NO_SIMULATORS);
        Scanner::new(&fx.env, &runner)
            .scan_target(catalog::find(target).unwrap())
            .items
    }

    fn clean(fx: &Fixture, mode: Mode, items: &[Item], keep: &KeepList) -> (Vec<Cleaned>, OpLog) {
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::answering("");
        let results = Cleaner::new(&fx.env, &log, mode, &runner).clean(items, keep, &mut |_| {});
        (results, log)
    }

    fn outcomes(results: &[Cleaned]) -> Vec<Outcome> {
        results.iter().map(|r| r.outcome.clone()).collect()
    }

    #[test]
    fn items_are_removed_and_their_space_counted() {
        let fx = Fixture::new();
        let a = fx.dir(".gradle/caches/modules-2");
        let b = fx.dir(".gradle/caches/jars-9");
        let items = scanned(&fx, "gradle.caches");

        let (results, _) = clean(&fx, Mode::Permanent, &items, &KeepList::empty(&fx.env));

        assert_eq!(
            outcomes(&results),
            items
                .iter()
                .map(|i| Outcome::Removed(i.size_bytes))
                .collect::<Vec<_>>()
        );
        assert!(!a.exists() && !b.exists());
        assert!(fx.outside_is_intact());
    }

    #[test]
    fn a_kept_item_is_left_alone_even_when_it_was_picked() {
        let fx = Fixture::new();
        let kept = fx.dir(".gradle/caches/modules-2");
        let items = scanned(&fx, "gradle.caches");
        let keep = KeepList::empty(&fx.env)
            .with_target("gradle.caches")
            .unwrap();

        let (results, log) = clean(&fx, Mode::Permanent, &items, &keep);

        assert_eq!(outcomes(&results), [Outcome::Kept]);
        assert!(kept.exists());
        assert_eq!(log.read_all().unwrap().entries, vec![]);
    }

    #[test]
    fn a_dry_run_removes_nothing_and_logs_nothing() {
        let fx = Fixture::new();
        let folder = fx.dir(".gradle/caches/modules-2");
        let items = scanned(&fx, "gradle.caches");

        let (results, log) = clean(&fx, Mode::DryRun, &items, &KeepList::empty(&fx.env));

        assert_eq!(
            outcomes(&results),
            [Outcome::WouldRemove(items[0].size_bytes)]
        );
        assert_eq!(results[0].outcome.freed_bytes(), items[0].size_bytes);
        assert!(folder.exists());
        assert_eq!(log.read_all().unwrap().entries, vec![]);
    }

    #[test]
    fn something_already_gone_is_not_a_failure() {
        let fx = Fixture::new();
        let folder = fx.dir(".gradle/caches/modules-2");
        let items = scanned(&fx, "gradle.caches");
        fs::remove_dir_all(&folder).unwrap();

        let (results, _) = clean(&fx, Mode::Permanent, &items, &KeepList::empty(&fx.env));

        assert_eq!(outcomes(&results), [Outcome::AlreadyGone]);
        assert_eq!(results[0].outcome.freed_bytes(), 0);
    }

    #[test]
    fn one_failure_does_not_stop_the_rest() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let stuck = fx.dir(".gradle/caches/a-stuck");
        fx.dir(".gradle/caches/b-fine");
        let items = scanned(&fx, "gradle.caches");
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o500)).unwrap();

        let (results, _) = clean(&fx, Mode::Permanent, &items, &KeepList::empty(&fx.env));

        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(results[0].outcome, Outcome::Failed(_)),
            "{results:?}"
        );
        assert_eq!(results[1].outcome, Outcome::Removed(items[1].size_bytes));
    }

    #[test]
    fn an_emulator_and_its_ini_go_together() {
        let fx = Fixture::new();
        let avd = fx.dir(".android/avd/Pixel.avd");
        let ini = fx.home().join(".android/avd/Pixel.ini");
        fs::write(&ini, "path.rel=avd/Pixel.avd\n").unwrap();
        let items = scanned(&fx, "android.avd");

        let (results, log) = clean(&fx, Mode::Permanent, &items, &KeepList::empty(&fx.env));

        assert!(matches!(results[0].outcome, Outcome::Removed(_)));
        assert!(!avd.exists() && !ini.exists());
        assert_eq!(
            log.read_all().unwrap().entries.len(),
            4,
            "started and finished for each"
        );
    }

    #[test]
    fn simulators_are_deleted_with_simctl_and_logged_as_permanent() {
        let fx = Fixture::new();
        let folder = fx
            .home()
            .join("Library/Developer/CoreSimulator/Devices/AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
        let item = Item {
            target: "xcode.sim-unavailable",
            name: "Unavailable simulators (1)".to_string(),
            removal: Removal::SimctlDelete {
                simulators: vec![Simulator {
                    udid: "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".to_string(),
                    folder: folder.clone(),
                }],
            },
            size_bytes: 10,
            partial: false,
            modified: None,
        };
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::answering("");

        let results = Cleaner::new(&fx.env, &log, Mode::Trash, &runner).clean(
            std::slice::from_ref(&item),
            &KeepList::empty(&fx.env),
            &mut |_| {},
        );

        assert_eq!(outcomes(&results), [Outcome::Removed(10)]);
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [[
                "xcrun",
                "simctl",
                "delete",
                "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"
            ]]
        );
        let entries = log.read_all().unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.mode == Mode::Permanent));
        assert_eq!(entries[1].stage, Stage::Finished);
        assert_eq!(entries[1].path, folder.display().to_string());
    }

    #[test]
    fn a_failing_simctl_is_a_failed_item_and_is_logged() {
        let fx = Fixture::new();
        let item = Item {
            target: "xcode.sim-unavailable",
            name: "Unavailable simulators (1)".to_string(),
            removal: Removal::SimctlDelete {
                simulators: vec![Simulator {
                    udid: "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".to_string(),
                    folder: fx.home().join("x/AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA"),
                }],
            },
            size_bytes: 10,
            partial: false,
            modified: None,
        };
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::failing("Invalid device");

        let results = Cleaner::new(&fx.env, &log, Mode::Permanent, &runner).clean(
            &[item],
            &KeepList::empty(&fx.env),
            &mut |_| {},
        );

        assert!(matches!(&results[0].outcome, Outcome::Failed(r) if r.contains("Invalid device")));
        let entries = log.read_all().unwrap().entries;
        assert!(
            entries[1]
                .error
                .as_deref()
                .unwrap()
                .contains("Invalid device")
        );
    }

    #[test]
    fn progress_is_reported_after_each_item() {
        let fx = Fixture::new();
        fx.dir(".gradle/caches/a");
        fx.dir(".gradle/caches/b");
        let items = scanned(&fx, "gradle.caches");
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::answering("");
        let seen = RefCell::new(Vec::new());

        Cleaner::new(&fx.env, &log, Mode::DryRun, &runner).clean(
            &items,
            &KeepList::empty(&fx.env),
            &mut |done| seen.borrow_mut().push(done.item.name.clone()),
        );

        assert_eq!(seen.into_inner(), ["a", "b"]);
    }

    fn simulators_item(fx: &Fixture, udids: &[&str]) -> Item {
        Item {
            target: "xcode.sim-unavailable",
            name: format!("Unavailable simulators ({})", udids.len()),
            removal: Removal::SimctlDelete {
                simulators: udids
                    .iter()
                    .map(|udid| Simulator {
                        udid: (*udid).to_string(),
                        folder: fx
                            .home()
                            .join("Library/Developer/CoreSimulator/Devices")
                            .join(udid),
                    })
                    .collect(),
            },
            size_bytes: 10,
            partial: false,
            modified: None,
        }
    }

    #[test]
    fn a_word_simctl_treats_specially_is_never_run() {
        let fx = Fixture::new();
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::answering("");

        let results = Cleaner::new(&fx.env, &log, Mode::Permanent, &runner).clean(
            &[simulators_item(&fx, &["all"])],
            &KeepList::empty(&fx.env),
            &mut |_| {},
        );

        assert!(
            matches!(&results[0].outcome, Outcome::Failed(r) if r.contains("not a simulator id"))
        );
        assert_eq!(runner.calls.lock().unwrap().len(), 0);
        assert_eq!(log.read_all().unwrap().entries, vec![]);
    }

    #[test]
    fn every_simulator_is_tried_even_after_one_fails() {
        let fx = Fixture::new();
        let log = OpLog::open(&fx.env).unwrap();
        let runner = FakeRunner::failing("Invalid device");
        let first = "11111111-1111-4111-8111-111111111111";
        let second = "22222222-2222-4222-8222-222222222222";

        let results = Cleaner::new(&fx.env, &log, Mode::Permanent, &runner).clean(
            &[simulators_item(&fx, &[first, second])],
            &KeepList::empty(&fx.env),
            &mut |_| {},
        );

        assert_eq!(runner.calls.lock().unwrap().len(), 2);
        let Outcome::Failed(reason) = &results[0].outcome else {
            panic!("{results:?}")
        };
        assert!(
            reason.contains(first) && reason.contains(second),
            "{reason}"
        );
    }
}
