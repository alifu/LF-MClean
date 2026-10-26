//! The state of the interactive screens and how keys and background events change it. Nothing
//! here draws or touches the disk: a change returns a new `App` and the effects to carry out.

use std::sync::Arc;

use chrono::{DateTime, FixedOffset};

use super::keep_screen::KeepScreen;
use super::list::{Group, ListState, Row};
use super::review::Review;
use super::sections::{clean_sections, package_sections};
use crate::catalog::Safety;
use crate::clean::Cleaned;
use crate::env::Env;
use crate::keep::KeepList;
use crate::mode::Mode;
use crate::oplog::LogContents;
use crate::oplog::Stage;
use crate::packages::{Browse, Usage};
use crate::scan::{Item, Report};
use crate::system::Tool;

/// Body lines assumed until the terminal size is known.
const DEFAULT_BODY_ROWS: usize = 20;

/// A key, as the screens understand it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Return.
    Enter,
    /// Escape.
    Esc,
    /// Delete the last typed character.
    Backspace,
    /// A typed character; space included.
    Char(char),
    /// Ctrl-C: leave at once, from any screen.
    Interrupt,
}

/// Which screen is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The main menu.
    Menu,
    /// Caches by category.
    Clean,
    /// `CocoaPods` and Swift packages.
    Packages,
    /// Build folders of projects (comes with project discovery).
    Purge,
    /// The keep list.
    Keep,
    /// What was removed before.
    History,
    /// What is about to be removed.
    Review,
    /// Removal in progress.
    Cleaning,
    /// What was removed.
    Summary,
}

/// The menu entries, in order, with what they open and their subcommand.
pub const MENU: &[(Screen, &str, &str, &str)] = &[
    (
        Screen::Clean,
        "Clean",
        "Xcode, Android, Gradle and Kotlin caches",
        "lfmclean clean",
    ),
    (
        Screen::Packages,
        "Packages",
        "Pick CocoaPods and SPM packages to delete or keep",
        "lfmclean packages",
    ),
    (
        Screen::Purge,
        "Purge",
        "Build folders of your CMP projects",
        "lfmclean purge",
    ),
    (
        Screen::Keep,
        "Keep list",
        "Caches and packages that are never removed",
        "lfmclean keep",
    ),
    (
        Screen::History,
        "History",
        "What was removed, and when",
        "lfmclean history",
    ),
    (Screen::Menu, "Quit", "", ""),
];

/// Work for the event loop; its results come back as `Event`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Scan every cache; the number tells this scan from older ones still running.
    Scan(u64),
    /// Scan the package caches and the projects' lock files; numbered like `Scan`.
    ScanPackages(u64),
    /// Write the keep list.
    SaveKeep(KeepList),
    /// Read the removal log.
    LoadHistory,
    /// Load the keep list again, then remove these items.
    Clean(Vec<Item>),
    /// Leave.
    Quit,
}

/// Something that finished in the background.
#[derive(Debug)]
pub enum Event {
    /// Scan number `.0` moved on to this target.
    Scanning(u64, &'static str),
    /// Scan number `.0` finished.
    Scanned(u64, Report),
    /// Package scan number `.0` finished.
    PackagesScanned(u64, Browse, Usage),
    /// Background work stopped without finishing, from a bug; what it was doing.
    Crashed(String),
    /// The running tools are known (or could not be checked).
    Tools(Result<Vec<Tool>, String>),
    /// The keep list was written, or not.
    KeepSaved(Result<(), String>),
    /// The removal log was read, or not.
    History(Result<LogContents, String>),
    /// The keep list could not be read again, so nothing was removed.
    CleanRefused(String),
    /// One item was handled.
    Cleaned(Cleaned),
    /// Every item was handled; the free space now, if known.
    CleanFinished(Option<u64>),
}

/// Something loaded in the background.
#[derive(Debug, Clone)]
pub enum Loading<T> {
    /// Not asked for yet, or out of date.
    Idle,
    /// Being loaded; what it is doing.
    Running(String),
    /// Loaded.
    Done(T),
}

/// Removal in progress, then its result.
#[derive(Debug, Clone)]
pub struct Cleaning {
    /// How many items were sent.
    pub total: usize,
    /// The items handled so far.
    pub done: Vec<Cleaned>,
    /// Free space after, when known.
    pub free_after: Option<u64>,
}

/// Everything the screens show.
#[derive(Debug, Clone)]
pub struct App {
    /// How removal works this run.
    pub mode: Mode,
    /// The screen shown.
    pub screen: Screen,
    /// The menu cursor.
    pub menu_cursor: usize,
    /// The keep list, or why it cannot be used (then nothing can be removed).
    pub keep: Result<KeepList, String>,
    /// The cache scan.
    pub scan: Loading<Arc<Report>>,
    /// The number of the scan whose results count; results of older scans are dropped.
    pub scan_generation: u64,
    /// The same for the package scan.
    pub packages_generation: u64,
    /// Lines the screen body has, so the Review screen knows what has been seen.
    pub body_rows: usize,
    /// The package scan.
    pub packages: Loading<Arc<(Browse, Usage)>>,
    /// The Clean screen list.
    pub clean_list: ListState,
    /// The Packages screen list.
    pub package_list: ListState,
    /// Tools running now.
    pub running: Vec<Tool>,
    /// Free space when the program started.
    pub free_space: Option<u64>,
    /// The removal log.
    pub history: Loading<Arc<Result<LogContents, String>>>,
    /// The first history line shown.
    pub history_scroll: usize,
    /// The Keep list screen.
    pub keep_screen: KeepScreen,
    /// The Review screen, while it is open.
    pub review: Option<Review>,
    /// The removal in progress or done.
    pub cleaning: Option<Cleaning>,
    /// A one-line note shown until the next key.
    pub message: Option<String>,
    /// The time ages are counted from.
    pub now: DateTime<FixedOffset>,
    /// An empty keep list for this home, used to draw lists while the real one is broken.
    blank_keep: KeepList,
}

impl App {
    /// The menu, before anything is scanned.
    #[must_use]
    pub fn new(
        mode: Mode,
        env: &Env,
        keep: Result<KeepList, String>,
        free_space: Option<u64>,
        now: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            blank_keep: KeepList::empty(env),
            mode,
            screen: Screen::Menu,
            menu_cursor: 0,
            keep,
            scan: Loading::Idle,
            scan_generation: 0,
            packages_generation: 0,
            body_rows: DEFAULT_BODY_ROWS,
            packages: Loading::Idle,
            clean_list: ListState::default(),
            package_list: ListState::default(),
            running: Vec::new(),
            free_space,
            history: Loading::Idle,
            history_scroll: 0,
            keep_screen: KeepScreen::default(),
            review: None,
            cleaning: None,
            message: None,
            now,
        }
    }

    /// The keep list to mark kept items with; when it is broken nothing is marked, and nothing
    /// can be removed either.
    #[must_use]
    pub fn keep_or_empty(&self) -> KeepList {
        match &self.keep {
            Ok(keep) => keep.clone(),
            Err(_) => self.blank_keep.clone(),
        }
    }

    /// The total the scan found, once it is done.
    #[must_use]
    pub fn found_bytes(&self) -> Option<u64> {
        match &self.scan {
            Loading::Done(report) => Some(report.total_bytes()),
            _ => None,
        }
    }

    /// The app told how many lines the screen body has.
    #[must_use]
    pub fn with_body_rows(self, body_rows: usize) -> Self {
        let review = self
            .review
            .clone()
            .map(|review| review.with_body_rows(body_rows));
        Self {
            body_rows,
            review,
            ..self
        }
    }

    /// How many removals the history lists.
    #[must_use]
    pub fn history_lines(&self) -> usize {
        match &self.history {
            Loading::Done(result) => result.as_ref().as_ref().map_or(0, |contents| {
                contents
                    .entries
                    .iter()
                    .filter(|e| e.stage == Stage::Finished)
                    .count()
            }),
            _ => 0,
        }
    }

    /// The new state after a key, and what to do.
    #[must_use]
    pub fn on_key(&self, key: Key) -> (Self, Vec<Effect>) {
        if key == Key::Interrupt {
            return (self.clone(), vec![Effect::Quit]);
        }
        let app = Self {
            message: None,
            ..self.clone()
        };
        match app.screen {
            Screen::Menu => app.menu_key(key),
            Screen::Clean | Screen::Packages => app.list_key(key),
            Screen::Purge => app.back_on(key),
            Screen::Keep => app.keep_key(key),
            Screen::History => app.history_key(key),
            Screen::Review => app.review_key(key),
            Screen::Cleaning => (app, Vec::new()),
            Screen::Summary => app.summary_key(key),
        }
    }

    /// The new state after something finished in the background, and what to do.
    #[must_use]
    pub fn on_event(&self, event: Event) -> (Self, Vec<Effect>) {
        let mut app = self.clone();
        match event {
            Event::Scanning(generation, _) | Event::Scanned(generation, _)
                if generation != app.scan_generation => {}
            Event::PackagesScanned(generation, ..) if generation != app.packages_generation => {}
            Event::Scanning(_, name) => app.scan = Loading::Running(name.to_string()),
            Event::Scanned(_, report) => {
                let sections = clean_sections(&report, &app.keep_or_empty(), app.now);
                app.clean_list = app.clean_list.with_sections(sections);
                app.scan = Loading::Done(Arc::new(report));
            }
            Event::PackagesScanned(_, browse, usage) => {
                let sections = package_sections(&browse, &usage, &app.keep_or_empty(), app.now);
                app.package_list = app.package_list.with_sections(sections);
                app.packages = Loading::Done(Arc::new((browse, usage)));
            }
            Event::Crashed(what) => {
                app.message = Some(format!("{what} stopped because of a bug."));
                if app.screen == Screen::Cleaning {
                    app.screen = Screen::Summary;
                }
                if matches!(app.scan, Loading::Running(_)) {
                    app.scan = Loading::Idle;
                }
                if matches!(app.packages, Loading::Running(_)) {
                    app.packages = Loading::Idle;
                }
            }
            Event::Tools(Ok(tools)) => app.running = tools,
            Event::Tools(Err(reason)) => {
                app.message = Some(format!("Running apps could not be checked: {reason}"));
            }
            Event::KeepSaved(Ok(())) => {}
            Event::KeepSaved(Err(reason)) => {
                app.message = Some(format!("The keep list could not be saved: {reason}"));
            }
            Event::History(result) => {
                app.history = Loading::Done(Arc::new(result));
                app.history_scroll = 0;
            }
            Event::CleanRefused(reason) => {
                app.screen = Screen::Review;
                app.message = Some(format!("Nothing was removed: {reason}"));
            }
            Event::Cleaned(done) => {
                if let Some(cleaning) = &mut app.cleaning {
                    cleaning.done.push(done);
                }
            }
            Event::CleanFinished(free_after) => {
                if let Some(cleaning) = &mut app.cleaning {
                    cleaning.free_after = free_after;
                }
                app.screen = Screen::Summary;
            }
        }
        (app, Vec::new())
    }

    fn menu_key(self, key: Key) -> (Self, Vec<Effect>) {
        let last = MENU.len() - 1;
        match key {
            Key::Up => (
                Self {
                    menu_cursor: self.menu_cursor.saturating_sub(1),
                    ..self
                },
                Vec::new(),
            ),
            Key::Down => (
                Self {
                    menu_cursor: (self.menu_cursor + 1).min(last),
                    ..self
                },
                Vec::new(),
            ),
            Key::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index > last {
                    return (self, Vec::new());
                }
                Self {
                    menu_cursor: index,
                    ..self
                }
                .open(index)
            }
            Key::Enter => {
                let index = self.menu_cursor;
                self.open(index)
            }
            Key::Char('q') | Key::Esc => (self, vec![Effect::Quit]),
            _ => (self, Vec::new()),
        }
    }

    /// The app opened on `screen` as if its menu entry had been chosen; the menu if there is none.
    #[must_use]
    pub fn start_at(self, screen: Screen) -> (Self, Vec<Effect>) {
        // The menu is already showing, and its last entry (Quit) also names it.
        if screen == Screen::Menu {
            return (self, Vec::new());
        }
        match MENU.iter().position(|&(entry, ..)| entry == screen) {
            Some(index) => self.open(index),
            None => (self, Vec::new()),
        }
    }

    /// Opens a menu entry, starting what it needs.
    #[must_use]
    pub fn open(self, index: usize) -> (Self, Vec<Effect>) {
        let Some(&(screen, ..)) = MENU.get(index) else {
            return (self, Vec::new());
        };
        if index == MENU.len() - 1 {
            return (self, vec![Effect::Quit]);
        }
        let app = Self { screen, ..self };
        match screen {
            Screen::Clean if matches!(app.scan, Loading::Idle) => (
                Self {
                    scan: Loading::Running(String::new()),
                    ..app
                },
                vec![Effect::Scan(app.scan_generation)],
            ),
            Screen::Packages if matches!(app.packages, Loading::Idle) => (
                Self {
                    packages: Loading::Running(String::new()),
                    ..app
                },
                vec![Effect::ScanPackages(app.packages_generation)],
            ),
            Screen::History => (
                Self {
                    history: Loading::Running(String::new()),
                    ..app
                },
                vec![Effect::LoadHistory],
            ),
            Screen::Keep => {
                let keep_screen = KeepScreen::default();
                (Self { keep_screen, ..app }, Vec::new())
            }
            _ => (app, Vec::new()),
        }
    }

    fn back_on(self, key: Key) -> (Self, Vec<Effect>) {
        match key {
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

    fn list(&self) -> &ListState {
        if self.screen == Screen::Packages {
            &self.package_list
        } else {
            &self.clean_list
        }
    }

    fn with_list(self, list: ListState) -> Self {
        if self.screen == Screen::Packages {
            Self {
                package_list: list,
                ..self
            }
        } else {
            Self {
                clean_list: list,
                ..self
            }
        }
    }

    fn noted(self, message: impl Into<String>) -> (Self, Vec<Effect>) {
        (
            Self {
                message: Some(message.into()),
                ..self
            },
            Vec::new(),
        )
    }

    fn list_key(self, key: Key) -> (Self, Vec<Effect>) {
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
                let next =
                    list.ticking(|group, entry| group.safety == Safety::Safe && !entry.in_use);
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

    fn tick_unused(self) -> (Self, Vec<Effect>) {
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

    fn toggle_keep(self) -> (Self, Vec<Effect>) {
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
        app
    }

    fn open_review(self) -> (Self, Vec<Effect>) {
        if let Err(reason) = self.keep.clone() {
            return self.noted(format!(
                "Nothing can be removed until the keep list is fixed: {reason}"
            ));
        }
        let picked: Vec<(Item, Safety)> = self
            .list()
            .picked()
            .into_iter()
            .map(|(group, entry)| (entry.item.clone(), group.safety))
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

    fn review_key(self, key: Key) -> (Self, Vec<Effect>) {
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

    fn summary_key(self, key: Key) -> (Self, Vec<Effect>) {
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
                    review: None,
                    ..self
                };
                (app, Vec::new())
            }
            _ => (self, Vec::new()),
        }
    }

    fn keep_key(self, key: Key) -> (Self, Vec<Effect>) {
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

    fn history_key(self, key: Key) -> (Self, Vec<Effect>) {
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
