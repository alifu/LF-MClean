//! The loop that draws the `App`, turns keys into new states and carries out their effects. It
//! works on any `ratatui` backend and any source of keys, so tests drive it without a terminal.

use std::fmt::Display;
use std::io;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use anyhow::{Result, anyhow};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};

use super::app::{App, Effect, Event, Key};
use super::{perform, view};
use crate::env::Env;

/// How long to wait for a key before looking for background events.
const TICK: Duration = Duration::from_millis(100);
/// Lines of the screen that are not body: the header and the footer.
const FRAME_LINES: u16 = 4;

/// Where the keys come from.
pub(super) trait KeySource {
    /// The next key, if one comes within `wait`.
    fn next_key(&mut self, wait: Duration) -> io::Result<Option<Key>>;
}

/// The keys of the real terminal.
pub(super) struct TerminalKeys;

impl KeySource for TerminalKeys {
    fn next_key(&mut self, wait: Duration) -> io::Result<Option<Key>> {
        if !event::poll(wait)? {
            return Ok(None);
        }
        Ok(match event::read()? {
            TermEvent::Key(press) if press.kind == KeyEventKind::Press => key_of(press),
            _ => None,
        })
    }
}

fn terminal_error(err: impl Display) -> anyhow::Error {
    anyhow!("the terminal cannot be used: {err}")
}

/// Draws, reads a key, applies background events and carries out effects, until `Effect::Quit`.
pub(super) fn event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    env: &Env,
    sender: &Sender<Event>,
    receiver: &Receiver<Event>,
    keys: &mut dyn KeySource,
) -> Result<()> {
    loop {
        let rows = terminal
            .size()
            .map_err(terminal_error)?
            .height
            .saturating_sub(FRAME_LINES);
        app = app.with_body_rows(usize::from(rows));
        terminal
            .draw(|frame| view::draw(frame, &app))
            .map_err(terminal_error)?;
        let mut effects = Vec::new();
        if let Some(key) = keys.next_key(TICK)? {
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
pub(super) fn key_of(press: KeyEvent) -> Option<Key> {
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
