//! Sync between PCs through a folder the user chooses (docs/sync.md; upstream
//! Sync.swift and SyncMerge.swift). This crate is the merge, the items and
//! the files in that folder; the app only says when to run a pass and applies
//! what comes back.
//!
//! What is synced is a flat map of items, key → a small JSON text. Each PC
//! keeps, for every item, the value it last agreed on with a stamp (when it
//! was set, in seconds) and the PC that set it; a deletion is the same entry
//! with no value, a tombstone, so "gone" travels as surely as "changed".
//!
//! A pass on one PC:
//!   1. stamp: the items as this PC has them now, against what it last agreed
//!      (its base). What differs is an edit made here, stamped later than
//!      anything it has seen (so a PC whose clock runs behind still wins over
//!      a value it already saw); what it agreed on and no longer has is a
//!      tombstone. A first pass while other PCs are there stamps 0, so what is
//!      already in the folder wins where both have an item.
//!   2. merge: this map and every other PC's file, item by item: the latest
//!      stamp, then the device id, then a tombstone, then the larger text —
//!      the same answer on every PC whatever order it reads them in.
//!   3. what changed is applied here; the merged map is the new base and this
//!      PC's own file.
//!
//! Items this build can't read (a newer version's, a damaged one) are carried
//! along untouched and never turned into tombstones.

mod folder;
mod items;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use folder::{INSIDE, Outcome, forget, leave, run};
pub use items::*;

pub type Items = BTreeMap<String, String>;
pub type Map = BTreeMap<String, Entry>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The item's JSON; None for one deleted.
    pub v: Option<String>,
    /// When it was set, seconds since 1970, or later (see `stamp`).
    pub t: f64,
    /// The PC that set it.
    pub d: String,
}

/// One PC's file in one scope's folder: only that PC writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub format: u32,
    pub device: String,
    /// The PC's name, for Settings to say who was last seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// "app", or "profile-<id>": a file in the wrong folder is not read.
    pub scope: String,
    pub written: f64,
    pub items: Map,
}

/// The file format. A file of another is passed over, not misread.
pub const FORMAT: u32 = 1;
/// How long a deletion is remembered: a PC away longer may bring back what
/// was deleted meanwhile, and nothing worse.
pub const TOMBSTONE_LIFE: f64 = 365.0 * 24.0 * 60.0 * 60.0;
/// The kinds of item whose vanishing all at once is not taken as their
/// deletion (see `stamp`), and how many there must have been.
pub const WHOLE_KINDS: [&str; 4] = ["bm:", "kids:", "space:", "pin:"];
pub const WHOLE_KIND_FLOOR: usize = 5;

/// Whether `a` beats `b` for the same item. Total and the same on every PC.
pub fn wins(a: &Entry, b: &Entry) -> bool {
    if a.t != b.t {
        return a.t > b.t;
    }
    if a.d != b.d {
        return a.d > b.d;
    }
    match (&a.v, &b.v) {
        (None, None) => false,
        (None, _) => true,
        (_, None) => false,
        (Some(x), Some(y)) => x > y,
    }
}

/// Every map's items, the winner of each.
pub fn merge(maps: &[Map]) -> Map {
    let mut out = Map::new();
    for map in maps {
        for (key, entry) in map {
            if out.get(key).is_some_and(|held| !wins(entry, held)) {
                continue;
            }
            out.insert(key.clone(), entry.clone());
        }
    }
    out
}

/// The items as this PC has them now, written over what it last agreed.
/// `understood` says which of base's items this build can read: only those
/// can be found missing and deleted. `joining`: its items are stamped 0.
pub fn stamp(
    current: &Items,
    base: &Map,
    device: &str,
    now: f64,
    joining: bool,
    understood: &dyn Fn(&str, &str) -> bool,
) -> Map {
    let seen = base.values().map(|e| e.t).fold(0.0, f64::max);
    let clock = if joining { 0.0 } else { now.max(seen + 0.001) };
    let mut out = base.clone();
    for (key, value) in current {
        if base.get(key).and_then(|e| e.v.as_ref()) != Some(value) {
            out.insert(key.clone(), Entry { v: Some(value.clone()), t: clock, d: device.into() });
        }
    }
    // A whole kind gone at once is a file that didn't read or a browser
    // reset, far likelier than a choice: it comes back from the folder.
    let lost: Vec<&str> = WHOLE_KINDS
        .into_iter()
        .filter(|kind| {
            !current.keys().any(|k| k.starts_with(kind))
                && base.iter().filter(|(k, e)| k.starts_with(kind) && e.v.is_some()).count() >= WHOLE_KIND_FLOOR
        })
        .collect();
    for (key, entry) in base {
        let Some(value) = &entry.v else { continue };
        if current.contains_key(key) || !understood(key, value) || lost.iter().any(|k| key.starts_with(k)) {
            continue;
        }
        out.insert(key.clone(), Entry { v: None, t: clock, d: device.into() });
    }
    out
}

/// One pass: this PC's items stamped over its base (None before its first
/// pass), merged with the others', old deletions forgotten. The result is the
/// new base, this PC's file, and (its live items) what this PC should have.
pub fn pass(
    current: &Items,
    base: Option<&Map>,
    theirs: &[Map],
    device: &str,
    now: f64,
    understood: &dyn Fn(&str, &str) -> bool,
) -> Map {
    let empty = Map::new();
    let joining = base.is_none() && !theirs.is_empty();
    let mine = stamp(current, base.unwrap_or(&empty), device, now, joining, understood);
    let mut all = vec![mine];
    all.extend_from_slice(theirs);
    prune(merge(&all), now)
}

/// The items that are there, without the tombstones.
pub fn live(map: &Map) -> Items {
    map.iter().filter_map(|(k, e)| e.v.clone().map(|v| (k.clone(), v))).collect()
}

/// Deletions older than `TOMBSTONE_LIFE` forgotten.
pub fn prune(mut map: Map, now: f64) -> Map {
    map.retain(|_, e| e.v.is_some() || e.t >= now - TOMBSTONE_LIFE);
    map
}

/// Another PC's file, if this build can read it for this scope: damaged,
/// other-format and wrong-folder files are None, and an entry whose stamp is
/// not a number is left out.
pub fn decode(data: &[u8], scope: &str) -> Option<File> {
    let mut file: File = serde_json::from_slice(data).ok()?;
    if file.format != FORMAT || file.scope != scope || !is_id(&file.device) {
        return None;
    }
    file.items.retain(|_, e| e.t.is_finite() && e.t >= 0.0 && !e.d.is_empty());
    Some(file)
}

pub fn encode(file: &File) -> Vec<u8> {
    // Keys come out sorted (BTreeMap), so the same file is the same bytes.
    serde_json::to_vec(file).unwrap_or_default()
}

/// A value as an item's text: serde_json's maps are sorted, so the same value
/// is always the same text and "changed" means changed.
pub fn text<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn value<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_str(text).ok()
}

/// A list's order applied to the ids that are there: the order's first, then
/// the rest in `fallback`'s order, then any left by id, so an item added on
/// one PC while another reordered is kept, at the end, and every PC ends with
/// the same list.
pub fn ordered(ids: &BTreeSet<String>, order: &[String], fallback: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for id in order.iter().chain(fallback) {
        if ids.contains(id) && seen.insert(id.clone()) {
            out.push(id.clone());
        }
    }
    out.extend(ids.difference(&seen).cloned());
    out
}

/// An id that can stand in a key and a file name: letters, digits and '-',
/// as a UUID is. Never a path.
pub fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
