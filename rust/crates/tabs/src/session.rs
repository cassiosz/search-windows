//! What was open last time (upstream Session.swift): the addresses and their
//! names, and which one you were looking at, and nothing else; the rest is
//! on the page or in the history. The JSON is shaped like upstream's
//! session.json so the two read alike.

use crate::{Tab, Tabs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One tab (Session.Entry). Every field but the address is optional on the
/// way in: a key an older or newer version doesn't write is no reason to
/// lose the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Empty for a blank tab.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// A pin's letter. None for an ordinary tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// The name you gave the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A pin's own page, the one it was pinned at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// A pin's lasting id, for sync (this port's own field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_id: Option<String>,
    /// When you last left the tab, unix milliseconds. Upstream writes a
    /// Swift Date here; this file is this port's own, so it keeps it simple.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The whole file (Session.Shape).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    #[serde(default)]
    pub tabs: Vec<Entry>,
    /// The tab in front, by its place in `tabs`; -1 when none is marked.
    #[serde(default)]
    pub active: i64,
    /// Not used yet. Kept as read and written back as it was, so a session
    /// that has groups or splits (upstream's) isn't stripped of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splits: Vec<Value>,
}


impl Shape {
    /// What to write for the row as it stands.
    pub fn from_tabs(tabs: &Tabs) -> Shape {
        let entries = tabs
            .iter()
            .map(|t| Entry {
                url: t.url.clone().unwrap_or_default(),
                title: t.title.clone(),
                pin: t.pin_letter.clone().filter(|_| t.pinned),
                name: t.name.clone(),
                home: t.home.clone(),
                pin_id: t.pin_id.clone().filter(|_| t.pinned),
                touched: Some(t.last_active_ms),
                muted: t.muted,
            })
            .collect();
        let active = tabs.iter().position(|t| t.id == tabs.selected_id()).map_or(0, |i| i as i64);
        Shape { tabs: entries, active, groups: tabs.groups.clone(), splits: tabs.splits.clone() }
    }

    /// The row to open with. Tabs come back asleep, as last session's do,
    /// except the one you were on; nothing marked in front (or a place that
    /// isn't there) opens on an empty tab beside the pins. `now_ms` stands in
    /// for tabs written without their time: they start counting now.
    pub fn into_tabs(self, now_ms: u64) -> Tabs {
        let front = usize::try_from(self.active).ok();
        let (groups, splits) = (self.groups, self.splits);
        let mut list = Vec::with_capacity(self.tabs.len());
        for (place, e) in self.tabs.into_iter().enumerate() {
            let url = Some(e.url).filter(|u| !u.is_empty());
            list.push((
                Some(place) == front,
                Tab {
                    asleep: url.is_some(),
                    url,
                    title: e.title,
                    pinned: e.pin.is_some(),
                    pin_id: e.pin_id.filter(|_| e.pin.is_some()),
                    pin_letter: e.pin,
                    name: e.name,
                    home: e.home,
                    muted: e.muted,
                    last_active_ms: e.touched.unwrap_or(now_ms),
                    ..Tab::blank(0, now_ms)
                },
            ));
        }
        // Pins are first in the row; a file written by hand might not agree.
        // Stable, so each kind keeps its order.
        list.sort_by_key(|(_, t)| !t.pinned);
        let active = list.iter().position(|(f, _)| *f);
        let mut tabs = Tabs::from_parts(list.into_iter().map(|(_, t)| t).collect(), active, now_ms);
        tabs.groups = groups;
        tabs.splits = splits;
        tabs
    }

    /// Settings › General › Start with a fresh window (Session.fresh): what
    /// you said to keep stays (pins and named tabs) and nothing is in
    /// front, so the window opens on an empty tab beside the pins.
    pub fn fresh(&self) -> Shape {
        let tabs = self.tabs.iter().filter(|e| e.pin.is_some() || e.name.is_some()).cloned().collect();
        Shape { tabs, active: -1, groups: None, splits: Vec::new() }
    }
}

/// Writes the session without ever leaving a half-written file where the
/// last good one was: to a temporary file beside it, then renamed over.
pub fn save(path: &Path, shape: &Shape) -> std::io::Result<()> {
    let data = serde_json::to_vec(shape).map_err(std::io::Error::other)?;
    let temp = beside(path, ".tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(&data)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Reads it back. No file, or one that won't decode, is an empty session,
/// never a failure. A file that is there but won't decode is not the same as
/// none, though: it is moved aside to `<name>.bad` rather than overwritten by
/// the next save without a trace (Session.read quarantines it too).
pub fn load(path: &Path) -> Shape {
    let Ok(data) = std::fs::read(path) else { return Shape::default() };
    serde_json::from_slice(&data).unwrap_or_else(|_| {
        let _ = std::fs::rename(path, beside(path, ".bad"));
        Shape::default()
    })
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
