//! The last screen before removal: every path, its size and safety, the total, and warnings.
//! `y` confirms; when a risky item is ticked, the word `delete` must be typed instead. Either is
//! taken only once every line of the list has been on screen.

use std::collections::BTreeSet;

use super::app::{Key, Screen};
use crate::catalog::{self, Safety};
use crate::scan::{Item, Removal};
use crate::system::{Tool, tools_using};

/// The word typed to remove risky items.
pub const CONFIRM_WORD: &str = "delete";
/// Lines above the list: the title and a blank line.
pub const TOP_LINES: usize = 2;
/// Lines below the list besides the warnings: blank, total, mode, blank, prompt, note.
const BOTTOM_LINES: usize = 6;

/// What the person picked, waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// The items and how careful to be with each.
    pub items: Vec<(Item, Safety)>,
    /// The screen to go back to.
    pub from: Screen,
    /// What has been typed so far, when a word is needed.
    pub typed: String,
    /// Things to know before saying yes.
    pub warnings: Vec<String>,
    /// The first list line shown.
    pub scroll: usize,
    /// The lowest list line that has been on screen, plus one.
    pub seen: usize,
    /// Lines the screen body has.
    pub body_rows: usize,
    /// Why the last key did nothing.
    pub note: Option<String>,
}

/// What a key on the review screen means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Still deciding.
    Wait(Review),
    /// Go back without removing.
    Back,
    /// Remove.
    Go,
}

impl Review {
    /// A review of `items`, warning about the `running` tools that use them.
    #[must_use]
    pub fn new(
        items: Vec<(Item, Safety)>,
        from: Screen,
        running: &[Tool],
        body_rows: usize,
    ) -> Self {
        let warnings = warnings(&items, running);
        Self {
            items,
            from,
            typed: String::new(),
            warnings,
            scroll: 0,
            seen: 0,
            body_rows: 0,
            note: None,
        }
        .with_body_rows(body_rows)
    }

    /// The review on a screen body this tall; what fits is counted as seen.
    #[must_use]
    pub fn with_body_rows(self, body_rows: usize) -> Self {
        let next = Self { body_rows, ..self };
        let scroll = next.scroll.min(next.max_scroll());
        Self { scroll, ..next }.marked_seen()
    }

    fn marked_seen(self) -> Self {
        let seen = self
            .seen
            .max((self.scroll + self.list_rows()).min(self.list_len()));
        Self { seen, ..self }
    }

    /// Every line of the list: each item, then each folder it removes.
    #[must_use]
    pub fn list_len(&self) -> usize {
        self.items
            .iter()
            .map(|(item, _)| 1 + item.removal.folders().len())
            .sum()
    }

    /// How many list lines fit between the title and the total, warnings and prompt.
    #[must_use]
    pub fn list_rows(&self) -> usize {
        self.body_rows
            .saturating_sub(TOP_LINES + BOTTOM_LINES + self.warnings.len())
            .max(1)
    }

    fn max_scroll(&self) -> usize {
        self.list_len().saturating_sub(self.list_rows())
    }

    /// True once every list line has been on screen.
    #[must_use]
    pub fn seen_all(&self) -> bool {
        self.seen >= self.list_len()
    }

    fn scrolled(&self, by: isize) -> Self {
        let scroll = self.scroll.saturating_add_signed(by).min(self.max_scroll());
        Self {
            scroll,
            note: None,
            ..self.clone()
        }
        .marked_seen()
    }

    /// True when a risky item is picked, so the word must be typed.
    #[must_use]
    pub fn needs_word(&self) -> bool {
        self.items
            .iter()
            .any(|(_, safety)| *safety == Safety::Risky)
    }

    /// Total size of the items.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|(item, _)| item.size_bytes).sum()
    }

    /// What a key means here.
    #[must_use]
    pub fn on_key(&self, key: Key) -> Answer {
        if key == Key::Esc {
            return Answer::Back;
        }
        let page = isize::try_from(self.list_rows()).unwrap_or(1);
        match key {
            Key::Up => return Answer::Wait(self.scrolled(-1)),
            Key::Down => return Answer::Wait(self.scrolled(1)),
            Key::Char(' ') if !self.needs_word() => return Answer::Wait(self.scrolled(page)),
            _ => {}
        }
        let confirms = matches!(key, Key::Char('y' | 'Y')) && !self.needs_word()
            || key == Key::Enter && self.needs_word() && self.typed == CONFIRM_WORD;
        if confirms && !self.seen_all() {
            let note = "Scroll down (↓) to see every item before removing.".to_string();
            return Answer::Wait(Self {
                note: Some(note),
                ..self.clone()
            });
        }
        if !self.needs_word() {
            return match key {
                Key::Char('y' | 'Y') => Answer::Go,
                Key::Char('n' | 'N' | 'q') => Answer::Back,
                _ => Answer::Wait(self.clone()),
            };
        }
        let mut next = self.clone();
        match key {
            Key::Enter if self.typed == CONFIRM_WORD => return Answer::Go,
            Key::Enter => next.typed.clear(),
            Key::Backspace => {
                next.typed.pop();
            }
            Key::Char(c) if next.typed.len() < CONFIRM_WORD.len() * 2 => next.typed.push(c),
            _ => {}
        }
        Answer::Wait(next)
    }
}

fn warnings(items: &[(Item, Safety)], running: &[Tool]) -> Vec<String> {
    let targets: BTreeSet<&'static str> = items.iter().map(|(item, _)| item.target).collect();
    let mut notes: Vec<String> = running
        .iter()
        .filter_map(|tool| {
            let used: Vec<&str> = targets
                .iter()
                .filter(|id| !tools_using(&[*tool], id).is_empty())
                .filter_map(|id| catalog::find(id).map(|t| t.name))
                .collect();
            (!used.is_empty()).then(|| {
                format!(
                    "{} is running and uses {}: {}.",
                    tool.name(),
                    used.join(", "),
                    tool.advice()
                )
            })
        })
        .collect();
    if items
        .iter()
        .any(|(item, _)| matches!(item.removal, Removal::SimctlDelete { .. }))
    {
        notes.push(
            "Simulators are deleted for good by simctl; they do not go to the Trash.".to_string(),
        );
    }
    notes
}
