//! The Purge screen: the build folders of the user's projects, one group per project.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, FixedOffset};

use super::app::Loading;
use super::list::{Group, KeepRef, ListState, Section};
use super::sections::{aged, entry};
use crate::catalog::{self, Safety};
use crate::format::{age, items, printable};
use crate::keep::KeepList;
use crate::purge::{Found, Project, default_folders};

/// What the Purge screen knows.
#[derive(Debug, Clone)]
pub struct PurgeState {
    /// The folders searched for projects.
    pub folders: Vec<PathBuf>,
    /// The number of the search whose results count; results of older ones are dropped.
    pub generation: u64,
    /// The search.
    pub found: Loading<Arc<Found>>,
    /// The list of projects.
    pub list: ListState,
}

impl PurgeState {
    /// Nothing searched yet, in the usual project folders of `home`.
    #[must_use]
    pub fn new(env: &crate::env::Env) -> Self {
        Self {
            folders: default_folders(env),
            generation: 0,
            found: Loading::Idle,
            list: ListState::default(),
        }
    }

    /// The same, after what is on disk changed: searched again next time, ticks dropped.
    #[must_use]
    pub fn reset(&self) -> Self {
        Self {
            folders: self.folders.clone(),
            generation: self.generation + 1,
            found: Loading::Idle,
            list: ListState::default(),
        }
    }

    /// The list marked again for a new keep list.
    #[must_use]
    pub fn with_keep(self, keep: &KeepList, now: DateTime<FixedOffset>) -> Self {
        match &self.found {
            Loading::Done(found) => {
                let list = self.list.with_sections(purge_sections(found, keep, now));
                Self { list, ..self }
            }
            _ => self,
        }
    }
}

/// One section holding one group per project, each opening onto its folders.
#[must_use]
pub fn purge_sections(found: &Found, keep: &KeepList, now: DateTime<FixedOffset>) -> Vec<Section> {
    if found.projects.is_empty() {
        return Vec::new();
    }
    vec![Section {
        title: "Projects".to_string(),
        groups: found
            .projects
            .iter()
            .map(|project| project_group(project, keep, now))
            .collect(),
    }]
}

fn project_group(project: &Project, keep: &KeepList, now: DateTime<FixedOffset>) -> Group {
    let changed = project
        .changed
        .map_or_else(String::new, |time| format!("changed {} · ", age(time, now)));
    Group {
        key: project.folder.display().to_string(),
        label: format!("{} ({})", printable(&project.name), project.items.len()),
        detail: format!("{changed}{}", items(project.items.len())),
        safety: project
            .items
            .iter()
            .filter_map(|item| catalog::find(item.target))
            .map(|target| target.safety)
            .max()
            .unwrap_or(Safety::Safe),
        keep_ref: KeepRef::Path(project.folder.clone()),
        entries: project
            .items
            .iter()
            .map(|item| {
                let mut row = entry(item, printable(&item.name), aged(item, now), keep);
                row.safety = catalog::find(item.target).map(|target| target.safety);
                row
            })
            .collect(),
        unused: false,
    }
}
