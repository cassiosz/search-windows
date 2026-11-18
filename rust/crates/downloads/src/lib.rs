//! What is being fetched and what you have kept: the model behind a downloads
//! page. Port of the model parts of upstream Sources/Search/Fetching.swift
//! (a download's states, progress, pause/resume/cancel) and Loot.swift (the
//! remembered list, `downloads.json`). Pure logic: no window, no WebView2.
//!
//! The app maps WebView2's ICoreWebView2DownloadOperation onto this: it calls
//! `begin` on DownloadStarting, `progress` on BytesReceivedChanged,
//! `set_paused`/`finish`/`fail` on StateChanged, and acts on the `can_*`
//! answers before calling Pause/Resume/Cancel on the operation. Showing a file
//! in its folder is the app's job.
//!
//! Time is passed in as milliseconds since the Unix epoch.

mod names;
#[cfg(test)]
mod tests;

pub use names::{free_name, free_path, sanitize_file_name};

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use url::Url;

/// Finished downloads remembered across runs: "fifty is more than anybody
/// scrolls back through" (Loot.add).
pub const KEEP: usize = 50;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    InProgress,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl State {
    /// Still under way: counts toward the downloads button's pulse and
    /// is never dropped from the list.
    pub fn active(self) -> bool {
        matches!(self, State::InProgress | State::Paused)
    }
}

/// What a page's button asks for; the app turns it into a call on the
/// WebView2 operation, then tells the model what happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Pause,
    Resume,
    Cancel,
}

/// One row of the downloads page. The file keeps these (done ones only), in
/// upstream's `Keep` spirit: name, host it came from, path, date, address.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    /// Where it came from, for Copy Address: http(s) only and without a
    /// password, as `Keep.address(of:)`; empty when it means nothing outside
    /// its page (blob:, data:) or was forgotten with the history.
    #[serde(default)]
    pub url: String,
    /// The file's name as it reads in the list.
    #[serde(default)]
    pub name: String,
    /// Full path of the file on disk (where it goes, while under way).
    #[serde(default)]
    pub path: String,
    /// The site it came from (`Keep.from`).
    #[serde(default)]
    pub from: String,
    /// Bytes expected; 0 while unknown.
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub received: u64,
    pub state: State,
    #[serde(default)]
    pub started_at: u64,
    /// Why it failed, for the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// False for a private tab's: shown while under way, never written to the
    /// file (upstream's `unlisted`).
    #[serde(skip, default = "listed")]
    pub listed: bool,
}

fn listed() -> bool {
    true
}

impl Entry {
    /// 0.0..=1.0, or None while the size is not known (an indeterminate bar).
    pub fn fraction(&self) -> Option<f64> {
        (self.total > 0).then(|| (self.received as f64 / self.total as f64).clamp(0.0, 1.0))
    }

    pub fn can_pause(&self) -> bool {
        self.state == State::InProgress
    }

    pub fn can_resume(&self) -> bool {
        self.state == State::Paused
    }

    pub fn can_cancel(&self) -> bool {
        self.state.active()
    }
}

/// The address as another app can fetch it: only http(s), no user or
/// password (`Keep.address(of:)`).
pub fn address_of(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) if matches!(u.scheme(), "http" | "https") => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.into()
        }
        _ => String::new(),
    }
}

#[derive(Default)]
pub struct Downloads {
    /// Newest first.
    entries: Vec<Entry>,
    next: u64,
}

impl Downloads {
    pub fn new() -> Downloads {
        Downloads { entries: Vec::new(), next: 1 }
    }

    /// Reads the file. Missing or corrupt is empty: never a panic, never an
    /// error to show. Only finished downloads are ever in it, but a hand-edited
    /// file is held to that and to `KEEP`.
    pub fn load(path: &Path) -> Downloads {
        let list = std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<Vec<Entry>>(&data).ok())
            .unwrap_or_default();
        let mut d = Downloads::new();
        for mut e in list.into_iter().filter(|e| e.state == State::Done).take(KEEP) {
            e.listed = true;
            d.next = d.next.max(e.id + 1);
            d.entries.push(e);
        }
        d
    }

    /// Atomic (temp file, then rename). Only finished, listed downloads go in
    /// the file: one under way means nothing after a restart, and a private
    /// tab's is never kept.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let kept: Vec<&Entry> =
            self.entries.iter().filter(|e| e.state == State::Done && e.listed).take(KEEP).collect();
        let data = serde_json::to_vec(&kept).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        let temp = Path::new(&temp);
        let result = (|| {
            let mut file = std::fs::File::create(temp)?;
            file.write_all(&data)?;
            file.sync_all()?;
            std::fs::rename(temp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// How many are under way (the button's pulse, `FetchDoor.shows`).
    pub fn active(&self) -> usize {
        self.entries.iter().filter(|e| e.state.active()).count()
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// A download has started: at the top of the list. `path` is the full
    /// destination (see `free_path`); the name is its last component.
    /// `listed` false for a private tab.
    pub fn begin(&mut self, url: &str, path: &str, total: u64, listed: bool, now: u64) -> u64 {
        let id = self.next;
        self.next += 1;
        let name = Path::new(path).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let from = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        self.entries.insert(
            0,
            Entry {
                id,
                url: address_of(url),
                name,
                path: path.to_string(),
                from,
                total,
                received: 0,
                state: State::InProgress,
                started_at: now,
                error: None,
                listed,
            },
        );
        id
    }

    /// Bytes so far, from the progress event. Only while under way, and only
    /// a change is reported (each one draws the row again, as in
    /// `FetchEntry.record`): true when the row changed.
    pub fn progress(&mut self, id: u64, received: u64, total: u64) -> bool {
        let Some(e) = self.get_mut(id).filter(|e| e.state.active()) else { return false };
        // A total of 0 is "unknown": it never erases one that was known.
        let total = if total > 0 { total } else { e.total };
        let changed = e.received != received || e.total != total;
        e.received = received;
        e.total = total;
        changed
    }

    /// The operation went paused (true) or on again (false), as its state
    /// event says. Ignored once ended.
    pub fn set_paused(&mut self, id: u64, paused: bool) -> bool {
        let Some(e) = self.get_mut(id).filter(|e| e.state.active()) else { return false };
        let now = if paused { State::Paused } else { State::InProgress };
        let changed = e.state != now;
        e.state = now;
        changed
    }

    /// It is complete. A file listed before at the same path gives way to
    /// this one (`Loot.add`), and the list is cut to `KEEP` finished ones.
    pub fn finish(&mut self, id: u64) -> bool {
        let Some(e) = self.get_mut(id).filter(|e| e.state.active()) else { return false };
        e.state = State::Done;
        e.error = None;
        if e.total == 0 || e.received > e.total {
            e.total = e.received;
        } else {
            e.received = e.total;
        }
        let path = e.path.clone();
        self.entries.retain(|o| o.id == id || !(o.state == State::Done && o.path == path));
        self.trim();
        true
    }

    /// The final location, when the operation reports one different from the
    /// destination first given (the name shown follows it, `reached`).
    pub fn set_path(&mut self, id: u64, path: &str) -> bool {
        let Some(e) = self.get_mut(id) else { return false };
        e.path = path.to_string();
        e.name = Path::new(path).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        true
    }

    pub fn fail(&mut self, id: u64, reason: &str) -> bool {
        let Some(e) = self.get_mut(id).filter(|e| e.state.active()) else { return false };
        e.state = State::Failed;
        e.error = Some(reason.to_string());
        true
    }

    /// Whether the page's button may ask for this now. The app asks the
    /// operation, and tells the model the outcome (`set_paused`, `cancel`).
    pub fn allows(&self, id: u64, intent: Intent) -> bool {
        self.get(id).is_some_and(|e| match intent {
            Intent::Pause => e.can_pause(),
            Intent::Resume => e.can_resume(),
            Intent::Cancel => e.can_cancel(),
        })
    }

    /// It was cancelled (by the intent, or by the destination dialog).
    pub fn cancel(&mut self, id: u64) -> bool {
        let Some(e) = self.get_mut(id).filter(|e| e.state.active()) else { return false };
        e.state = State::Cancelled;
        true
    }

    /// Takes one out of the list. Under way ones are refused: cancel first.
    /// The file on disk is yours (Loot.forgetAll: "deleting them is the
    /// Finder's business, not a browser's").
    pub fn forget(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id || e.state.active());
        self.entries.len() != before
    }

    /// "Clear": every finished, failed and cancelled row goes; what is still
    /// under way stays. Returns how many went.
    pub fn clear_finished(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.state.active());
        before - self.entries.len()
    }

    /// History cleared: where files came from goes too, from the list and
    /// from what is still coming in (`forgetDownloadAddresses`). Returns
    /// whether anything changed, so the file needs writing.
    pub fn forget_addresses(&mut self) -> bool {
        let mut any = false;
        for e in self.entries.iter_mut().filter(|e| !e.url.is_empty()) {
            e.url.clear();
            any = true;
        }
        any
    }

    /// Finished ones past `KEEP` go, oldest first; ones under way never.
    fn trim(&mut self) {
        let mut done = 0;
        self.entries.retain(|e| {
            if e.state != State::Done {
                return true;
            }
            done += 1;
            done <= KEEP
        });
    }
}
