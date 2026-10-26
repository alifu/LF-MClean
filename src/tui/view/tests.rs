use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::draw;
use crate::clean::{Cleaned, Outcome};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::oplog::{LogContents, LogEntry};
use crate::packages::Usage;
use crate::testing::Fixture;
use crate::tui::app::{App, Event, Key};
use crate::tui::testkit::{MB, app, item, now, on_clean, on_packages, on_purge, press};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 24;

fn screen(app: &App) -> String {
    screen_of_height(app, HEIGHT)
}

/// The screen drawn `height` lines tall, as the event loop draws it: body rows set first.
fn screen_of_height(app: &App, height: u16) -> String {
    let app = app.clone().with_body_rows(usize::from(height - 4));
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, height)).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    terminal.backend().to_string()
}

#[test]
fn menu() {
    let fx = Fixture::new();

    insta::assert_snapshot!(screen(&app(&fx, Mode::Trash)));
}

#[test]
fn menu_with_a_broken_keep_list_in_permanent_mode() {
    let fx = Fixture::new();
    let app = App::new(
        Mode::Permanent,
        &fx.env,
        Err("line 2: unknown key `target`".to_string()),
        None,
        now(),
    );

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn clean_while_scanning() {
    let fx = Fixture::new();
    let (app, _) = app(&fx, Mode::Trash).on_key(Key::Char('1'));
    let (app, _) = app.on_event(Event::Scanning(app.scan_generation, "Gradle caches"));

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn clean_with_ticks_an_open_group_and_a_kept_target() {
    let fx = Fixture::new();
    let clean = on_clean(&fx)
        .on_event(Event::Tools(Ok(vec![crate::system::Tool::Xcode])))
        .0;
    let keys = [
        Key::Right,
        Key::Down,
        Key::Char(' '),
        Key::Down,
        Key::Down,
        Key::Down,
        Key::Char('k'),
        Key::Down,
        Key::Char(' '),
    ];

    let (app, _) = press(&clean, &keys);

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn packages_with_an_open_package() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_packages(&fx, Usage::default()),
        &[Key::Down, Key::Right, Key::Char('o')],
    );

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn review_longer_than_the_screen_after_y_too_soon() {
    let fx = Fixture::new();
    let short = on_clean(&fx).with_body_rows(11);
    let (app, _) = press(
        &short,
        &[Key::Char('a'), Key::Enter, Key::Down, Key::Char('y')],
    );

    insta::assert_snapshot!(screen_of_height(&app, 15));
}

#[test]
fn review_with_a_risky_item_and_a_typed_word() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_clean(&fx),
        &[
            Key::Char(' '),
            Key::Down,
            Key::Char(' '),
            Key::Enter,
            Key::Char('d'),
            Key::Char('e'),
        ],
    );

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn cleaning_half_way() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let done = Cleaned {
        item: item("xcode.derived-data", "App-abc", 1_500 * MB, 3),
        outcome: Outcome::Removed(1_500 * MB),
    };

    let (app, _) = app.on_event(Event::Cleaned(done));

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn summary_mole_style() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_clean(&fx),
        &[Key::Char('a'), Key::Enter, Key::Char('y')],
    );
    let results = [
        (
            item("xcode.derived-data", "App-abc", 1_500 * MB, 3),
            Outcome::Removed(1_500 * MB),
        ),
        (
            item("xcode.derived-data", "Lib-def", 400 * MB, 210),
            Outcome::Kept,
        ),
        (
            item("gradle.caches", "modules-2", 2_000 * MB, 1),
            Outcome::Failed("permission denied".to_string()),
        ),
    ];
    let app = results.into_iter().fold(app, |app, (item, outcome)| {
        app.on_event(Event::Cleaned(Cleaned { item, outcome })).0
    });

    let (app, _) = app.on_event(Event::CleanFinished(Some(42_700 * MB)));

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn history_newest_first() {
    let fx = Fixture::new();
    let (app, _) = app(&fx, Mode::Trash).on_key(Key::Char('5'));
    let entry = |path: &str, mode, size, error: Option<&str>| {
        LogEntry::finished(
            now(),
            std::path::Path::new(path),
            mode,
            size,
            error.map(str::to_string),
        )
    };
    let contents = LogContents {
        entries: vec![
            LogEntry::started(
                now(),
                std::path::Path::new("/Users/me/.gradle/caches/jars-9"),
                Mode::Trash,
                5 * MB,
            ),
            entry("/Users/me/.gradle/caches/jars-9", Mode::Trash, 5 * MB, None),
            entry(
                "/Users/me/.konan/dependencies",
                Mode::Permanent,
                900 * MB,
                Some("permission denied"),
            ),
        ],
        damaged_lines: vec![7],
    };

    let (app, _) = app.on_event(Event::History(Ok(contents)));

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn keep_list_with_a_marked_entry() {
    let fx = Fixture::new();
    let keep = KeepList::empty(&fx.env)
        .with_target("xcode.archives")
        .unwrap()
        .with_package("pods:Firebase@11.0.0".parse().unwrap())
        .with_path("/Volumes/Work/keep")
        .unwrap();
    let start = App::new(Mode::Trash, &fx.env, Ok(keep), Some(41_200 * MB), now());
    let (app, _) = press(&start, &[Key::Char('4'), Key::Down, Key::Char(' ')]);

    insta::assert_snapshot!(screen(&app));
}

#[test]
fn purge_while_searching() {
    let fx = Fixture::new();

    insta::assert_snapshot!(screen(&app(&fx, Mode::DryRun).on_key(Key::Char('3')).0));
}

#[test]
fn purge_lists_projects_with_one_opened() {
    let fx = Fixture::new();
    let (app, _) = press(
        &on_purge(&fx),
        &[Key::Down, Key::Right, Key::Down, Key::Char(' ')],
    );

    insta::assert_snapshot!(screen(&app));
}
