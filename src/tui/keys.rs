//! What keys do on each screen. Split from `app` to keep both small; the state and the events
//! stay there.

use super::app::{App, Cleaning, Effect, Key, Loading, Screen};
use super::keep_screen::KeepScreen;
use super::list::{Group, ListState, Row};
use super::review::Review;
use super::sections::{clean_sections, package_sections};
use crate::catalog::Safety;
use crate::keep::KeepList;
use crate::scan::Item;

impl App {
    pub(super) fn list_key(self, key: Key) -> (Self, Vec<Effect>) {
        let list = self.list().clone();
        match key {
            Key::Up => (self.with_list(list.moved(-1)), Vec::new()),
            Key::Down => (self.with_list(list.moved(1)), Vec::new()),
            Key::Right => (self.with_list(list.opened(true)), Vec::new()),
            Key::Left => (self.with_list(list.opened(false)), Vec::new()),
            Key::Char(' ') => match list.toggled() {
                Some(next) => (self.with_list(next), Vec::new()),
                None if list.current().is_some() => {
                    self.noted("Kept: it cannot be ticked. Press k to stop keeping it.")
                }
                None => (self, Vec::new()),
            },
            Key::Char('a') => {
                let next = list.ticking(|group, entry| {
                    group.safety_of(entry) == Safety::Safe && !entry.in_use
                });
                (self.with_list(next), Vec::new())
            }
            Key::Char('n') => (self.with_list(list.cleared()), Vec::new()),
            Key::Char('o') if self.screen == Screen::Packages => {
                let next = list.ticking(is_older_version);
                (self.with_list(next), Vec::new())
            }
            Key::Char('u') if self.screen == Screen::Packages => self.tick_unused(),
            Key::Char('k') => self.toggle_keep(),
            Key::Enter => self.open_review(),
            Key::Esc | Key::Char('q') => (
                Self {
                    screen: Screen::Menu,
                    ..self
                },
                Vec::new(),
            ),
            _ => (self, Vec::new()),
        }
    }

    pub(super) fn tick_unused(self) -> (Self, Vec<Effect>) {
        let complete = match &self.packages {
            Loading::Done(found) => found.1.is_complete(),
            _ => false,
        };
        if !complete {
            return self.noted(
                "Some project folders could not be read, so \"unused\" is not certain; tick by hand.",
            );
        }
        let next = self.package_list.ticking(|group, _| group.unused);
        (self.with_list(next), Vec::new())
    }

    pub(super) fn toggle_keep(self) -> (Self, Vec<Effect>) {
        let keep = match self.keep.clone() {
            Ok(keep) => keep,
            Err(reason) => return self.noted(format!("The keep list cannot be changed: {reason}")),
        };
        let Some(keep_ref) = self.list().keep_ref().cloned() else {
            return (self, Vec::new());
        };
        match keep_ref.toggled(&keep) {
            Ok(next) => {
                let app = self.with_keep(&next);
                let still_kept = app
                    .list()
                    .current()
                    .is_some_and(|row| row_kept(app.list(), row));
                let note = if !keep_ref.is_in(&next) && still_kept {
                    Some("Still kept by another keep-list entry; see Keep list.".to_string())
                } else {
                    None
                };
                (
                    Self {
                        message: note,
                        ..app
                    },
                    vec![Effect::SaveKeep(next)],
                )
            }
            Err(reason) => self.noted(format!("Cannot keep it: {reason}")),
        }
    }

    /// The app with a new keep list, its lists marked again.
    #[must_use]
    pub fn with_keep(self, keep: &KeepList) -> Self {
        let mut app = Self {
            keep: Ok(keep.clone()),
            ..self
        };
        if let Loading::Done(report) = &app.scan {
            app.clean_list = app
                .clean_list
                .with_sections(clean_sections(report, keep, app.now));
        }
        if let Loading::Done(found) = &app.packages {
            let sections = package_sections(&found.0, &found.1, keep, app.now);
            app.package_list = app.package_list.with_sections(sections);
        }
        app.purge = app.purge.with_keep(keep, app.now);
        app
    }

    pub(super) fn open_review(self) -> (Self, Vec<Effect>) {
        if let Err(reason) = self.keep.clone() {
            return self.noted(format!(
                "Nothing can be removed until the keep list is fixed: {reason}"
            ));
        }
        let picked: Vec<(Item, Safety)> = self
            .list()
            .picked()
            .into_iter()
            .map(|(group, entry)| (entry.item.clone(), group.safety_of(entry)))
            .collect();
        if picked.is_empty() {
            return self.noted("Nothing is ticked. Space ticks the line under the cursor.");
        }
        let review = Review::new(picked, self.screen, &self.running, self.body_rows);
        (
            Self {
                review: Some(review),
                screen: Screen::Review,
                ..self
            },
            Vec::new(),
        )
    }

    pub(super) fn review_key(self, key: Key) -> (Self, Vec<Effect>) {
        let Some(review) = self.review.clone() else {
            return (
                Self {
                    screen: Screen::Menu,
                    ..self
                },
                Vec::new(),
            );
        };
        match review.on_key(key) {
            super::review::Answer::Wait(next) => (
                Self {
                    review: Some(next),
                    ..self
                },
                Vec::new(),
            ),
            super::review::Answer::Back => (
                Self {
                    screen: review.from,
                    review: None,
                    ..self
                },
                Vec::new(),
            ),
            super::review::Answer::Go => {
                let items: Vec<Item> = review.items.iter().map(|(item, _)| item.clone()).collect();
                let cleaning = Cleaning {
                    total: items.len(),
                    done: Vec::new(),
                    free_after: None,
                };
                let app = Self {
                    screen: Screen::Cleaning,
                    cleaning: Some(cleaning),
                    ..self
                };
                (app, vec![Effect::Clean(items)])
            }
        }
    }

    pub(super) fn summary_key(self, key: Key) -> (Self, Vec<Effect>) {
        match key {
            Key::Enter | Key::Esc | Key::Char('q') => {
                // What was on disk changed: scan again next time, and drop the old ticks.
                let app = Self {
                    screen: Screen::Menu,
                    scan: Loading::Idle,
                    packages: Loading::Idle,
                    scan_generation: self.scan_generation + 1,
                    packages_generation: self.packages_generation + 1,
                    clean_list: ListState::default(),
                    package_list: ListState::default(),
                    purge: self.purge.reset(),
                    review: None,
                    ..self
                };
                (app, Vec::new())
            }
            _ => (self, Vec::new()),
        }
    }

    pub(super) fn keep_key(self, key: Key) -> (Self, Vec<Effect>) {
        let Ok(keep) = self.keep.clone() else {
            return self.back_on(key);
        };
        match self.keep_screen.on_key(key, &keep) {
            super::keep_screen::Answer::Wait(next) => (
                Self {
                    keep_screen: next,
                    ..self
                },
                Vec::new(),
            ),
            super::keep_screen::Answer::Back => (
                Self {
                    screen: Screen::Menu,
                    ..self
                },
                Vec::new(),
            ),
            super::keep_screen::Answer::Save(next) => {
                let app = Self {
                    screen: Screen::Menu,
                    keep_screen: KeepScreen::default(),
                    message: Some("Keep list saved.".to_string()),
                    ..self
                }
                .with_keep(&next);
                (app, vec![Effect::SaveKeep(next)])
            }
        }
    }

    pub(super) fn history_key(self, key: Key) -> (Self, Vec<Effect>) {
        match key {
            Key::Up => (
                Self {
                    history_scroll: self.history_scroll.saturating_sub(1),
                    ..self
                },
                Vec::new(),
            ),
            Key::Down => {
                let last = self.history_lines().saturating_sub(1);
                let history_scroll = (self.history_scroll + 1).min(last);
                (
                    Self {
                        history_scroll,
                        ..self
                    },
                    Vec::new(),
                )
            }
            _ => self.back_on(key),
        }
    }
}

/// Every version of a package but the newest, and never one a project uses.
fn is_older_version(group: &Group, entry: &super::list::Entry) -> bool {
    !entry.in_use
        && group.entries.len() > 1
        && group.key.contains(':')
        && group
            .entries
            .last()
            .is_some_and(|newest| newest.item.key() != entry.item.key())
}

fn row_kept(list: &ListState, row: Row) -> bool {
    match row {
        Row::Group(s, g) => list.group(s, g).all_kept(),
        Row::Entry(s, g, e) => list.group(s, g).entries[e].kept,
        Row::Heading(_) => false,
    }
}
