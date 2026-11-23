//! The tabs, in the order the row shows them, and what happens to the row
//! when one is opened, closed, pinned or moved. Pure logic: no window, no
//! page. The app tells it what happened and does what it answers.
//!
//! Time is the caller's: `Tabs::set_now` takes unix milliseconds, and every
//! "last looked at" and the session file count on that one clock.

mod session;
mod sleep;
#[cfg(test)]
mod tests;

pub use session::{Entry, Shape, load, save};
pub use sleep::{SleepPace, WAIT_ON_PRESSURE_CRITICAL_MS, WAIT_ON_PRESSURE_WARNING_MS};

/// A pin as sync sees it (upstream PinDef): what it is, not what it shows now.
#[derive(Clone, Debug, PartialEq)]
pub struct PinDef {
    pub id: String,
    pub letter: String,
    pub home: String,
    pub title: String,
    pub name: Option<String>,
}

/// How many closed tabs ⇧⌘T can go back through (Browser.swift `remember`:
/// "Twelve steps back").
const CLOSED_KEPT: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    /// Never reused within one `Tabs`, so a late answer from a page can tell
    /// its tab is gone.
    pub id: u64,
    /// None for a blank tab: nothing typed yet, nothing to keep.
    pub url: Option<String>,
    pub title: String,
    pub pinned: bool,
    /// The letter a pin shows (Tab.monogram), set when pinned.
    pub pin_letter: Option<String>,
    /// The name you gave it (Session.Entry.name). A named tab is one kept.
    pub name: Option<String>,
    /// A pin's own page, the one it was pinned at (Session.Entry.home).
    pub home: Option<String>,
    /// A pin's lasting id, the same on every PC sync reaches (PinDef.id).
    /// Given by the app when it first needs one; None for any other tab.
    pub pin_id: Option<String>,
    pub muted: bool,
    /// When it was last left, in the caller's clock (Tab.touched).
    pub last_active_ms: u64,
    /// The tab a link opened it from, if one did (Tab.opener).
    pub opener: Option<u64>,
    /// Its page given back (see sleep.rs); selecting it wakes it.
    pub asleep: bool,
    // What keeps a tab from sleeping (Browser.busy and friends). Set by the
    // app from what the page is doing.
    pub loading: bool,
    pub audible: bool,
    /// On a call, downloading, holding something typed, video out: anything
    /// a sleep would cut off.
    pub busy: bool,
    /// The site was chosen to be kept awake (Browser.keepsAwake).
    pub keep_awake: bool,
    /// The site may send notifications, which a page asleep couldn't.
    pub notifies: bool,
}

impl Tab {
    pub(crate) fn blank(id: u64, now: u64) -> Tab {
        Tab {
            id,
            url: None,
            title: String::new(),
            pinned: false,
            pin_letter: None,
            name: None,
            home: None,
            pin_id: None,
            muted: false,
            last_active_ms: now,
            opener: None,
            asleep: false,
            loading: false,
            audible: false,
            busy: false,
            keep_awake: false,
            notifies: false,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.url.is_none()
    }
}

/// Settings › Tabs, the ones that change where tabs land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewTabPlace {
    /// Beside the tab you are on (the default).
    AfterCurrent,
    Top,
    End,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub new_tab_place: NewTabPlace,
    /// Closing goes back to the tab viewed before, not the neighbour.
    pub close_goes_back: bool,
    /// Closing the last loose tab beside only pins leaves a new tab.
    pub new_tab_after_last: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { new_tab_place: NewTabPlace::AfterCurrent, close_goes_back: false, new_tab_after_last: false }
    }
}

/// A tab as it closed, for ⇧⌘T (Browser.Ghost).
#[derive(Clone, Debug, PartialEq)]
struct Ghost {
    url: String,
    title: String,
    index: usize,
    name: Option<String>,
    muted: bool,
}

/// Links opened behind one page, so a row of them reads in click order
/// (Browser.LinkRun).
#[derive(Clone, Debug)]
struct LinkRun {
    opener: u64,
    last: u64,
}

#[derive(Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    selected: u64,
    next_id: u64,
    now: u64,
    pub options: Options,
    closed: Vec<Ghost>,
    /// Tabs left, the most recently left last: what "back" means when a
    /// tab closes (TabSwitcher.lastLeft).
    left: Vec<u64>,
    run: Option<LinkRun>,
    /// The session file's `groups` and `splits`, which this port doesn't use
    /// yet: carried through load -> save untouched so a session shaped like
    /// upstream's isn't stripped of them (see `session::Shape`).
    pub(crate) groups: Option<serde_json::Value>,
    pub splits: Vec<serde_json::Value>,
}

impl Default for Tabs {
    fn default() -> Tabs {
        Tabs::new()
    }
}

impl Tabs {
    /// A window always has a tab: it opens on a blank one.
    pub fn new() -> Tabs {
        let mut tabs = Tabs::empty();
        tabs.selected = tabs.push_blank();
        tabs
    }

    fn empty() -> Tabs {
        Tabs {
            tabs: Vec::new(),
            selected: 0,
            next_id: 1,
            now: 0,
            options: Options::default(),
            closed: Vec::new(),
            left: Vec::new(),
            run: None,
            groups: None,
            splits: Vec::new(),
        }
    }

    /// Rebuilds a row from a list of tabs (session restore). `tabs` ids are
    /// reassigned; `selected` is an index into it. Empty gives a blank tab.
    pub(crate) fn from_parts(list: Vec<Tab>, selected: Option<usize>, now: u64) -> Tabs {
        // Not `Tabs::new()`: that makes a blank tab and spends id 1 on it.
        let mut t = Tabs::empty();
        t.now = now;
        for mut tab in list {
            tab.id = t.next_id;
            t.next_id += 1;
            t.tabs.push(tab);
        }
        match selected.filter(|&i| i < t.tabs.len()) {
            Some(i) => {
                t.selected = t.tabs[i].id;
                t.tabs[i].asleep = false;
            }
            // Nothing was in front (a fresh start) or nothing is left: the
            // window opens on an empty tab beside the pins (Browser.restoreSession).
            None => t.selected = t.push_blank_at(t.tabs.len()),
        }
        t
    }

    fn push_blank(&mut self) -> u64 {
        self.push_blank_at(self.tabs.len())
    }

    fn push_blank_at(&mut self, at: usize) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.insert(at, Tab::blank(id, self.now));
        id
    }

    /// The caller's clock. Call before anything that should stamp a time.
    pub fn set_now(&mut self, ms: u64) {
        self.now = ms;
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    // ---- reading ----

    /// The tabs in display order: pins first.
    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn selected(&self) -> &Tab {
        // `selected` always names a tab in the row: every path that removes
        // one picks the next first.
        self.get(self.selected).expect("selected tab is in the row")
    }

    pub fn selected_id(&self) -> u64 {
        self.selected
    }

    pub fn pinned_count(&self) -> usize {
        self.tabs.iter().take_while(|t| t.pinned).count()
    }

    /// What ⇧⌘T would bring back, if anything.
    pub fn can_reopen(&self) -> bool {
        !self.closed.is_empty()
    }

    // ---- where tabs go ----

    /// Where a new tab goes: beside the tab you are on, but never among the
    /// pins — from a pin, it comes first after them (Browser.placeForNew).
    fn place_for_new(&self) -> usize {
        match self.options.new_tab_place {
            NewTabPlace::Top => self.pinned_count(),
            NewTabPlace::End => self.tabs.len(),
            NewTabPlace::AfterCurrent => {
                let here = self.index_of(self.selected).unwrap_or(self.tabs.len());
                (here + 1).max(self.pinned_count()).min(self.tabs.len())
            }
        }
    }

    /// ⌘T's tab and a page a link opens in front: the end of the row, unless
    /// the top was chosen (Browser.placeForFront).
    fn place_for_front(&self) -> usize {
        if self.options.new_tab_place == NewTabPlace::Top { self.pinned_count() } else { self.tabs.len() }
    }

    /// A link opened behind its page goes right after it, and after the links
    /// it already opened, so a row of them reads in click order
    /// (Browser.placeForLink).
    fn place_for_link(&self, opener: u64) -> usize {
        let Some(here) = self.index_of(opener) else { return self.place_for_new() };
        let mut anchor = here;
        if let Some(run) = &self.run
            && run.opener == opener
            && let Some(last) = self.index_of(run.last)
            && last > here
        {
            anchor = last;
        }
        (anchor + 1).max(self.pinned_count())
    }

    // ---- opening ----

    /// ⌘T. An empty tab already there is reused, moved to where a new one
    /// goes: repeated ⌘T must not leave a row of empty tabs (Browser.newTab).
    pub fn new_tab(&mut self) -> u64 {
        let blank = self.tabs.iter().rposition(|t| t.is_blank() && !t.pinned);
        let id = match blank {
            Some(from) => {
                let id = self.tabs[from].id;
                let to = self.place_for_front();
                let tab = self.tabs.remove(from);
                // Removing the old slot first shifts everything after it.
                self.tabs.insert(if from < to { to - 1 } else { to }, tab);
                id
            }
            None => {
                let at = self.place_for_front();
                self.push_blank_at(at)
            }
        };
        self.select(id);
        id
    }

    /// A link opened in front: at the end of the row (Browser.open with
    /// foreground), and in front of you.
    pub fn open_in_front(&mut self, url: &str, opener: Option<u64>) -> u64 {
        let at = self.place_for_front();
        let id = self.push_blank_at(at);
        self.fill(id, url, opener);
        self.select(id);
        id
    }

    /// A link opened behind its page (⌘-click, middle button): after the
    /// opener and the links it already opened. `opener` None means the tab in
    /// front. The selection stays where it is.
    pub fn open_in_background(&mut self, url: &str, opener: Option<u64>) -> u64 {
        let opener = opener.filter(|&o| self.get(o).is_some()).unwrap_or(self.selected);
        let at = self.place_for_link(opener);
        let id = self.push_blank_at(at);
        self.fill(id, url, Some(opener));
        self.run = Some(LinkRun { opener, last: id });
        id
    }

    fn fill(&mut self, id: u64, url: &str, opener: Option<u64>) {
        let tab = self.get_mut(id).expect("just inserted");
        tab.url = Some(url.to_string());
        tab.opener = opener;
    }

    // ---- selecting ----

    /// Goes to a tab, waking it. The one left is stamped with the time and
    /// remembered for "back"; the run of background links ends, as it does
    /// when you go to another tab.
    pub fn select(&mut self, id: u64) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        if id != self.selected {
            let now = self.now;
            let old = self.selected;
            if let Some(t) = self.get_mut(old) {
                t.last_active_ms = now;
            }
            self.left.retain(|&l| l != old && l != id);
            self.left.push(old);
            self.run = None;
            self.selected = id;
        }
        let t = self.get_mut(id).expect("checked");
        t.asleep = false;
        true
    }

    /// Ctrl+1..8 pick that place in the row; 9 is the last tab, whatever
    /// number it has (App.swift: `number == 9 ? last : number - 1`).
    pub fn select_number(&mut self, n: usize) -> Option<u64> {
        let index = match n {
            1..=8 => n - 1,
            9 => self.tabs.len().checked_sub(1)?,
            _ => return None,
        };
        let id = self.tabs.get(index)?.id;
        self.select(id);
        Some(id)
    }

    /// Ctrl+Tab: the next tab in the row, round to the first again.
    pub fn select_next(&mut self) -> u64 {
        self.step(1)
    }

    /// Ctrl+Shift+Tab.
    pub fn select_previous(&mut self) -> u64 {
        self.step(-1)
    }

    fn step(&mut self, direction: isize) -> u64 {
        let n = self.tabs.len() as isize;
        let here = self.index_of(self.selected).unwrap_or(0) as isize;
        let id = self.tabs[(here + direction).rem_euclid(n) as usize].id;
        self.select(id);
        id
    }

    /// The tab left last that is still in the row, other than `id`
    /// (Browser.tabBefore).
    fn tab_before(&self, id: u64) -> Option<u64> {
        self.left.iter().rev().copied().find(|&l| l != id && self.get(l).is_some())
    }

    // ---- closing ----

    /// Whether closing this tab closes the window: the last tab, when it is
    /// blank (Browser.close). The app checks before calling `close`.
    pub fn closes_window(&self, id: u64) -> bool {
        self.tabs.len() == 1 && self.get(id).is_some_and(|t| t.is_blank() && !t.pinned)
    }

    /// ⌘W. Returns the tab now selected when closing changed the selection
    /// (None when a background tab went, or nothing changed).
    pub fn close(&mut self, id: u64) -> Option<u64> {
        let index = self.index_of(id)?;

        // A pinned tab is not closed by ⌘W — it is put down. The page is let
        // go, and you land on whatever you were looking at before.
        if self.tabs[index].pinned {
            let was = self.selected == id;
            self.tabs[index].asleep = true;
            return if was { Some(self.leave(id)) } else { None };
        }

        if self.tabs.len() == 1 {
            if self.tabs[0].is_blank() {
                return None;
            }
            // The last tab leaves a blank one behind.
            self.remember(index);
            self.left.clear();
            self.run = None;
            self.tabs.clear();
            self.selected = self.push_blank();
            return Some(self.selected);
        }

        // The last loose tab beside only pins: with the switch on, a new tab
        // takes its place rather than a pin waking (Browser.close, onlyPins).
        let only_pins = self.options.new_tab_after_last && !self.tabs.iter().any(|t| t.id != id && !t.pinned);
        if self.selected == id && only_pins {
            if self.tabs[index].is_blank() {
                return None;
            }
            self.remember(index);
            self.tabs.remove(index);
            self.forget(id);
            self.selected = self.push_blank_at(index);
            return Some(self.selected);
        }

        let opener = self.tabs[index].opener;
        self.remember(index);
        self.tabs.remove(index);
        self.forget(id);
        if self.selected != id {
            return None;
        }
        // The tab viewed before, when asked for; a page a link opened goes
        // back to the page it came from while that is still the tab you were
        // on before it; otherwise the neighbour on the right, or the last
        // one if there is no right.
        let back = self.tab_before(id);
        let before = if self.options.close_goes_back {
            back
        } else {
            back.filter(|&b| Some(b) == opener)
        };
        let next = before.unwrap_or(self.tabs[index.min(self.tabs.len() - 1)].id);
        self.land(next);
        Some(next)
    }

    /// Onto a tab when the one in front is already gone from the row.
    fn land(&mut self, id: u64) {
        self.selected = id;
        self.run = None;
        if let Some(t) = self.get_mut(id) {
            t.asleep = false;
        }
    }

    /// Off a pin just put down onto the tab looked at last. Ordinary tabs
    /// first: falling back to any recent tab meant closing one pin landed you
    /// on another pin, and ⌘W bounced between the two (Browser.leave).
    fn leave(&mut self, id: u64) -> u64 {
        let awake = |t: &&Tab| t.id != id && !t.asleep;
        let loose = self.tabs.iter().filter(awake).filter(|t| !t.pinned).max_by_key(|t| t.last_active_ms);
        let any = self.tabs.iter().filter(awake).max_by_key(|t| t.last_active_ms);
        match loose.or(any).map(|t| t.id) {
            Some(back) => {
                self.select(back);
                back
            }
            None => self.new_tab(),
        }
    }

    fn forget(&mut self, id: u64) {
        self.left.retain(|&l| l != id);
        if self.run.as_ref().is_some_and(|r| r.opener == id || r.last == id) {
            self.run = None;
        }
    }

    /// Kept for ⇧⌘T, newest last, twelve steps back. A blank tab has nothing
    /// to come back to (Browser.remember).
    fn remember(&mut self, index: usize) {
        let t = &self.tabs[index];
        let Some(url) = t.url.clone() else { return };
        self.closed.push(Ghost { url, title: t.title.clone(), index, name: t.name.clone(), muted: t.muted });
        if self.closed.len() > CLOSED_KEPT {
            self.closed.remove(0);
        }
    }

    /// ⇧⌘T: the tab closed last, back at its place and in front. It keeps
    /// its name; the place is never among the pins (Browser.reopen).
    pub fn reopen_closed(&mut self) -> Option<u64> {
        let ghost = self.closed.pop()?;
        let at = ghost.index.max(self.pinned_count()).min(self.tabs.len());
        let id = self.push_blank_at(at);
        let t = self.get_mut(id).expect("just inserted");
        t.url = Some(ghost.url);
        t.title = ghost.title;
        t.name = ghost.name;
        t.muted = ghost.muted;
        self.select(id);
        Some(id)
    }

    // ---- pins ----

    /// Pinned tabs live at the head of the row, in the order they were
    /// pinned, so their letters never move under your hand (Browser.pin).
    pub fn pin(&mut self, id: u64) -> bool {
        let Some(from) = self.index_of(id) else { return false };
        if self.tabs[from].pinned {
            return true;
        }
        let mut tab = self.tabs.remove(from);
        tab.pinned = true;
        tab.pin_letter = Some(monogram(tab.url.as_deref()));
        tab.home = tab.url.clone();
        let at = self.pinned_count();
        self.tabs.insert(at, tab);
        true
    }

    /// Back out of the pinned block, to the head of the loose tabs
    /// (Browser.unpin). It forgets its letter and home.
    pub fn unpin(&mut self, id: u64) -> bool {
        let Some(from) = self.index_of(id) else { return false };
        if !self.tabs[from].pinned {
            return true;
        }
        let mut tab = self.tabs.remove(from);
        tab.pinned = false;
        tab.pin_letter = None;
        tab.home = None;
        tab.pin_id = None;
        let at = self.pinned_count();
        self.tabs.insert(at, tab);
        true
    }

    /// The row's pins as sync sees them, in order, each given an id first if
    /// it has none (`new_id`). `skip` tabs (private ones) are left out. The
    /// flag says an id was given, so the row is to be written.
    pub fn pin_defs(&mut self, skip: &dyn Fn(u64) -> bool, new_id: &mut dyn FnMut() -> String) -> (Vec<PinDef>, bool) {
        let mut given = false;
        let mut out = Vec::new();
        for tab in self.tabs.iter_mut().filter(|t| t.pinned && !skip(t.id)) {
            let id = tab.pin_id.get_or_insert_with(|| {
                given = true;
                new_id()
            });
            out.push(PinDef {
                id: id.clone(),
                letter: tab.pin_letter.clone().unwrap_or_default(),
                home: tab.home.clone().or_else(|| tab.url.clone()).unwrap_or_default(),
                title: tab.title.clone(),
                name: tab.name.clone(),
            });
        }
        (out, given)
    }

    /// Makes the row's pins `defs`, in that order (Pins.adopt): one known by
    /// its id takes the letter, page and name; a new one is added asleep at
    /// its page; one not in `defs` is unpinned and stays as a tab, so nothing
    /// open is closed under you. `skip` tabs and pins without an id are left
    /// as they are, after the others. Returns whether anything changed.
    pub fn adopt_pins(&mut self, defs: &[PinDef], skip: &dyn Fn(u64) -> bool) -> bool {
        let before: Vec<Tab> = self.tabs.iter().filter(|t| t.pinned).cloned().collect();
        let gone: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.pinned && !skip(t.id) && t.pin_id.as_deref().is_some_and(|p| !defs.iter().any(|d| d.id == p)))
            .map(|t| t.id)
            .collect();
        for id in gone {
            self.unpin(id);
        }
        let mut pins = Vec::new();
        for def in defs {
            let at = self.tabs.iter().position(|t| t.pinned && !skip(t.id) && t.pin_id.as_deref() == Some(def.id.as_str()));
            let mut tab = match at {
                Some(at) => self.tabs.remove(at),
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    let mut tab = Tab::blank(id, self.now);
                    tab.url = Some(def.home.clone());
                    tab.title = def.title.clone();
                    tab.asleep = true;
                    tab.pinned = true;
                    tab.pin_id = Some(def.id.clone());
                    tab
                }
            };
            tab.pin_letter = Some(def.letter.clone());
            tab.home = Some(def.home.clone());
            tab.name = def.name.clone();
            pins.push(tab);
        }
        // What wasn't the merge's to say (private, no id yet) stays, after.
        let at = self.pinned_count();
        let rest: Vec<Tab> = self.tabs.drain(..at).collect();
        pins.extend(rest);
        self.tabs.splice(0..0, pins);
        let after: Vec<&Tab> = self.tabs.iter().filter(|t| t.pinned).collect();
        after.len() != before.len() || after.iter().zip(&before).any(|(a, b)| *a != b)
    }

    /// Dragged from one place in the row to another. Pins stay first: a pin
    /// moves only among pins, any other tab only among the others. Returns
    /// whether anything moved.
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() {
            return false;
        }
        let pins = self.pinned_count();
        let to = if self.tabs[from].pinned {
            to.min(pins - 1)
        } else {
            to.max(pins).min(self.tabs.len() - 1)
        };
        if to == from {
            return false;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.run = None;
        true
    }
}

/// The letter a pin shows: the first letter or digit of the site's name,
/// upper-cased, or a dot when there is no site.
fn monogram(url: Option<&str>) -> String {
    let url = url.unwrap_or("");
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    host.chars().find(|c| c.is_alphanumeric()).map_or_else(|| "\u{2022}".into(), |c| c.to_uppercase().collect())
}
