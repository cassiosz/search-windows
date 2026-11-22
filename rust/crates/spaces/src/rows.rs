//! One row of tabs per space (upstream's `Parked`, Spaces.swift:285-293):
//! the space on screen and the ones left behind, each a searchwin_tabs::Tabs
//! read from its own session file the first time it is wanted.

use crate::session_path;
use searchwin_tabs::{Shape, Tabs};
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub struct Rows {
    rows: HashMap<String, Tabs>,
}

impl Rows {
    pub fn new() -> Rows {
        Rows::default()
    }

    /// A space's row: the one kept in memory, or read back from its file,
    /// tabs asleep as at launch (Spaces.swift:403-417). A space with no file
    /// yet is a fresh row.
    pub fn row(&mut self, base: &Path, space: &str, now_ms: u64) -> &mut Tabs {
        self.rows
            .entry(space.to_string())
            .or_insert_with(|| searchwin_tabs::load(&session_path(base, space)).into_tabs(now_ms))
    }

    pub fn get(&self, space: &str) -> Option<&Tabs> {
        self.rows.get(space)
    }

    pub fn get_mut(&mut self, space: &str) -> Option<&mut Tabs> {
        self.rows.get_mut(space)
    }

    pub fn insert(&mut self, space: &str, tabs: Tabs) {
        self.rows.insert(space.to_string(), tabs);
    }

    /// A deleted space's row: its tabs are the app's to close (Spaces.swift:959-965).
    pub fn remove(&mut self, space: &str) -> Option<Tabs> {
        self.rows.remove(space)
    }

    /// The spaces with a row in memory.
    pub fn spaces(&self) -> impl Iterator<Item = &str> {
        self.rows.keys().map(String::as_str)
    }

    /// Every tab of every row, for the sleep timer (Spaces.swift:340-341).
    pub fn tabs(&self) -> impl Iterator<Item = (&str, &searchwin_tabs::Tab)> {
        self.rows.iter().flat_map(|(s, t)| t.iter().map(move |tab| (s.as_str(), tab)))
    }

    /// Writes one space's row to its session file.
    pub fn save(&self, base: &Path, space: &str) -> std::io::Result<()> {
        match self.rows.get(space) {
            Some(tabs) => searchwin_tabs::save(&session_path(base, space), &Shape::from_tabs(tabs)),
            None => Ok(()),
        }
    }

    /// Writes every row in memory; the first error is returned, the rest
    /// are still written.
    pub fn save_all(&self, base: &Path) -> std::io::Result<()> {
        let mut first = Ok(());
        for space in self.rows.keys() {
            let done = self.save(base, space);
            if first.is_ok() {
                first = done;
            }
        }
        first
    }
}
