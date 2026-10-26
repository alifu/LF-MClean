use super::app::{Effect, Event, Key, Loading, Screen};
use super::list::Tick;
use super::review::CONFIRM_WORD;
use super::testkit::{MB, app, item, now, on_clean, on_packages, press};
use crate::clean::{Cleaned, Outcome};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::packages::Usage;
use crate::testing::Fixture;
use crate::tui::app::App;

fn ticked_names(app: &App) -> Vec<String> {
    let list = if app.screen == Screen::Packages {
        &app.package_list
    } else {
        &app.clean_list
    };
    list.picked()
        .iter()
        .map(|(_, e)| e.item.name.clone())
        .collect()
}

fn word(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

// The menu.

#[test]
fn the_menu_moves_and_opens_with_numbers_or_enter() {
    let fx = Fixture::new();
    let start = app(&fx, Mode::Trash);

    let (moved, _) = press(&start, &[Key::Down, Key::Down, Key::Up]);
    let (clean, effects) = start.on_key(Key::Enter);
    let (packages, package_effects) = start.on_key(Key::Char('2'));

    assert_eq!(moved.menu_cursor, 1);
    assert_eq!(
        (clean.screen, effects),
        (Screen::Clean, vec![Effect::Scan(0)])
    );
    assert!(matches!(clean.scan, Loading::Running(_)));
    assert_eq!(
        (packages.screen, package_effects),
        (Screen::Packages, vec![Effect::ScanPackages(0)])
    );
}

#[test]
fn quit_from_the_menu_and_back_to_it_from_screens() {
    let fx = Fixture::new();
    let start = app(&fx, Mode::Trash);

    assert_eq!(start.on_key(Key::Char('q')).1, [Effect::Quit]);
    assert_eq!(start.on_key(Key::Char('6')).1, [Effect::Quit]);
    let (on_purge, _) = start.on_key(Key::Char('3'));
    assert_eq!(on_purge.screen, Screen::Purge);
    assert_eq!(on_purge.on_key(Key::Char('q')).0.screen, Screen::Menu);
    assert_eq!(start.on_key(Key::Char('9')).0.screen, Screen::Menu);
}

#[test]
fn ctrl_c_quits_from_any_screen() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    for app in [app(&fx, Mode::Trash), on_clean(&fx), cleaning] {
        let (_, effects) = app.on_key(Key::Interrupt);

        assert_eq!(effects, [Effect::Quit]);
    }
}

#[test]
fn scan_results_from_before_a_rescan_are_dropped() {
    let fx = Fixture::new();
    let (summary, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let summary = summary.on_event(Event::CleanFinished(None)).0;
    let (rescanning, _) = press(&summary, &[Key::Enter, Key::Char('1')]);

    let old = rescanning
        .on_event(Event::Scanning(0, "Gradle caches"))
        .0
        .on_event(Event::Scanned(0, super::testkit::report()))
        .0;
    let new = old.on_event(Event::Scanned(1, super::testkit::report())).0;

    assert_eq!(rescanning.scan_generation, 1);
    assert!(matches!(old.scan, Loading::Running(ref now) if now.is_empty()));
    assert!(matches!(new.scan, Loading::Done(_)));
}

#[test]
fn package_results_from_before_a_rescan_are_dropped() {
    let fx = Fixture::new();
    let (summary, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let summary = summary.on_event(Event::CleanFinished(None)).0;
    let (packages, _) = press(&summary, &[Key::Enter, Key::Char('2')]);

    let after = packages
        .on_event(Event::PackagesScanned(
            0,
            super::testkit::browse(),
            Usage::default(),
        ))
        .0;

    assert_eq!(packages.packages_generation, 1);
    assert!(matches!(after.packages, Loading::Running(_)));
}

#[test]
fn a_crash_while_cleaning_ends_on_the_summary_with_a_message() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (after, _) = cleaning.on_event(Event::Crashed("Cleaning".to_string()));

    assert_eq!(after.screen, Screen::Summary);
    assert!(after.message.unwrap().contains("Cleaning stopped"));
}

#[test]
fn a_crashed_scan_can_be_started_again() {
    let fx = Fixture::new();
    let (scanning, _) = app(&fx, Mode::Trash).on_key(Key::Char('1'));

    let (crashed, _) = scanning.on_event(Event::Crashed("The scan".to_string()));
    let (_, effects) = press(&crashed, &[Key::Esc, Key::Char('1')]);

    assert!(matches!(crashed.scan, Loading::Idle));
    assert_eq!(effects, [Effect::Scan(0)]);
}

#[test]
fn a_finished_scan_is_not_redone_when_coming_back() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (back, _) = clean.on_key(Key::Esc);
    let (again, effects) = back.on_key(Key::Char('1'));

    assert_eq!(effects, []);
    assert!(matches!(again.scan, Loading::Done(_)));
}

#[test]
fn history_is_read_each_time_it_opens() {
    let fx = Fixture::new();

    let (history, effects) = app(&fx, Mode::Trash).on_key(Key::Char('5'));

    assert_eq!(
        (history.screen, effects),
        (Screen::History, vec![Effect::LoadHistory])
    );
}

// Ticking.

#[test]
fn space_on_a_group_ticks_all_its_items_and_again_unticks_them() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (ticked, _) = press(&clean, &[Key::Char(' ')]);
    let (unticked, _) = press(&ticked, &[Key::Char(' ')]);

    assert_eq!(ticked_names(&ticked), ["App-abc", "Lib-def"]);
    assert_eq!(ticked_names(&unticked), Vec::<String>::new());
}

#[test]
fn an_open_group_ticks_one_item_at_a_time() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (app, _) = press(&clean, &[Key::Right, Key::Down, Key::Down, Key::Char(' ')]);

    assert_eq!(ticked_names(&app), ["Lib-def"]);
    let group = app.clean_list.group(0, 0);
    assert_eq!(app.clean_list.tick_of(group), Tick::Some);
    let (closed, _) = press(&app, &[Key::Left]);
    assert_eq!(
        closed.clean_list.current(),
        Some(super::list::Row::Group(0, 0))
    );
}

#[test]
fn all_safe_never_ticks_caution_or_risky_items() {
    let fx = Fixture::new();

    let (app, _) = press(&on_clean(&fx), &[Key::Char('a')]);

    assert_eq!(ticked_names(&app), ["App-abc", "Lib-def", "modules-2"]);
    let (none, _) = press(&app, &[Key::Char('n')]);
    assert_eq!(ticked_names(&none), Vec::<String>::new());
}

#[test]
fn the_cursor_skips_headings_and_stops_at_the_ends() {
    let fx = Fixture::new();
    let clean = on_clean(&fx);

    let (top, _) = press(&clean, &[Key::Up, Key::Up]);
    let (bottom, _) = press(&clean, &[Key::Down; 20]);

    assert_eq!(top.clean_list.cursor, 1, "row 0 is the Xcode heading");
    let rows = bottom.clean_list.rows();
    assert_eq!(bottom.clean_list.cursor, rows.len() - 1);
}

// Keeping.

#[test]
fn k_keeps_a_target_saves_it_and_unticks_its_items() {
    let fx = Fixture::new();
    let (ticked, _) = press(&on_clean(&fx), &[Key::Char(' ')]);

    let (kept, effects) = press(&ticked, &[Key::Char('k')]);

    let keep = kept.keep.clone().unwrap();
    assert_eq!(keep.targets().collect::<Vec<_>>(), ["xcode.derived-data"]);
    assert_eq!(effects, [Effect::SaveKeep(keep)]);
    assert_eq!(ticked_names(&kept), Vec::<String>::new());
    assert!(kept.clean_list.group(0, 0).all_kept());
}

#[test]
fn a_kept_group_cannot_be_ticked_and_says_why() {
    let fx = Fixture::new();
    let (kept, _) = press(&on_clean(&fx), &[Key::Char('k')]);

    let (after, _) = press(&kept, &[Key::Char(' ')]);
    let (all_safe, _) = press(&kept, &[Key::Char('a')]);

    assert_eq!(ticked_names(&after), Vec::<String>::new());
    assert!(after.message.as_deref().unwrap().contains("Kept"));
    assert_eq!(ticked_names(&all_safe), ["modules-2"]);
}

#[test]
fn k_again_stops_keeping() {
    let fx = Fixture::new();

    let (app, effects) = press(&on_clean(&fx), &[Key::Char('k'), Key::Char('k')]);

    assert!(app.keep.unwrap().is_empty());
    assert_eq!(effects.len(), 2);
}

#[test]
fn a_keep_list_loaded_at_start_marks_items() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env).with_target("android.avd").unwrap();
    let start = App::new(Mode::Trash, &fx.env, Ok(keep), None, now());

    let (app, _) = start.on_key(Key::Char('1'));
    let app = app.on_event(Event::Scanned(0, super::testkit::report())).0;

    let android = app
        .clean_list
        .sections
        .iter()
        .find(|s| s.title == "Android")
        .unwrap();
    assert!(android.groups[0].all_kept());
}

#[test]
fn a_broken_keep_list_blocks_keeping_and_removing() {
    let fx = Fixture::new();
    let start = App::new(
        Mode::Trash,
        &fx.env,
        Err("bad line 3".to_string()),
        None,
        now(),
    );
    let (app, _) = start.on_key(Key::Char('1'));
    let app = app.on_event(Event::Scanned(0, super::testkit::report())).0;

    let (kept, keep_effects) = press(&app, &[Key::Char('k')]);
    let (review, review_effects) = press(&app, &[Key::Char(' '), Key::Enter]);

    assert_eq!(keep_effects, []);
    assert!(kept.message.unwrap().contains("bad line 3"));
    assert_eq!(review_effects, []);
    assert_eq!(review.screen, Screen::Clean);
    assert!(review.message.unwrap().contains("keep list"));
}

// Reviewing and cleaning.

#[test]
fn enter_with_nothing_ticked_says_so() {
    let fx = Fixture::new();

    let (app, _) = press(&on_clean(&fx), &[Key::Enter]);

    assert_eq!(app.screen, Screen::Clean);
    assert!(app.message.unwrap().contains("Nothing is ticked"));
}

#[test]
fn safe_items_are_removed_after_y() {
    let fx = Fixture::new();
    let (review, _) = press(&on_clean(&fx), &[Key::Char('a'), Key::Enter]);

    let (cleaning, effects) = press(&review, &[Key::Char('y')]);

    assert_eq!(review.screen, Screen::Review);
    assert!(!review.review.as_ref().unwrap().needs_word());
    assert_eq!(cleaning.screen, Screen::Cleaning);
    let Effect::Clean(items) = &effects[0] else {
        panic!("{effects:?}")
    };
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["App-abc", "Lib-def", "modules-2"]);
}

#[test]
fn a_review_longer_than_the_screen_must_be_scrolled_before_y() {
    let fx = Fixture::new();
    // 11 body lines leave 2 for the 6 list lines (3 items, each with 1 folder), after the title,
    // the lines below the list and the simctl warning.
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let (refused, refused_effects) = press(&review, &[Key::Char('y')]);
    let (scrolled, _) = press(&refused, &[Key::Down, Key::Down, Key::Down, Key::Down]);
    let (cleaning, effects) = press(&scrolled, &[Key::Char('y')]);

    assert_eq!((refused.screen, refused_effects), (Screen::Review, vec![]));
    assert!(refused.review.as_ref().unwrap().note.is_some());
    assert!(scrolled.review.as_ref().unwrap().seen_all());
    assert_eq!(cleaning.screen, Screen::Cleaning);
    assert!(matches!(&effects[..], [Effect::Clean(items)] if items.len() == 3));
}

#[test]
fn space_pages_through_a_long_review() {
    let fx = Fixture::new();
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let (paged, _) = press(&review, &[Key::Char(' '), Key::Char(' ')]);

    let paged = paged.review.unwrap();
    assert_eq!(paged.scroll, 4);
    assert!(paged.seen_all());
}

#[test]
fn a_taller_screen_counts_the_newly_shown_lines_as_seen() {
    let fx = Fixture::new();
    let short = on_clean(&fx).with_body_rows(11);
    let (review, _) = press(&short, &[Key::Char('a'), Key::Enter]);

    let tall = review.clone().with_body_rows(40);

    assert!(!review.review.unwrap().seen_all());
    assert!(tall.review.unwrap().seen_all());
}

#[test]
fn risky_items_need_the_word_typed() {
    let fx = Fixture::new();
    // Down to Archives (risky), tick it.
    let (review, _) = press(&on_clean(&fx), &[Key::Down, Key::Char(' '), Key::Enter]);

    let (after_y, y_effects) = press(&review, &[Key::Char('y'), Key::Enter]);
    let (typo, typo_effects) = press(&review, &[word("delet"), vec![Key::Enter]].concat());
    let (fixed, effects) = press(
        &review,
        &[
            word("deletx"),
            vec![Key::Backspace, Key::Char('e'), Key::Enter],
        ]
        .concat(),
    );

    assert!(review.review.as_ref().unwrap().needs_word());
    assert_eq!((after_y.screen, y_effects), (Screen::Review, vec![]));
    assert_eq!((typo.screen, typo_effects), (Screen::Review, vec![]));
    assert_eq!(typo.review.unwrap().typed, "", "a wrong word is cleared");
    assert_eq!(
        fixed.screen,
        Screen::Cleaning,
        "{:?}",
        fixed.review.map(|r| r.typed)
    );
    assert!(matches!(&effects[..], [Effect::Clean(items)] if items.len() == 1));
    assert_eq!(CONFIRM_WORD, "delete");
}

#[test]
fn esc_on_review_goes_back_with_the_ticks() {
    let fx = Fixture::new();
    let (review, _) = press(&on_clean(&fx), &[Key::Char('a'), Key::Enter]);

    let (back, effects) = press(&review, &[Key::Esc]);

    assert_eq!((back.screen, effects), (Screen::Clean, vec![]));
    assert_eq!(ticked_names(&back).len(), 3);
}

#[test]
fn review_warns_about_running_tools_that_use_the_picked_caches() {
    let fx = Fixture::new();
    let clean = on_clean(&fx)
        .on_event(Event::Tools(Ok(vec![
            crate::system::Tool::Xcode,
            crate::system::Tool::Emulator,
        ])))
        .0;

    let (review, _) = press(&clean, &[Key::Char('a'), Key::Enter]);

    let warnings = review.review.unwrap().warnings;
    assert!(
        warnings
            .iter()
            .any(|w| w.starts_with("Xcode is running and uses DerivedData")),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("emulator")),
        "no emulator item is picked"
    );
}

#[test]
fn a_refused_clean_returns_to_review_with_the_reason() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (app, _) = cleaning.on_event(Event::CleanRefused("keep list broken".to_string()));

    assert_eq!(app.screen, Screen::Review);
    assert!(
        app.message
            .unwrap()
            .contains("Nothing was removed: keep list broken")
    );
}

#[test]
fn cleaning_progress_leads_to_the_summary_and_then_a_fresh_scan() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let done = Cleaned {
        item: item("gradle.caches", "modules-2", 5, 1),
        outcome: Outcome::Removed(5),
    };

    let (progress, _) = cleaning.on_event(Event::Cleaned(done));
    let (summary, _) = progress.on_event(Event::CleanFinished(Some(42_000 * MB)));
    let (menu, _) = summary.on_key(Key::Enter);
    let (clean_again, effects) = menu.on_key(Key::Char('1'));

    assert_eq!(progress.cleaning.as_ref().unwrap().done.len(), 1);
    assert_eq!(summary.screen, Screen::Summary);
    assert_eq!(
        summary.cleaning.as_ref().unwrap().free_after,
        Some(42_000 * MB)
    );
    assert_eq!(menu.screen, Screen::Menu);
    assert_eq!(effects, [Effect::Scan(1)]);
    assert_eq!(ticked_names(&clean_again), Vec::<String>::new());
}

#[test]
fn keys_do_nothing_while_cleaning() {
    let fx = Fixture::new();
    let (cleaning, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );

    let (still, effects) = press(&cleaning, &[Key::Esc, Key::Char('q'), Key::Enter]);

    assert_eq!((still.screen, effects), (Screen::Cleaning, vec![]));
}

// Packages.

#[test]
fn o_ticks_older_versions_that_no_project_uses() {
    let fx = Fixture::new();

    let (app, _) = press(&on_packages(&fx, Usage::default()), &[Key::Char('o')]);

    // 10.25.0 is older than the newest but a lock file names it.
    assert_eq!(ticked_names(&app), ["Firebase/10.20.0"]);
}

#[test]
fn u_ticks_unused_packages_only_when_every_project_was_read() {
    let fx = Fixture::new();
    let mut incomplete = Usage::default();
    incomplete
        .problems
        .push("Documents/Locked could not be searched".to_string());

    let (sure, _) = press(&on_packages(&fx, Usage::default()), &[Key::Char('u')]);
    let (unsure, _) = press(&on_packages(&fx, incomplete), &[Key::Char('u')]);

    assert_eq!(ticked_names(&sure), ["Alamofire/5.8.0"]);
    assert_eq!(ticked_names(&unsure), Vec::<String>::new());
    assert!(unsure.message.unwrap().contains("not certain"));
}

#[test]
fn k_on_a_package_keeps_every_version_and_on_a_version_only_it() {
    let fx = Fixture::new();
    let packages = on_packages(&fx, Usage::default());

    let (package, _) = press(&packages, &[Key::Down, Key::Char('k')]);
    let (version, _) = press(
        &packages,
        &[Key::Down, Key::Right, Key::Down, Key::Char('k')],
    );

    let written = |app: &App| {
        app.keep
            .clone()
            .unwrap()
            .packages()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(written(&package), ["pods:Firebase"]);
    // Made-up items carry no version in their path, so a version keep falls back to the item.
    assert_eq!(
        version.keep.clone().unwrap().targets().count() + written(&version).len(),
        1
    );
}

// The keep list screen.

#[test]
fn the_keep_screen_removes_marked_entries_on_enter() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .with_path("/Volumes/Work/keep")
        .unwrap();
    let start = App::new(Mode::Trash, &fx.env, Ok(keep), None, now());

    let (on_keep, _) = start.on_key(Key::Char('4'));
    let (saved, effects) = press(&on_keep, &[Key::Down, Key::Char(' '), Key::Enter]);

    let left = saved.keep.clone().unwrap();
    assert_eq!(left.targets().collect::<Vec<_>>(), ["xcode.archives"]);
    assert_eq!(left.paths().count(), 0);
    assert_eq!(effects, [Effect::SaveKeep(left)]);
    assert_eq!(saved.screen, Screen::Menu);
}

#[test]
fn leaving_the_keep_screen_without_marks_changes_nothing() {
    let fx = Fixture::new();
    let (on_keep, _) = app(&fx, Mode::Trash).on_key(Key::Char('4'));

    assert_eq!(press(&on_keep, &[Key::Enter]).1, []);
    assert_eq!(press(&on_keep, &[Key::Char(' '), Key::Esc]).1, []);
}

#[test]
fn a_failed_keep_save_is_shown() {
    let fx = Fixture::new();

    let (app, _) = app(&fx, Mode::Trash).on_event(Event::KeepSaved(Err("disk full".to_string())));

    assert!(app.message.unwrap().contains("disk full"));
}

// The removal step, run as the screens run it: the keep list is read from disk again first.

fn scanned_gradle(fx: &Fixture) -> Vec<crate::scan::Item> {
    let runner = crate::testing::FakeRunner::answering(r#"{"devices": {}}"#);
    crate::scan::Scanner::new(&fx.env, &runner)
        .scan_target(crate::catalog::find("gradle.caches").unwrap())
        .items
}

fn run_clean(fx: &Fixture, items: &[crate::scan::Item]) -> Vec<Event> {
    let events = std::cell::RefCell::new(Vec::new());
    super::clean(&fx.env, Mode::Permanent, items, &|event| {
        events.borrow_mut().push(event);
    });
    events.into_inner()
}

#[test]
fn a_keep_file_broken_since_the_screens_opened_removes_nothing() {
    let fx = Fixture::new();
    let folder = fx.dir(".gradle/caches/modules-2");
    let items = scanned_gradle(&fx);
    let keep_file = KeepList::file(&fx.env);
    std::fs::create_dir_all(keep_file.parent().unwrap()).unwrap();
    std::fs::write(&keep_file, "targets = [").unwrap();

    let events = run_clean(&fx, &items);

    assert!(
        matches!(&events[..], [Event::CleanRefused(reason)] if reason.contains("keep list")),
        "{events:?}"
    );
    assert!(folder.exists());
}

#[test]
fn a_keep_saved_since_the_screens_opened_is_honoured() {
    let fx = Fixture::new();
    let folder = fx.dir(".gradle/caches/modules-2");
    let items = scanned_gradle(&fx);
    KeepList::empty(&fx.env)
        .with_target("gradle.caches")
        .unwrap()
        .save(&fx.env)
        .unwrap();

    let events = run_clean(&fx, &items);

    assert!(
        matches!(&events[0], Event::Cleaned(done) if done.outcome == Outcome::Kept),
        "{events:?}"
    );
    assert!(matches!(events.last(), Some(Event::CleanFinished(_))));
    assert!(folder.exists());
}

#[test]
fn without_a_keep_the_picked_items_are_removed_and_reported() {
    let fx = Fixture::new();
    let folder = fx.dir(".gradle/caches/modules-2");
    let items = scanned_gradle(&fx);

    let events = run_clean(&fx, &items);

    assert!(
        matches!(&events[0], Event::Cleaned(done) if matches!(done.outcome, Outcome::Removed(_)))
    );
    assert!(matches!(events[1], Event::CleanFinished(Some(_))));
    assert!(!folder.exists());
    assert!(fx.outside_is_intact());
}

#[test]
fn a_dry_run_clean_reports_but_leaves_every_folder() {
    let fx = Fixture::new();
    let folder = fx.dir(".gradle/caches/modules-2");
    let items = scanned_gradle(&fx);
    let events = std::cell::RefCell::new(Vec::new());

    super::clean(&fx.env, Mode::DryRun, &items, &|event| {
        events.borrow_mut().push(event);
    });

    let events = events.into_inner();
    assert!(
        matches!(&events[0], Event::Cleaned(done) if matches!(done.outcome, Outcome::WouldRemove(_))),
        "{events:?}"
    );
    assert!(folder.exists());
}

// Effects, run as the event loop runs them, with results coming back over the channel.

fn performed(fx: &Fixture, effect: Effect) -> Event {
    let (sender, receiver) = std::sync::mpsc::channel();
    super::perform(effect, &fx.env, Mode::DryRun, &sender);
    receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .unwrap()
}

#[test]
fn the_save_keep_effect_writes_the_file() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap();

    let event = performed(&fx, Effect::SaveKeep(keep.clone()));

    assert!(matches!(event, Event::KeepSaved(Ok(()))));
    assert_eq!(KeepList::load(&fx.env).unwrap(), keep);
}

#[test]
fn the_history_effect_reads_the_log() {
    let fx = Fixture::new();

    let event = performed(&fx, Effect::LoadHistory);

    assert!(matches!(event, Event::History(Ok(contents)) if contents.entries.is_empty()));
}

#[test]
fn the_package_effect_scans_the_package_caches() {
    let fx = Fixture::new();
    fx.dir("Library/Caches/org.swift.swiftpm/repositories/SnapKit-a5691e5b");

    let event = performed(&fx, Effect::ScanPackages(0));

    let Event::PackagesScanned(0, found, usage) = event else {
        panic!("{event:?}")
    };
    assert_eq!(found.packages[0].name, "SnapKit");
    assert!(usage.is_complete());
}

#[test]
fn terminal_keys_become_screen_keys() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);

    assert_eq!(
        super::key_of(plain(KeyCode::Char(' '))),
        Some(Key::Char(' '))
    );
    assert_eq!(super::key_of(plain(KeyCode::Up)), Some(Key::Up));
    assert_eq!(
        super::key_of(plain(KeyCode::Backspace)),
        Some(Key::Backspace)
    );
    assert_eq!(super::key_of(plain(KeyCode::F(1))), None);
    assert_eq!(super::key_of(ctrl('c')), Some(Key::Interrupt));
    assert_eq!(super::key_of(ctrl('y')), None);
    assert_eq!(
        super::key_of(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT)),
        None
    );
    assert_eq!(
        super::key_of(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
        Some(Key::Char('Y'))
    );
}
