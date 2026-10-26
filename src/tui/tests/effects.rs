use super::*;

// The removal step, run as the screens run it: the keep list is read from disk again first.

fn scanned_gradle(fx: &Fixture) -> Vec<crate::scan::Item> {
    let runner = crate::testing::FakeRunner::answering(r#"{"devices": {}}"#);
    crate::scan::Scanner::new(&fx.env, &runner)
        .scan_target(crate::catalog::find("gradle.caches").unwrap())
        .items
}

pub(super) fn run_clean(fx: &Fixture, items: &[crate::scan::Item]) -> Vec<Event> {
    let events = std::cell::RefCell::new(Vec::new());
    crate::tui::clean(&fx.env, Mode::Permanent, items, &|event| {
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

    crate::tui::clean(&fx.env, Mode::DryRun, &items, &|event| {
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
    crate::tui::perform(effect, &fx.env, Mode::DryRun, &sender);
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
        crate::tui::event_loop::key_of(plain(KeyCode::Char(' '))),
        Some(Key::Char(' '))
    );
    assert_eq!(
        crate::tui::event_loop::key_of(plain(KeyCode::Up)),
        Some(Key::Up)
    );
    assert_eq!(
        crate::tui::event_loop::key_of(plain(KeyCode::Backspace)),
        Some(Key::Backspace)
    );
    assert_eq!(crate::tui::event_loop::key_of(plain(KeyCode::F(1))), None);
    assert_eq!(
        crate::tui::event_loop::key_of(ctrl('c')),
        Some(Key::Interrupt)
    );
    assert_eq!(crate::tui::event_loop::key_of(ctrl('y')), None);
    assert_eq!(
        crate::tui::event_loop::key_of(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT)),
        None
    );
    assert_eq!(
        crate::tui::event_loop::key_of(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT)),
        Some(Key::Char('Y'))
    );
}
