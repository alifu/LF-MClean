//! The list shared by the Clean and Packages screens: sections of groups (a cache or a package),
//! each opening onto its items. Kept things are marked here and can never be ticked.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::catalog::Safety;
use crate::keep::{KeepList, KeptPackage};
use crate::scan::Item;

/// What `k` keeps on a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepRef {
    /// A whole target.
    Target(&'static str),
    /// A package, or one version of it.
    Package(KeptPackage),
    /// One path.
    Path(PathBuf),
}

impl KeepRef {
    /// True when the keep list holds exactly this entry (not only something covering it).
    #[must_use]
    pub fn is_in(&self, keep: &KeepList) -> bool {
        match self {
            Self::Target(id) => keep.targets().any(|kept| kept == *id),
            Self::Package(package) => keep.without_package(package) != *keep,
            Self::Path(path) => keep.without_path(path) != *keep,
        }
    }

    /// The keep list with this entry added, or removed when it is there.
    ///
    /// # Errors
    /// When the entry cannot be written to the keep list.
    pub fn toggled(&self, keep: &KeepList) -> Result<KeepList, String> {
        if self.is_in(keep) {
            return Ok(match self {
                Self::Target(id) => keep.without_target(id),
                Self::Package(package) => keep.without_package(package),
                Self::Path(path) => keep.without_path(path),
            });
        }
        match self {
            Self::Target(id) => keep.with_target(id),
            Self::Package(package) => Ok(keep.with_package(package.clone())),
            Self::Path(path) => path
                .to_str()
                .ok_or_else(|| format!("{} is not UTF-8", path.display()))
                .and_then(|text| keep.with_path(text)),
        }
    }
}

/// One removable thing in a group.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The scan item.
    pub item: Item,
    /// The name shown.
    pub label: String,
    /// Text after the size: an age, "in use", and so on.
    pub detail: String,
    /// What `k` keeps.
    pub keep_ref: KeepRef,
    /// True when the keep list keeps it (by any entry).
    pub kept: bool,
    /// True when a project's lock file names this copy; shortcuts never tick it.
    pub in_use: bool,
    /// How careful to be with this one when it differs from its group (a project's `Pods`).
    pub safety: Option<Safety>,
}

/// A cache or a package, with its items.
#[derive(Debug, Clone)]
pub struct Group {
    /// A key unique in the list, used to remember which groups are open.
    pub key: String,
    /// The name shown.
    pub label: String,
    /// Text after the size: "3 versions · used by: App", and so on.
    pub detail: String,
    /// How careful to be.
    pub safety: Safety,
    /// What `k` keeps on the group row.
    pub keep_ref: KeepRef,
    /// The items.
    pub entries: Vec<Entry>,
    /// A package no project's lock file names.
    pub unused: bool,
}

impl Group {
    /// How careful to be with `entry`: its own level, else the group's.
    #[must_use]
    pub fn safety_of(&self, entry: &Entry) -> Safety {
        entry.safety.unwrap_or(self.safety)
    }

    /// Total size of the items.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.item.size_bytes).sum()
    }

    /// True when every item is kept.
    #[must_use]
    pub fn all_kept(&self) -> bool {
        self.entries.iter().all(|e| e.kept)
    }
}

/// A heading and its groups.
#[derive(Debug, Clone)]
pub struct Section {
    /// The heading.
    pub title: String,
    /// The groups, in order.
    pub groups: Vec<Group>,
}

/// One line of the list as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A section heading; the cursor skips it.
    Heading(usize),
    /// A group: section, group.
    Group(usize, usize),
    /// An item: section, group, entry.
    Entry(usize, usize, usize),
}

/// How much of a group is ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Nothing.
    None,
    /// Some items.
    Some,
    /// Every item that is not kept.
    All,
}

/// The list with its cursor, open groups and ticked items. Every change returns a new value.
#[derive(Debug, Clone, Default)]
pub struct ListState {
    /// The sections shown.
    pub sections: Vec<Section>,
    /// Index into `rows()` of the cursor; always a group or an item row when there is one.
    pub cursor: usize,
    /// Keys of the open groups.
    pub open: BTreeSet<String>,
    /// Keys (`Item::key`) of the ticked items.
    pub ticked: BTreeSet<String>,
}

impl ListState {
    /// A list showing `sections`, with the cursor on the first group.
    #[must_use]
    pub fn new(sections: Vec<Section>) -> Self {
        let list = Self {
            sections,
            ..Self::default()
        };
        let first = list.movable_rows().first().copied().unwrap_or(0);
        Self {
            cursor: first,
            ..list
        }
    }

    /// The same list with new sections (after a keep change or a new scan): open groups and
    /// ticks stay where they still apply, and nothing kept stays ticked.
    #[must_use]
    pub fn with_sections(&self, sections: Vec<Section>) -> Self {
        let tickable: BTreeSet<String> = sections
            .iter()
            .flat_map(|s| &s.groups)
            .flat_map(|g| &g.entries)
            .filter(|e| !e.kept)
            .map(|e| e.item.key())
            .collect();
        let next = Self {
            sections,
            ticked: self.ticked.intersection(&tickable).cloned().collect(),
            ..self.clone()
        };
        next.clamped()
    }

    /// Every row in drawing order.
    #[must_use]
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (s, section) in self.sections.iter().enumerate() {
            rows.push(Row::Heading(s));
            for (g, group) in section.groups.iter().enumerate() {
                rows.push(Row::Group(s, g));
                if self.open.contains(&group.key) {
                    rows.extend((0..group.entries.len()).map(|e| Row::Entry(s, g, e)));
                }
            }
        }
        rows
    }

    fn movable_rows(&self) -> Vec<usize> {
        self.rows()
            .iter()
            .enumerate()
            .filter(|(_, row)| !matches!(row, Row::Heading(_)))
            .map(|(i, _)| i)
            .collect()
    }

    fn clamped(&self) -> Self {
        let movable = self.movable_rows();
        let cursor = movable
            .iter()
            .copied()
            .find(|&i| i >= self.cursor)
            .or_else(|| movable.last().copied())
            .unwrap_or(0);
        Self {
            cursor,
            ..self.clone()
        }
    }

    /// The row under the cursor.
    #[must_use]
    pub fn current(&self) -> Option<Row> {
        self.rows()
            .get(self.cursor)
            .copied()
            .filter(|row| !matches!(row, Row::Heading(_)))
    }

    /// The group at `(section, group)`.
    #[must_use]
    pub fn group(&self, section: usize, group: usize) -> &Group {
        &self.sections[section].groups[group]
    }

    /// The cursor moved by `step` rows, skipping headings and stopping at the ends.
    #[must_use]
    pub fn moved(&self, step: isize) -> Self {
        let movable = self.movable_rows();
        let at = movable.iter().position(|&i| i == self.cursor).unwrap_or(0);
        let last = movable.len().saturating_sub(1);
        let to = at.saturating_add_signed(step).min(last);
        Self {
            cursor: movable.get(to).copied().unwrap_or(0),
            ..self.clone()
        }
    }

    /// The group under the cursor opened or closed. On an item row, closing closes its group.
    #[must_use]
    pub fn opened(&self, open: bool) -> Self {
        let Some(row) = self.current() else {
            return self.clone();
        };
        let (s, g) = match row {
            Row::Group(s, g) | Row::Entry(s, g, _) => (s, g),
            Row::Heading(_) => return self.clone(),
        };
        let key = self.group(s, g).key.clone();
        let mut next = self.clone();
        if open {
            next.open.insert(key);
        } else {
            next.open.remove(&key);
            next.cursor = next
                .rows()
                .iter()
                .position(|r| *r == Row::Group(s, g))
                .unwrap_or(0);
        }
        next.clamped()
    }

    /// How much of a group is ticked.
    #[must_use]
    pub fn tick_of(&self, group: &Group) -> Tick {
        let tickable: Vec<_> = group.entries.iter().filter(|e| !e.kept).collect();
        let ticked = tickable
            .iter()
            .filter(|e| self.ticked.contains(&e.item.key()))
            .count();
        match ticked {
            0 => Tick::None,
            n if n == tickable.len() => Tick::All,
            _ => Tick::Some,
        }
    }

    /// Space toggled on the cursor row: a group ticks or unticks all its items that are not kept.
    /// Returns `None` when the row is kept and cannot be ticked.
    #[must_use]
    pub fn toggled(&self) -> Option<Self> {
        let mut next = self.clone();
        match self.current()? {
            Row::Group(s, g) => {
                let group = self.group(s, g);
                if group.all_kept() {
                    return None;
                }
                let keys = group
                    .entries
                    .iter()
                    .filter(|e| !e.kept)
                    .map(|e| e.item.key());
                if self.tick_of(group) == Tick::All {
                    keys.for_each(|key| {
                        next.ticked.remove(&key);
                    });
                } else {
                    next.ticked.extend(keys);
                }
            }
            Row::Entry(s, g, e) => {
                let entry = &self.group(s, g).entries[e];
                if entry.kept {
                    return None;
                }
                let key = entry.item.key();
                if !next.ticked.remove(&key) {
                    next.ticked.insert(key);
                }
            }
            Row::Heading(_) => return None,
        }
        Some(next)
    }

    /// The items for which `pick` says yes ticked as well (never a kept one).
    #[must_use]
    pub fn ticking(&self, pick: impl Fn(&Group, &Entry) -> bool) -> Self {
        let mut next = self.clone();
        for group in self.sections.iter().flat_map(|s| &s.groups) {
            next.ticked.extend(
                group
                    .entries
                    .iter()
                    .filter(|e| !e.kept && pick(group, e))
                    .map(|e| e.item.key()),
            );
        }
        next
    }

    /// Nothing ticked.
    #[must_use]
    pub fn cleared(&self) -> Self {
        Self {
            ticked: BTreeSet::new(),
            ..self.clone()
        }
    }

    /// The ticked entries that are not kept, with their group, in list order.
    #[must_use]
    pub fn picked(&self) -> Vec<(&Group, &Entry)> {
        self.sections
            .iter()
            .flat_map(|s| &s.groups)
            .flat_map(|g| g.entries.iter().map(move |e| (g, e)))
            .filter(|(_, e)| !e.kept && self.ticked.contains(&e.item.key()))
            .collect()
    }

    /// Total size of the ticked entries.
    #[must_use]
    pub fn picked_bytes(&self) -> u64 {
        self.picked().iter().map(|(_, e)| e.item.size_bytes).sum()
    }

    /// Total size of everything listed.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.sections
            .iter()
            .flat_map(|s| &s.groups)
            .map(Group::total_bytes)
            .sum()
    }

    /// What `k` keeps on the cursor row.
    #[must_use]
    pub fn keep_ref(&self) -> Option<&KeepRef> {
        match self.current()? {
            Row::Group(s, g) => Some(&self.group(s, g).keep_ref),
            Row::Entry(s, g, e) => Some(&self.group(s, g).entries[e].keep_ref),
            Row::Heading(_) => None,
        }
    }
}
