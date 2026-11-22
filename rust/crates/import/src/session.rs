//! The tabs a Chromium browser has open, and their groups, read from the
//! newest "Session_*" in a profile's Sessions folder, or the older "Current
//! Session" beside it (ImportSession.swift). Read whole into memory through a
//! sharing handle, so a running browser still writing it is never disturbed.
//!
//! SNSS: "SNSS", a version, then frames — a u16 size, a command id and its
//! bytes — appended as the browser goes. Only the commands that say what is
//! open are read; any other is passed over by its size. A torn frame at the
//! end ends the read; a file that isn't SNSS, or is encrypted, has nothing.
//! Recently closed tabs ("Tabs_*") and saved groups are left behind.

use crate::browsers::{Browser, Engine, read_capped};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// At most this many tabs come over, so a session of thousands doesn't make
/// thousands of tabs at once.
pub const LIMIT: usize = 300;

/// A tab group's id in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub high: u64,
    pub low: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    /// Never set on a pin.
    pub group: Option<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Each open normal window's tabs, in their order there.
    pub windows: Vec<Vec<OpenTab>>,
    /// The groups those tabs are in. Only ones with tabs.
    pub groups: HashMap<Token, Group>,
    /// How many there were before the limit.
    pub found: usize,
}

impl Snapshot {
    pub fn tabs(&self) -> usize {
        self.windows.iter().map(Vec::len).sum()
    }

    /// No more than `limit` tabs, window by window, and only the groups those
    /// still hold.
    pub fn capped(&self, limit: usize) -> Snapshot {
        let mut out = Snapshot { found: self.found.max(self.tabs()), ..Snapshot::default() };
        let mut room = limit;
        for window in &self.windows {
            if room == 0 {
                break;
            }
            let kept: Vec<OpenTab> = window.iter().take(room).cloned().collect();
            room -= kept.len();
            out.windows.push(kept);
        }
        out.keep_groups(&self.groups);
        out
    }

    /// Only the tabs `keep` says yes to (a re-import: those not brought
    /// before), and the groups that still have one.
    pub fn retain(&mut self, mut keep: impl FnMut(&OpenTab) -> bool) {
        for window in &mut self.windows {
            window.retain(&mut keep);
        }
        self.windows.retain(|w| !w.is_empty());
        let groups = std::mem::take(&mut self.groups);
        self.keep_groups(&groups);
    }

    fn keep_groups(&mut self, from: &HashMap<Token, Group>) {
        let used: HashSet<Token> = self.windows.iter().flatten().filter_map(|t| t.group).collect();
        self.groups = used.into_iter().filter_map(|t| Some((t, from.get(&t)?.clone()))).collect();
    }

    /// Opened behind the tab in front, in order, pins pinned. Tabs has no
    /// groups: a group's tabs come as plain tabs. Returns how many opened.
    pub fn open_into(&self, tabs: &mut searchwin_tabs::Tabs) -> usize {
        let mut opened = 0;
        for tab in self.windows.iter().flatten() {
            let id = tabs.open_in_background(&tab.url, None);
            if let Some(t) = tabs.get_mut(id) {
                t.title = tab.title.clone();
            }
            if tab.pinned {
                tabs.pin(id);
            }
            opened += 1;
        }
        opened
    }
}

/// The open tabs of one profile, or every one's, at most LIMIT. Firefox's
/// session file is compressed jsonlz4 and isn't read: an empty snapshot.
pub fn open_tabs(browser: &Browser, profile: Option<&str>) -> Snapshot {
    let mut all = Snapshot::default();
    if browser.engine != Engine::Chromium {
        return all;
    }
    for p in browser.profiles(profile) {
        let Some(data) = session_file(&p.path).and_then(|f| read_capped(&f, 64 << 20)) else { continue };
        let read = read(&data);
        all.windows.extend(read.windows);
        all.found += read.found;
        // Tokens are random per group, so two profiles never share one.
        for (k, v) in read.groups {
            all.groups.entry(k).or_insert(v);
        }
    }
    all.capped(LIMIT)
}

/// The newest "Session_*" in Sessions, else "Current Session".
fn session_file(profile: &Path) -> Option<PathBuf> {
    let newest = std::fs::read_dir(profile.join("Sessions")).ok().and_then(|dir| {
        dir.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("Session_"))
            .filter_map(|e| {
                let meta = e.metadata().ok().filter(|m| m.is_file())?;
                Some((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), e.file_name(), e.path()))
            })
            .max()
            .map(|(_, _, path)| path)
    });
    newest.or_else(|| Some(profile.join("Current Session")).filter(|p| p.is_file()))
}

// The commands that say what is open (components/sessions in Chromium).
const WINDOW: u8 = 0;
const INDEX: u8 = 2;
const NAVIGATION: u8 = 6;
const SELECTED_NAVIGATION: u8 = 7;
const WINDOW_TYPE: u8 = 9;
const PINNED: u8 = 12;
const TAB_CLOSED: u8 = 16;
const WINDOW_CLOSED: u8 = 17;
const PRUNED: u8 = 24;
const GROUP: u8 = 25;
const GROUP_METADATA: u8 = 27;

#[derive(Default)]
struct State {
    window_of: HashMap<i32, i32>,
    window_tabs: HashMap<i32, HashSet<i32>>,
    window_order: Vec<i32>,
    window_type: HashMap<i32, i32>,
    tab_index: HashMap<i32, i32>,
    tab_seen: HashMap<i32, usize>,
    entries: HashMap<i32, HashMap<i32, (String, String)>>,
    selected: HashMap<i32, i32>,
    pinned: HashSet<i32>,
    tab_group: HashMap<i32, Token>,
    meta: HashMap<Token, Group>,
}

impl State {
    fn note(&mut self, tab: i32) {
        let n = self.tab_seen.len();
        self.tab_seen.entry(tab).or_insert(n);
    }

    fn note_window(&mut self, window: i32) {
        if !self.window_order.contains(&window) {
            self.window_order.push(window);
        }
    }

    fn close(&mut self, tab: i32) {
        if let Some(w) = self.window_of.remove(&tab)
            && let Some(set) = self.window_tabs.get_mut(&w)
        {
            set.remove(&tab);
        }
        self.tab_index.remove(&tab);
        self.entries.remove(&tab);
        self.selected.remove(&tab);
        self.pinned.remove(&tab);
        self.tab_group.remove(&tab);
    }
}

/// The open windows a session file holds.
pub fn read(data: &[u8]) -> Snapshot {
    let mut file = Cursor::new(data);
    // The magic, then a version this reads: 1, or 3.
    if file.bytes(4) != Some(b"SNSS") || !matches!(file.i32(), Some(1 | 3)) {
        return Snapshot::default();
    }
    let mut s = State::default();
    while file.left() > 0 {
        let Some(size) = file.u16().map(usize::from).filter(|&n| n >= 1 && n <= file.left()) else { break };
        let (Some(id), Some(body)) = (file.u8(), file.bytes(size - 1)) else { break };
        let mut f = Cursor::new(body);
        match id {
            WINDOW => {
                if let (Some(window), Some(tab)) = (f.i32(), f.i32()) {
                    if let Some(old) = s.window_of.get(&tab).copied()
                        && let Some(set) = s.window_tabs.get_mut(&old)
                    {
                        set.remove(&tab);
                    }
                    s.window_of.insert(tab, window);
                    s.window_tabs.entry(window).or_default().insert(tab);
                    s.note_window(window);
                    s.note(tab);
                }
            }
            INDEX => {
                if let (Some(tab), Some(index)) = (f.i32(), f.i32()) {
                    s.tab_index.insert(tab, index);
                    s.note(tab);
                }
            }
            NAVIGATION => {
                // A pickle: its size, the tab, the entry's index, its
                // address, its title, then page state not needed.
                f.skip(4);
                if let (Some(tab), Some(index), Some(url), Some(title)) = (f.i32(), f.i32(), f.utf8(), f.utf16()) {
                    s.entries.entry(tab).or_default().insert(index, (url, title));
                    s.note(tab);
                }
            }
            SELECTED_NAVIGATION => {
                if let (Some(tab), Some(index)) = (f.i32(), f.i32()) {
                    s.selected.insert(tab, index);
                    s.note(tab);
                }
            }
            WINDOW_TYPE => {
                if let (Some(window), Some(kind)) = (f.i32(), f.i32()) {
                    s.window_type.insert(window, kind);
                    s.note_window(window);
                }
            }
            PINNED => {
                if let (Some(tab), Some(state)) = (f.i32(), f.u8()) {
                    if state != 0 {
                        s.pinned.insert(tab);
                    } else {
                        s.pinned.remove(&tab);
                    }
                    s.note(tab);
                }
            }
            TAB_CLOSED => {
                if let Some(tab) = f.i32() {
                    s.close(tab);
                }
            }
            WINDOW_CLOSED => {
                if let Some(window) = f.i32() {
                    s.window_type.remove(&window);
                    for tab in s.window_tabs.remove(&window).unwrap_or_default() {
                        s.close(tab);
                    }
                }
            }
            PRUNED => {
                // Entries [index, index + count) are gone and the ones after
                // move down by count, as Chromium does.
                if let (Some(tab), Some(index), Some(count)) = (f.i32(), f.i32(), f.i32())
                    && index >= 0
                    && count > 0
                    && let Some(navs) = s.entries.get(&tab)
                {
                    let (from, end) = (i64::from(index), i64::from(index) + i64::from(count));
                    let moved = |at: i64| -> Option<i32> {
                        if at < from {
                            Some(at as i32)
                        } else if at >= end {
                            Some((at - i64::from(count)) as i32)
                        } else {
                            None
                        }
                    };
                    let kept: HashMap<i32, (String, String)> =
                        navs.iter().filter_map(|(&at, e)| Some((moved(i64::from(at))?, e.clone()))).collect();
                    s.entries.insert(tab, kept);
                    if let Some(&current) = s.selected.get(&tab) {
                        match moved(i64::from(current)).or(if index > 0 { Some(index - 1) } else { None }) {
                            Some(to) => s.selected.insert(tab, to),
                            None => s.selected.remove(&tab),
                        };
                    }
                }
            }
            GROUP => {
                // A raw struct: the tab, padding to the token's alignment, the
                // token, and whether it is in a group at all.
                if let Some(tab) = f.i32() {
                    f.skip(4);
                    if let (Some(high), Some(low)) = (f.u64(), f.u64()) {
                        if f.u8().unwrap_or(0) == 0 {
                            s.tab_group.remove(&tab);
                        } else {
                            s.tab_group.insert(tab, Token { high, low });
                        }
                    }
                    s.note(tab);
                }
            }
            GROUP_METADATA => {
                // A pickle: its size, the token, the name, the colour, then
                // (in newer files) whether it is folded.
                f.skip(4);
                if let (Some(high), Some(low), Some(title)) = (f.u64(), f.u64(), f.utf16()) {
                    f.skip(4);
                    let collapsed = f.i32().is_some_and(|c| c != 0);
                    s.meta.insert(Token { high, low }, Group { title, collapsed });
                }
            }
            _ => {}
        }
    }

    let mut snapshot = Snapshot::default();
    for window in &s.window_order {
        // Popups, apps and devtools aren't tabs to keep. A window never given
        // a type is a normal one.
        if s.window_type.get(window).is_some_and(|&t| t != 0) {
            continue;
        }
        let mut ids: Vec<i32> = s.window_tabs.get(window).map(|t| t.iter().copied().collect()).unwrap_or_default();
        ids.sort_by_key(|t| (s.tab_index.get(t).copied().unwrap_or(i32::MAX), s.tab_seen.get(t).copied().unwrap_or(0)));
        let tabs: Vec<OpenTab> = ids
            .into_iter()
            .filter_map(|tab| {
                let navs = s.entries.get(&tab).filter(|n| !n.is_empty())?;
                let at = s.selected.get(&tab).filter(|i| navs.contains_key(i)).or_else(|| navs.keys().max())?;
                let (url, title) = navs.get(at)?;
                let url = crate::read::web(url)?;
                let pinned = s.pinned.contains(&tab);
                Some(OpenTab {
                    title: titled(title, Some(&url)),
                    url,
                    pinned,
                    group: if pinned { None } else { s.tab_group.get(&tab).copied() },
                })
            })
            .collect();
        if !tabs.is_empty() {
            snapshot.windows.push(tabs);
        }
    }
    // Only groups that still have tabs; one never named gets a plain name.
    let mut unnamed = 0;
    let used: Vec<Token> = snapshot.windows.iter().flatten().filter_map(|t| t.group).collect();
    for token in used {
        if snapshot.groups.contains_key(&token) {
            continue;
        }
        let group = match s.meta.get(&token) {
            Some(found) if !found.title.trim().is_empty() => Group { title: titled(&found.title, None), collapsed: found.collapsed },
            other => {
                unnamed += 1;
                Group { title: format!("Group {unnamed}"), collapsed: other.is_some_and(|g| g.collapsed) }
            }
        };
        snapshot.groups.insert(token, group);
    }
    snapshot.found = snapshot.tabs();
    snapshot
}

/// A title fit to keep: trimmed, capped, never empty.
pub(crate) fn titled(title: &str, url: Option<&str>) -> String {
    let trimmed = title.trim();
    if !trimmed.is_empty() {
        return trimmed.chars().take(300).collect();
    }
    url.map(|u| url::Url::parse(u).ok().and_then(|p| p.host_str().map(String::from)).unwrap_or_else(|| u.to_string()))
        .unwrap_or_else(|| "Untitled".into())
}

/// Bytes a frame at a time, little-endian; a read past the end is None.
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Cursor<'a> {
        Cursor { data, at: 0 }
    }

    fn left(&self) -> usize {
        self.data.len() - self.at
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(out)
    }

    fn skip(&mut self, n: usize) {
        self.at += n.min(self.left());
    }

    fn u8(&mut self) -> Option<u8> {
        self.bytes(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.bytes(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn i32(&mut self) -> Option<i32> {
        self.bytes(4).map(|b| i32::from_le_bytes(b.try_into().unwrap_or_default()))
    }

    fn u64(&mut self) -> Option<u64> {
        self.bytes(8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()))
    }

    /// A pickle string: a byte count, the bytes, padding to four.
    fn utf8(&mut self) -> Option<String> {
        let n = usize::try_from(self.i32()?).ok()?;
        let bytes = self.bytes(n)?;
        self.skip((4 - n % 4) % 4);
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    /// A pickle string16: a unit count, UTF-16LE, padding to four. A lone
    /// surrogate becomes the replacement character.
    fn utf16(&mut self) -> Option<String> {
        let n = usize::try_from(self.i32()?).ok()?.checked_mul(2)?;
        let bytes = self.bytes(n)?;
        self.skip((4 - n % 4) % 4);
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        Some(String::from_utf16_lossy(&units))
    }
}
