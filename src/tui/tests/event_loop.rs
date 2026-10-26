use std::collections::VecDeque;
use std::io;
use std::sync::mpsc;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
use crate::tui::event_loop::{KeySource, event_loop};

/// Keys read from a list; once it is empty the loop sees only quiet ticks.
struct Scripted(VecDeque<Key>);

impl KeySource for Scripted {
    fn next_key(&mut self, _wait: Duration) -> io::Result<Option<Key>> {
        Ok(self.0.pop_front())
    }
}

/// Runs the loop on `start` with these keys and returns what it ended with and what it drew last.
fn run_loop(
    fx: &Fixture,
    start: App,
    size: (u16, u16),
    keys: Vec<Key>,
) -> (anyhow::Result<()>, String) {
    let (sender, receiver) = mpsc::channel();
    let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
    let result = event_loop(
        &mut terminal,
        start,
        &fx.env,
        &sender,
        &receiver,
        &mut Scripted(keys.into()),
    );
    (result, terminal.backend().to_string())
}

#[test]
fn ctrl_c_ends_the_loop_from_the_menu() {
    let fx = Fixture::new();

    let (result, drawn) = run_loop(&fx, app(&fx, Mode::DryRun), (100, 24), vec![Key::Interrupt]);

    result.unwrap();
    assert!(drawn.contains("LF-MClean"));
}

#[test]
fn q_ends_the_loop_after_the_screen_changed() {
    let fx = Fixture::new();
    let keys = vec![Key::Char('4'), Key::Esc, Key::Char('q')];

    let (result, drawn) = run_loop(&fx, app(&fx, Mode::DryRun), (100, 24), keys);

    result.unwrap();
    assert!(drawn.contains("LF-MClean"));
}

#[test]
fn keep_changes_are_saved_in_the_loop_before_it_ends() {
    let fx = Fixture::new();
    let keys = vec![Key::Char('k'), Key::Interrupt];

    let (result, _) = run_loop(&fx, on_clean(&fx), (100, 24), keys);

    result.unwrap();
    let saved = KeepList::load(&fx.env).unwrap();
    assert_eq!(
        saved,
        KeepList::empty(&fx.env)
            .with_target("xcode.derived-data")
            .unwrap()
    );
}

#[test]
fn the_list_follows_the_height_of_the_terminal() {
    let fx = Fixture::new();
    let keys: Vec<Key> = std::iter::repeat_n(Key::Down, 12)
        .chain([Key::Interrupt])
        .collect();

    let (result, drawn) = run_loop(&fx, on_clean(&fx), (100, 12), keys);

    result.unwrap();
    assert!(drawn.contains("Kotlin/Native toolchains"), "{drawn}");
    assert!(!drawn.contains("DerivedData"), "{drawn}");
}

#[test]
fn a_background_event_is_applied_between_keys() {
    let fx = Fixture::new();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Event::KeepSaved(Err("disk full".to_string())))
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();

    event_loop(
        &mut terminal,
        app(&fx, Mode::DryRun),
        &fx.env,
        &sender,
        &receiver,
        &mut Scripted(VecDeque::from([Key::Char('x'), Key::Interrupt])),
    )
    .unwrap();

    assert!(terminal.backend().to_string().contains("disk full"));
}

struct Broken;

impl KeySource for Broken {
    fn next_key(&mut self, _wait: Duration) -> io::Result<Option<Key>> {
        Err(io::Error::other("the terminal went away"))
    }
}

#[test]
fn a_terminal_that_fails_ends_the_loop_with_an_error() {
    let fx = Fixture::new();
    let (sender, receiver) = mpsc::channel();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();

    let result = event_loop(
        &mut terminal,
        app(&fx, Mode::DryRun),
        &fx.env,
        &sender,
        &receiver,
        &mut Broken,
    );

    assert!(result.unwrap_err().to_string().contains("went away"));
}
