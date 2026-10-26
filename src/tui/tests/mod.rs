//! Screen and effect tests, one file per part of the interface.

use crate::clean::{Cleaned, Outcome};
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::packages::Usage;
use crate::testing::Fixture;
use crate::tui::app::App;
use crate::tui::app::{Effect, Event, Key, Loading, Screen};
use crate::tui::list::Tick;
use crate::tui::review::CONFIRM_WORD;
use crate::tui::testkit::{MB, app, item, now, on_clean, on_packages, on_purge, press};

fn ticked_names(app: &App) -> Vec<String> {
    app.list()
        .picked()
        .iter()
        .map(|(_, e)| e.item.name.clone())
        .collect()
}

fn word(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

mod effects;
mod event_loop;
mod keep_screen;
mod lists;
mod menu;
mod packages;
mod purge;
mod review;
