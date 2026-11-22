//! What was brought over from each browser, and when (ImportRecord.swift):
//! the sheet says so beside the browser, "Replace" takes back out exactly
//! the bookmarks that came from it, and bringing again adds only what is new.
//! Kept by the browser's name, whichever profile.
//!
//! Bookmarks need no memory to come again without doubles: `Bookmarks::take`
//! leaves a page already there. History and tabs do: a place's visits are
//! remembered as they were counted, so again brings only the visits since;
//! a tab or pin brought once (by its address) isn't brought twice.

use searchwin_history::Entry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// When last brought from, Unix ms.
    pub date: u64,
    #[serde(default)]
    pub bookmarks: usize,
    #[serde(default)]
    pub places: usize,
    #[serde(default)]
    pub passwords: usize,
    /// Every bookmark and folder an import from it added.
    #[serde(default, rename = "bookmarkIDs")]
    pub bookmark_ids: Vec<String>,
    /// Arc's spaces made, and pins brought. None before any was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spaces: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<usize>,
    /// Visits brought per place, by a hash of its address.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visits: BTreeMap<u64, u32>,
    /// Tabs and pins brought, by a hash of what they were and their address.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub seen: BTreeSet<u64>,
}

/// What an import just added, for `Records::note`.
#[derive(Clone, Debug, Default)]
pub struct Added {
    pub bookmark_ids: Vec<String>,
    pub bookmarks: usize,
    pub places: usize,
    pub passwords: usize,
    pub spaces: usize,
    pub pinned: usize,
    pub tabs: usize,
}

/// Every browser's record, by its name.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct Records(BTreeMap<String, Record>);

impl Records {
    /// From the file; empty when there is none or it doesn't read.
    pub fn load(path: &Path) -> Records {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Written whole to a temporary file beside, then put in place.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, path)
    }

    pub fn of(&self, name: &str) -> Option<&Record> {
        self.0.get(name)
    }

    /// What an import from `name` just added, on top of what was noted.
    pub fn note(&mut self, name: &str, added: Added, now_ms: u64) {
        let record = self.0.entry(name.to_string()).or_default();
        record.date = now_ms;
        record.bookmarks += added.bookmarks;
        record.places += added.places;
        record.passwords += added.passwords;
        record.bookmark_ids.extend(added.bookmark_ids);
        for (field, n) in [(&mut record.spaces, added.spaces), (&mut record.pinned, added.pinned), (&mut record.tabs, added.tabs)] {
            if n > 0 {
                *field = Some(field.unwrap_or(0) + n);
            }
        }
    }

    /// Its bookmarks taken back out: what is recorded of them goes too.
    pub fn forget_bookmarks(&mut self, name: &str) {
        if let Some(record) = self.0.get_mut(name) {
            record.bookmark_ids.clear();
            record.bookmarks = 0;
        }
    }

    /// Of `entries` read from `name`, the visits not brought before: a place
    /// new here whole, one seen before with only its count since, one with
    /// no new visit left out. What is returned is remembered as brought.
    pub fn fresh_history(&mut self, name: &str, entries: Vec<Entry>) -> Vec<Entry> {
        let record = self.0.entry(name.to_string()).or_default();
        let mut out = Vec::new();
        for mut entry in entries {
            let key = hash(&entry.url);
            let before = record.visits.get(&key).copied().unwrap_or(0);
            if entry.count <= before {
                continue;
            }
            record.visits.insert(key, entry.count);
            entry.count -= before;
            out.push(entry);
        }
        out
    }

    /// Whether `url` of this kind ("tab", "pin", "favorite") is new from
    /// `name`; it is remembered as brought either way.
    pub fn first_time(&mut self, name: &str, kind: &str, url: &str) -> bool {
        let record = self.0.entry(name.to_string()).or_default();
        record.seen.insert(hash(&format!("{kind}\n{url}")))
    }
}

/// FNV-1a: the same number for the same address on every run and version,
/// which std's hasher doesn't promise.
fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
