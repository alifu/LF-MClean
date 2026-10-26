//! The interactive screens: a loop that draws the `App`, turns keys into new states, and runs the
//! slow work (scans, removal) on background threads that report back with events.

pub mod app;
pub mod keep_screen;
mod keys;
pub mod list;
pub mod purge;
pub mod review;
pub mod sections;
mod view;

#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

pub use app::Screen;
use app::{App, Effect, Event, Key};

use crate::clean::Cleaner;
use crate::command::SystemRunner;
use crate::env::Env;
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::oplog::OpLog;
use crate::packages::{browse, find_usage};
use crate::scan::Scanner;
use crate::system::{free_space, running_tools};

/// How long to wait for a key before looking for background events.
const TICK: Duration = Duration::from_millis(100);
/// Lines of the screen that are not body: the header and the footer.
const FRAME_LINES: u16 = 4;

/// Runs the screens until the person quits.
///
/// # Errors
/// When the terminal cannot be used.
pub fn run(
    env: &Env,
    mode: Mode,
    start: Screen,
    purge_folders: Option<Vec<std::path::PathBuf>>,
) -> Result<()> {
    let keep = KeepList::load(env).map_err(|err| err.to_string());
    let app = App::new(
        mode,
        env,
        keep,
        free_space(env.home()).ok(),
        Local::now().fixed_offset(),
    );
    let app = match purge_folders {
        Some(folders) => app.with_purge_folders(folders),
        None => app,
    };
    let (app, first) = app.start_at(start);
    // Unbounded is fine: at most one event per target or removed item.
    let (sender, receiver) = mpsc::channel();
    spawn_tools_check(&sender);
    let mut terminal = ratatui::try_init()?;
    for effect in first {
        perform(effect, env, mode, &sender);
    }
    restore_only_for_main_thread_panics();
    let result = event_loop(&mut terminal, app, env, &sender, &receiver);
    ratatui::restore();
    result
}

/// `ratatui` restores the terminal on any panic. A background thread's panic is caught and shown
/// as an event while the screens keep running, so only a panic of the screens themselves may
/// restore the terminal and print.
fn restore_only_for_main_thread_panics() {
    let restore_and_report = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if thread::current().name() == Some("main") {
            restore_and_report(info);
        }
    }));
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    mut app: App,
    env: &Env,
    sender: &Sender<Event>,
    receiver: &Receiver<Event>,
) -> Result<()> {
    loop {
        let rows = terminal.size()?.height.saturating_sub(FRAME_LINES);
        app = app.with_body_rows(usize::from(rows));
        terminal.draw(|frame| view::draw(frame, &app))?;
        let mut effects = Vec::new();
        if event::poll(TICK)?
            && let TermEvent::Key(press) = event::read()?
            && press.kind == KeyEventKind::Press
            && let Some(key) = key_of(press)
        {
            let (next, more) = app.on_key(key);
            app = next;
            effects.extend(more);
        }
        while let Ok(event) = receiver.try_recv() {
            let (next, more) = app.on_event(event);
            app = next;
            effects.extend(more);
        }
        for effect in effects {
            match effect {
                Effect::Quit => return Ok(()),
                // Saved here, in order: two quick changes cannot land on disk the wrong way
                // round, and quitting right after cannot lose one.
                Effect::SaveKeep(keep) => {
                    let saved = keep.save(env).map_err(|e| e.to_string());
                    app = app.on_event(Event::KeepSaved(saved)).0;
                }
                other => perform(other, env, app.mode, sender),
            }
        }
    }
}

/// Plain keys only: Ctrl-C leaves, and other Ctrl or Alt chords are ignored, so Ctrl-Y cannot
/// confirm a removal.
fn key_of(press: KeyEvent) -> Option<Key> {
    let chord = press
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    if chord {
        return (press.modifiers.contains(KeyModifiers::CONTROL)
            && press.code == KeyCode::Char('c'))
        .then_some(Key::Interrupt);
    }
    Some(match press.code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}

/// Starts an effect on its own thread; its result comes back over `sender`. A panic there is
/// caught and sent as `Event::Crashed`, so no screen waits for ever.
fn perform(effect: Effect, env: &Env, mode: Mode, sender: &Sender<Event>) {
    let env = env.clone();
    let sender = sender.clone();
    thread::spawn(move || {
        // The receiver is gone only when the screens closed; nobody is left to tell.
        let send = |event| drop(sender.send(event));
        let what = describe(&effect);
        let ran = panic::catch_unwind(AssertUnwindSafe(|| run_effect(effect, &env, mode, &send)));
        if ran.is_err() {
            send(Event::Crashed(what.to_string()));
        }
    });
}

fn describe(effect: &Effect) -> &'static str {
    match effect {
        Effect::Scan(_) => "The scan",
        Effect::ScanPackages(_) => "The package scan",
        Effect::ScanPurge(..) => "The project search",
        Effect::SaveKeep(_) => "Saving the keep list",
        Effect::LoadHistory => "Reading the history",
        Effect::Clean(_) => "Cleaning",
        Effect::Quit => "Quitting",
    }
}

fn run_effect(effect: Effect, env: &Env, mode: Mode, send: &dyn Fn(Event)) {
    match effect {
        Effect::Scan(generation) => {
            let runner = SystemRunner::default();
            let report = Scanner::new(env, &runner)
                .scan_all(&mut |target| send(Event::Scanning(generation, target.name)));
            send(Event::Scanned(generation, report));
        }
        Effect::ScanPackages(generation) => {
            let runner = SystemRunner::default();
            let usage = find_usage(env);
            let found = browse(&Scanner::new(env, &runner), &usage);
            send(Event::PackagesScanned(generation, found, usage));
        }
        Effect::ScanPurge(generation, folders) => {
            send(Event::PurgeScanned(
                generation,
                crate::purge::find_projects(env, &folders),
            ));
        }
        Effect::SaveKeep(keep) => send(Event::KeepSaved(keep.save(env).map_err(|e| e.to_string()))),
        Effect::LoadHistory => {
            let contents = OpLog::open(env).and_then(|log| log.read_all());
            send(Event::History(contents.map_err(|e| e.to_string())));
        }
        Effect::Clean(items) => clean(env, mode, &items, send),
        Effect::Quit => {}
    }
}

/// Loads the keep list from disk again, so a keep saved a moment ago counts, then removes.
fn clean(env: &Env, mode: Mode, items: &[crate::scan::Item], send: &dyn Fn(Event)) {
    let keep = match KeepList::load(env) {
        Ok(keep) => keep,
        Err(err) => return send(Event::CleanRefused(err.to_string())),
    };
    let log = match OpLog::open(env) {
        Ok(log) => log,
        Err(err) => {
            return send(Event::CleanRefused(format!(
                "the log could not be opened: {err}"
            )));
        }
    };
    let runner = SystemRunner::default();
    Cleaner::new(env, &log, mode, &runner)
        .clean(items, &keep, &mut |done| send(Event::Cleaned(done.clone())));
    send(Event::CleanFinished(free_space(env.home()).ok()));
}

fn spawn_tools_check(sender: &Sender<Event>) {
    let sender = sender.clone();
    thread::spawn(move || {
        let tools = running_tools(&SystemRunner::default()).map_err(|e| e.to_string());
        drop(sender.send(Event::Tools(tools)));
    });
}
