use super::*;

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
fn starting_at_a_screen_opens_it_and_starts_what_it_needs() {
    let fx = Fixture::new();

    let (clean, clean_effects) = app(&fx, Mode::Trash).start_at(Screen::Clean);
    let (history, history_effects) = app(&fx, Mode::Trash).start_at(Screen::History);
    let (keep, keep_effects) = app(&fx, Mode::Trash).start_at(Screen::Keep);
    let (menu, menu_effects) = app(&fx, Mode::Trash).start_at(Screen::Menu);

    assert_eq!(
        (clean.screen, clean_effects),
        (Screen::Clean, vec![Effect::Scan(0)])
    );
    assert_eq!(
        (history.screen, history_effects),
        (Screen::History, vec![Effect::LoadHistory])
    );
    assert_eq!((keep.screen, keep_effects), (Screen::Keep, vec![]));
    assert_eq!((menu.screen, menu_effects), (Screen::Menu, vec![]));
}

#[test]
fn quit_from_the_menu_and_back_to_it_from_screens() {
    let fx = Fixture::new();
    let start = app(&fx, Mode::Trash);

    assert_eq!(start.on_key(Key::Char('q')).1, [Effect::Quit]);
    assert_eq!(start.on_key(Key::Char('6')).1, [Effect::Quit]);
    let (purge, _) = start.on_key(Key::Char('3'));
    assert_eq!(purge.screen, Screen::Purge);
    assert_eq!(purge.on_key(Key::Char('q')).0.screen, Screen::Menu);
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
        .on_event(Event::Scanned(0, crate::tui::testkit::report()))
        .0;
    let new = old
        .on_event(Event::Scanned(1, crate::tui::testkit::report()))
        .0;

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
            crate::tui::testkit::browse(),
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
    let message = after.message.unwrap();
    assert!(message.contains("Cleaning stopped"));
    assert!(message.contains(&fx.env.crash_log().display().to_string()));
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
