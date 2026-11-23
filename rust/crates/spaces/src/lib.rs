//! Spaces and profiles (upstream Spaces.swift, Profiles.swift, SpaceRoutes.swift,
//! OneWindow.swift and the order SpaceSwipe.swift follows), the model only:
//! no window, no page engine.
//!
//! Spaces are separate rows of tabs, each with its own session file and, if
//! it was made signed out, cookies of its own. Off until turned on
//! (Spaces.swift:7-10): until then there is one space, the first, and its
//! tabs are the session there has always been (`session.json`), its sites
//! the store there has always been, so turning spaces on signs nobody out.
//!
//! Profiles are whole separate browsers (Profiles.swift:5-25): their own
//! spaces, sign-ins, history and bookmarks. There is always one, Personal,
//! which is everything there was before profiles, left where it was. Shared
//! by every profile, on purpose: settings, extensions, passwords, downloads.
//!
//! Everything is kept in `spaces.json`, written atomically. No file, or one
//! that won't decode, is one profile with one space, switched off: exactly
//! today's app.

mod routes;
mod rows;

pub use routes::{Route, find as find_route, site, site_of};
pub use rows::Rows;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The first space: the session and the store there were before spaces
/// (Spaces.swift:43). Upstream's own id, so the files read alike.
pub const FIRST_SPACE_ID: &str = "00000000-0000-0000-0000-000000000001";
/// Personal, the profile everything from before profiles is in (Profiles.swift:34).
pub const PERSONAL_ID: &str = "00000000-0000-0000-0000-0000000000A1";

/// A space's colours (Spaces.swift:59-67), red/green/blue in 0..=1. The
/// first stands for none and is never drawn.
pub const COLOURS: [[f32; 3]; 6] = [
    [0.45, 0.47, 0.52],
    [0.26, 0.52, 0.96],
    [0.20, 0.66, 0.45],
    [0.96, 0.62, 0.20],
    [0.90, 0.33, 0.40],
    [0.62, 0.40, 0.90],
];
pub const COLOUR_NAMES: [&str; 6] = ["None", "Blue", "Green", "Orange", "Red", "Violet"];

/// The icons a space can wear (Spaces.swift:88-99), by upstream's symbol
/// name so a file from the Mac reads the same; the app draws each its own way.
pub const ICONS: [&str; 24] = [
    "briefcase", "building.2", "desktopcomputer", "laptopcomputer", "chevron.left.forwardslash.chevron.right", "terminal",
    "sparkles", "brain.head.profile", "lightbulb", "gamecontroller", "beach.umbrella", "cup.and.saucer",
    "music.note", "film", "paintpalette", "camera", "house", "book",
    "graduationcap", "cart", "airplane", "dumbbell", "leaf", "heart",
];
pub const ICON_NAMES: [&str; 24] = [
    "Work", "Office", "Desktop", "Laptop", "Code", "Terminal",
    "AI", "Thinking", "Ideas", "Games", "Leisure", "Café",
    "Music", "Film", "Art", "Photos", "Home", "Reading",
    "Studies", "Shopping", "Travel", "Sport", "Nature", "Personal",
];

/// How much of a space's colour the chrome wears, never the page
/// (Spaces.swift:69-84): faint, a little more on the dark ground. None for
/// no colour or an index out of range (from a newer version): draw nothing.
pub fn wash(colour: usize, dark: bool) -> Option<f64> {
    (colour > 0 && colour < COLOURS.len()).then_some(if dark { 0.11 } else { 0.07 })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    /// Which of `COLOURS`; 0 is none (Spaces.swift:23-26).
    #[serde(default)]
    pub colour: usize,
    /// One of `ICONS`; None for the default (see `symbol`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Signed in wherever its profile is, only the tabs its own, rather than
    /// a store of its own. Missing (a space from before the choice) is a
    /// store of its own (Spaces.swift:29-33).
    #[serde(default, skip_serializing_if = "is_false")]
    pub shares_sign_ins: bool,
    /// Where its downloads go; None for the folder in Settings (Spaces.swift:34).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<String>,
    /// Its profile; None for Personal (Spaces.swift:36-40).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Space {
    pub fn first() -> Space {
        Space {
            id: FIRST_SPACE_ID.into(),
            name: "Personal".into(),
            colour: 0,
            icon: None,
            shares_sign_ins: false,
            downloads: None,
            profile: None,
        }
    }

    pub fn is_first(&self) -> bool {
        self.id == FIRST_SPACE_ID
    }

    pub fn profile_id(&self) -> &str {
        self.profile.as_deref().unwrap_or(PERSONAL_ID)
    }

    /// The icon it shows: its own if known, else a house for the first and
    /// a briefcase for any other (Spaces.swift:49).
    pub fn symbol(&self) -> &str {
        match self.icon.as_deref() {
            Some(icon) if ICONS.contains(&icon) => icon,
            _ if self.is_first() => "house",
            _ => "briefcase",
        }
    }

    /// Its colour's index, None for none or one out of range (Spaces.swift:53).
    pub fn tint(&self) -> Option<usize> {
        (self.colour > 0 && self.colour < COLOURS.len()).then_some(self.colour)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Its first space, which can't be deleted while the profile lives
    /// (Profiles.swift:30-32).
    pub home: String,
}

impl Profile {
    pub fn personal() -> Profile {
        Profile { id: PERSONAL_ID.into(), name: "Personal".into(), home: FIRST_SPACE_ID.into() }
    }

    pub fn is_personal(&self) -> bool {
        self.id == PERSONAL_ID
    }
}

/// What a deletion leaves for the app to clear away: the session files of
/// the spaces gone, their WebView2 profiles (also kept in `erasing` until
/// the app says they are gone), and a profile's data folder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Removal {
    pub spaces: Vec<String>,
    pub webview_profiles: Vec<String>,
    pub profile: Option<String>,
}

impl Removal {
    /// Deletes the session files and the profile's folder under `base`.
    /// Never `session.json` nor `base` itself (Session.swift:107-110):
    /// those are the first space's and Personal's, which never go.
    pub fn erase_files(&self, base: &Path) {
        for space in self.spaces.iter().filter(|s| s.as_str() != FIRST_SPACE_ID) {
            let _ = std::fs::remove_file(session_path(base, space));
        }
        if let Some(profile) = self.profile.as_deref().filter(|p| *p != PERSONAL_ID) {
            let _ = std::fs::remove_dir_all(profile_folder(base, profile));
        }
    }
}

/// The result of a switch, for the app to swap rows (and, when
/// `profile_changed`, its page engine profile and history/bookmarks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switched {
    pub from: String,
    pub to: String,
    pub profile_changed: bool,
}

/// Every profile and space, in order, the one on screen, and the switches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spaces {
    #[serde(default)]
    profiles: Vec<Profile>,
    /// Every profile's spaces in the order arranged; a profile's own order
    /// is the order of its spaces here (Spaces.swift:103-105).
    #[serde(default)]
    spaces: Vec<Space>,
    #[serde(default)]
    current: String,
    /// Each profile's last space, the one it opens at (Spaces.swift:402,
    /// Profiles.swift:206-208).
    #[serde(default)]
    last: BTreeMap<String, String>,
    /// Settings › Spaces (Prefs.swift:771): off until turned on.
    #[serde(default)]
    on: bool,
    /// Send links to a space (Prefs.swift:642): off until turned on.
    #[serde(default)]
    sends_links: bool,
    /// Profiles in one window (Prefs.swift:777, OneWindow.swift:4): off until
    /// turned on.
    #[serde(default)]
    one_window: bool,
    #[serde(default)]
    routes: Vec<Route>,
    /// WebView2 profiles of deleted spaces and profiles still to be removed
    /// ("spaces.erasing", Spaces.swift:259-282).
    #[serde(default)]
    erasing: Vec<String>,
}

impl Default for Spaces {
    fn default() -> Spaces {
        Spaces {
            profiles: vec![Profile::personal()],
            spaces: vec![Space::first()],
            current: FIRST_SPACE_ID.into(),
            last: BTreeMap::new(),
            on: false,
            sends_links: false,
            one_window: false,
            routes: Vec::new(),
            erasing: Vec::new(),
        }
    }
}

// ---- files ----

/// `spaces.json` in the data folder.
pub fn spaces_path(base: &Path) -> PathBuf {
    base.join("spaces.json")
}

/// The first space's session is the one there always was; each other space
/// keeps its own beside it (Session.swift:59-63). Upstream writes the id as
/// Swift's uppercase uuidString.
pub fn session_file_name(space: &str) -> String {
    if space == FIRST_SPACE_ID { "session.json".into() } else { format!("session-{space}.json") }
}

pub fn session_path(base: &Path, space: &str) -> PathBuf {
    base.join(session_file_name(space))
}

/// Where a profile's own files (history.json, bookmarks.json …) live:
/// Personal's are the data folder itself, so nothing moves for anyone
/// (Profiles.swift:16-19); any other's are in `Profiles\<id>`. (Upstream
/// puts "history-<id>.json" beside Personal's, Profiles.swift:108; a folder
/// is one thing to delete with the profile.)
pub fn profile_folder(base: &Path, profile: &str) -> PathBuf {
    if profile == PERSONAL_ID { base.to_path_buf() } else { base.join("Profiles").join(profile) }
}

/// The WebView2 profile name (ICoreWebView2ControllerOptions.ProfileName) a
/// profile's sites use. None is the default profile the app has always
/// used: Personal's store is the one there always was (Profiles.swift:85-89).
pub fn webview_profile(profile: &str) -> Option<String> {
    (profile != PERSONAL_ID).then(|| format!("profile-{profile}"))
}

/// A space's own store, for one made signed out (Spaces.swift:206).
fn space_webview_profile(space: &str) -> String {
    format!("space-{space}")
}

impl Spaces {
    /// Reads spaces.json. No file is the default; one that won't decode is
    /// moved aside to `.bad` (as searchwin_tabs::load does) and is the
    /// default too. Either way never a failure.
    pub fn load(path: &Path) -> Spaces {
        let Ok(data) = std::fs::read(path) else { return Spaces::default() };
        match serde_json::from_slice::<Spaces>(&data) {
            Ok(mut spaces) => {
                spaces.repair();
                spaces
            }
            Err(_) => {
                let _ = std::fs::rename(path, beside(path, ".bad"));
                Spaces::default()
            }
        }
    }

    /// A temp file beside it, then renamed over: never a half-written file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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

    /// Whatever the file says, a list the rest can trust.
    fn repair(&mut self) {
        let upper = |s: &mut String| *s = s.trim().to_uppercase();
        // Profiles: Personal first with its fixed home, keeping the name it
        // was given (Profiles.swift:57-61); no duplicates, no blanks.
        let mut profiles = vec![Profile::personal()];
        for mut p in std::mem::take(&mut self.profiles) {
            upper(&mut p.id);
            upper(&mut p.home);
            if p.id == PERSONAL_ID {
                if !p.name.trim().is_empty() {
                    profiles[0].name = p.name;
                }
            } else if !p.id.is_empty() && !profiles.iter().any(|q| q.id == p.id) {
                profiles.push(p);
            }
        }
        self.profiles = profiles;
        // Spaces: one first space, in Personal, wherever it was placed; one
        // inserted at the front if the file lost it (Spaces.swift:106-127).
        // A space whose profile is gone goes to Personal rather than out of
        // reach with its tabs.
        let mut spaces: Vec<Space> = Vec::new();
        for mut s in std::mem::take(&mut self.spaces) {
            upper(&mut s.id);
            if let Some(p) = s.profile.as_mut() {
                upper(p);
            }
            if s.id.is_empty() || spaces.iter().any(|t| t.id == s.id) {
                continue;
            }
            if s.is_first() || s.profile.as_deref().is_some_and(|p| p == PERSONAL_ID || !self.profiles.iter().any(|q| q.id == p)) {
                s.profile = None;
            }
            spaces.push(s);
        }
        if !spaces.iter().any(Space::is_first) {
            spaces.insert(0, Space::first());
        }
        self.spaces = spaces;
        // Every other profile's home is one of its spaces: the one named,
        // else its first, else a new one.
        for at in 1..self.profiles.len() {
            let id = self.profiles[at].id.clone();
            let home = self.profiles[at].home.clone();
            let mine: Vec<&Space> = self.spaces.iter().filter(|s| s.profile_id() == id).collect();
            if mine.iter().any(|s| s.id == home) {
                continue;
            }
            if let Some(first) = mine.first() {
                self.profiles[at].home = first.id.clone();
            } else {
                let space = self.home_space_for(&id, &self.profiles[at].name.clone());
                self.profiles[at].home = space.id.clone();
                self.spaces.push(space);
            }
        }
        for r in &mut self.routes {
            upper(&mut r.space);
        }
        upper(&mut self.current);
        self.last = std::mem::take(&mut self.last)
            .into_iter()
            .map(|(p, s)| (p.trim().to_uppercase(), s.trim().to_uppercase()))
            .filter(|(p, s)| self.space(s).is_some_and(|s| s.profile_id() == p))
            .collect();
        if self.space(&self.current.clone()).is_none() {
            self.current = FIRST_SPACE_ID.into();
        }
        // Spaces off: the window shows its profile's home (Spaces.swift:541-545).
        if !self.on {
            self.current = self.home_of(self.current_profile()).to_string();
        }
    }

    fn home_space_for(&self, profile: &str, name: &str) -> Space {
        Space {
            id: self.new_id(),
            name: name.to_string(),
            colour: 0,
            icon: Some("house".into()),
            shares_sign_ins: true,
            downloads: None,
            profile: Some(profile.to_string()),
        }
    }

    // ---- reading ----

    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// More than one profile: only then does anything about profiles show
    /// (Profiles.swift:48-49).
    pub fn several_profiles(&self) -> bool {
        self.profiles.len() > 1
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// Every space of every profile, in order.
    pub fn all(&self) -> &[Space] {
        &self.spaces
    }

    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    /// One profile's spaces in their order (Spaces.swift:131).
    pub fn of_profile<'a>(&'a self, profile: &'a str) -> impl Iterator<Item = &'a Space> + 'a {
        self.spaces.iter().filter(move |s| s.profile_id() == profile)
    }

    /// The space on screen.
    pub fn current(&self) -> &Space {
        self.space(&self.current).unwrap_or(&self.spaces[0])
    }

    pub fn current_id(&self) -> &str {
        &self.current().id
    }

    pub fn current_profile(&self) -> &str {
        self.current().profile_id()
    }

    /// A profile's home: the space it shows with spaces off, and the one that
    /// can't be deleted (Spaces.swift:544-546). Personal's for one not known.
    pub fn home_of(&self, profile: &str) -> &str {
        self.profile(profile).map_or(FIRST_SPACE_ID, |p| p.home.as_str())
    }

    /// The profile a space is in: Personal for one that isn't known (Spaces.swift:193).
    pub fn profile_of(&self, space: &str) -> &str {
        self.space(space).map_or(PERSONAL_ID, Space::profile_id)
    }

    pub fn uses_spaces(&self) -> bool {
        self.on
    }

    pub fn sends_links(&self) -> bool {
        self.sends_links
    }

    pub fn profiles_in_one_window(&self) -> bool {
        self.one_window
    }

    /// Profiles in one window applies: turned on, with more than one profile
    /// to put in it (OneWindow.swift:31).
    pub fn merges(&self) -> bool {
        self.one_window && self.several_profiles()
    }

    /// Pins every space shows above its own (Prefs.swift:825-833): always
    /// on upstream since the Settings rework, shown only with spaces on.
    pub fn shows_shared_pins(&self) -> bool {
        self.on
    }

    /// Every space the window can go to, in order: its profile's, and with
    /// profiles in one window every profile's, profile by profile
    /// (OneWindow.swift:256-261). The space menu, ⌃1–⌃9 count through this.
    pub fn reachable(&self) -> Vec<&Space> {
        if self.merges() {
            self.profiles.iter().flat_map(|p| self.of_profile(&p.id)).collect()
        } else {
            self.of_profile(self.current_profile()).collect()
        }
    }

    /// Where the current space's downloads go: its folder with spaces on,
    /// else None for the one in Settings (Spaces.swift:344-347).
    pub fn downloads_folder(&self) -> Option<&str> {
        self.current().downloads.as_deref().filter(|_| self.on)
    }

    /// The WebView2 profile a space's tabs use (Spaces.swift:198-211): its
    /// profile's for the first space, a profile's home, and any space made
    /// signed in with the others; a profile of its own otherwise. None is
    /// the default profile, the one the app always used.
    pub fn webview_profile_for(&self, space: &str) -> Option<String> {
        let s = self.space(space)?;
        let profile = s.profile_id();
        if s.is_first() || s.shares_sign_ins || (profile != PERSONAL_ID && self.home_of(profile) == space) {
            webview_profile(profile)
        } else {
            Some(space_webview_profile(space))
        }
    }

    /// The label a rule's space shows: the profile's name before it once
    /// there is more than one (SpaceRoutes.swift:142-147).
    pub fn label(&self, space: &str) -> Option<String> {
        let s = self.space(space)?;
        if !self.several_profiles() {
            return Some(s.name.clone());
        }
        let profile = self.profile(s.profile_id()).map_or("Personal", |p| p.name.as_str());
        Some(format!("{profile} › {}", s.name))
    }

    /// The icon a new space gets unless told: the first no space of the
    /// current profile wears yet (Spaces.swift:464-467).
    pub fn free_icon(&self) -> &'static str {
        let used: Vec<&str> = self.of_profile(self.current_profile()).map(Space::symbol).collect();
        ICONS.iter().copied().find(|i| !used.contains(i)).unwrap_or("briefcase")
    }

    // ---- switches ----

    pub fn set_uses_spaces(&mut self, on: bool) {
        self.on = on;
        // Turned off: back to the home space. The others are kept, in case
        // they are turned on again (Spaces.swift:567-573).
        if !on {
            let home = self.home_of(self.current_profile()).to_string();
            self.current = home;
        }
    }

    pub fn set_sends_links(&mut self, on: bool) {
        self.sends_links = on;
    }

    pub fn set_profiles_in_one_window(&mut self, on: bool) {
        self.one_window = on;
    }

    // ---- going between spaces ----

    /// ⌃1–⌃9, the space menu: to a space. Only with spaces on
    /// (Spaces.swift:352-359); a space of another profile only with
    /// profiles in one window (Spaces.swift:354, OneWindow.swift:124-133).
    /// None when nothing changed.
    pub fn switch_to(&mut self, id: &str) -> Option<Switched> {
        if !self.on {
            return None;
        }
        let target = self.space(id)?;
        if target.profile_id() != self.current_profile() && !self.merges() {
            return None;
        }
        self.enter(id)
    }

    /// The move itself, remembering the profile's last space (Spaces.swift:398-402).
    fn enter(&mut self, id: &str) -> Option<Switched> {
        let to = self.space(id)?;
        if to.id == self.current {
            return None;
        }
        let (to, profile) = (to.id.clone(), to.profile_id().to_string());
        let from = std::mem::replace(&mut self.current, to);
        let profile_changed = self.profile_of(&from) != profile;
        self.last.insert(profile, self.current.clone());
        Some(Switched { from, to: self.current.clone(), profile_changed })
    }

    /// ⌃N: the Nth reachable space, 1..=9 (Shortcuts.swift:293-297, Spaces.swift:457-461).
    pub fn select_number(&mut self, n: usize) -> Option<Switched> {
        if !(1..=9).contains(&n) {
            return None;
        }
        let id = self.reachable().get(n - 1)?.id.clone();
        self.switch_to(&id)
    }

    /// The space a swipe brings, `step` = 1 for next, -1 for previous
    /// (SpaceSwipe.swift:75-96): the neighbour in the profile's order, no
    /// wrapping. Past either end, with profiles in one window, the profile
    /// before's last space or the next profile's first (OneWindow.swift:263-279).
    pub fn neighbour(&self, step: isize) -> Option<&str> {
        if !self.on || step == 0 {
            return None;
        }
        let mine: Vec<&Space> = self.of_profile(self.current_profile()).collect();
        let here = mine.iter().position(|s| s.id == self.current)?;
        if let Some(s) = here.checked_add_signed(step).and_then(|at| mine.get(at)) {
            return Some(&s.id);
        }
        if !self.merges() {
            return None;
        }
        let at = self.profiles.iter().position(|p| p.id == self.current_profile())?;
        let other = &self.profiles.get(at.checked_add_signed(step.signum())?)?.id;
        let mut theirs = self.of_profile(other);
        let s = if step > 0 { theirs.next() } else { theirs.last() };
        s.map(|s| s.id.as_str())
    }

    pub fn next_space(&mut self) -> Option<Switched> {
        let id = self.neighbour(1)?.to_string();
        self.switch_to(&id)
    }

    pub fn previous_space(&mut self) -> Option<Switched> {
        let id = self.neighbour(-1)?.to_string();
        self.switch_to(&id)
    }

    /// Another profile chosen: at the space it was last in, or its home
    /// (Profiles.swift:198-212, OneWindow.swift:138-148). The window keeps
    /// one profile's pages at a time, never another's under its sign-ins.
    /// With spaces off, its home.
    pub fn open_profile(&mut self, profile: &str) -> Option<Switched> {
        self.profile(profile)?;
        let at = match self.last.get(profile) {
            Some(last) if self.on && self.space(last).is_some() => last.clone(),
            _ => self.home_of(profile).to_string(),
        };
        self.enter(&at)
    }

    // ---- making and changing spaces ----

    /// A new space in the current profile, at the right-hand end, signed in
    /// where the others are or with cookies of its own; on screen when
    /// spaces are on (Spaces.swift:469-486). Returns its id.
    pub fn add_space(&mut self, name: &str, icon: Option<&str>, shares_sign_ins: bool) -> String {
        let profile = self.current_profile().to_string();
        let space = Space {
            id: self.new_id(),
            name: name.trim().to_string(),
            colour: 0,
            icon: Some(icon.filter(|i| ICONS.contains(i)).unwrap_or(self.free_icon()).to_string()),
            shares_sign_ins,
            downloads: None,
            profile: (profile != PERSONAL_ID).then_some(profile),
        };
        let id = space.id.clone();
        self.spaces.push(space);
        self.switch_to(&id);
        id
    }

    /// Rename; an empty name is no answer (Spaces.swift:517-521).
    pub fn rename_space(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        match self.spaces.iter_mut().find(|s| s.id == id) {
            Some(s) if !name.is_empty() => {
                s.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Only one of `ICONS` (Spaces.swift:523-527, 783-787).
    pub fn set_icon(&mut self, id: &str, icon: &str) -> bool {
        match self.spaces.iter_mut().find(|s| s.id == id) {
            Some(s) if ICONS.contains(&icon) => {
                s.icon = Some(icon.to_string());
                true
            }
            _ => false,
        }
    }

    /// Only one of `COLOURS`, 0 for none (Spaces.swift:529-533).
    pub fn set_colour(&mut self, id: &str, colour: usize) -> bool {
        match self.spaces.iter_mut().find(|s| s.id == id) {
            Some(s) if colour < COLOURS.len() => {
                s.colour = colour;
                true
            }
            _ => false,
        }
    }

    /// Its own downloads folder, or None for Settings' (Spaces.swift:535-539).
    pub fn set_downloads(&mut self, id: &str, folder: Option<&str>) -> bool {
        match self.spaces.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.downloads = folder.map(str::to_string);
                true
            }
            None => false,
        }
    }

    /// Dragged to another place among its profile's spaces; ⌃1–⌃9 and the
    /// swipe follow the order. The home can be moved too (Spaces.swift:488-494).
    pub fn move_space(&mut self, id: &str, to: usize) -> bool {
        let profile = self.profile_of(id).to_string();
        let places: Vec<usize> = (0..self.spaces.len()).filter(|&i| self.spaces[i].profile_id() == profile).collect();
        let Some(from) = places.iter().position(|&i| self.spaces[i].id == id) else { return false };
        if to >= places.len() || to == from {
            return false;
        }
        let mut mine: Vec<Space> = places.iter().map(|&i| self.spaces[i].clone()).collect();
        let moved = mine.remove(from);
        mine.insert(to, moved);
        for (&i, s) in places.iter().zip(mine) {
            self.spaces[i] = s;
        }
        true
    }

    /// A space, its tabs, and its cookies and sign-ins, gone. The first and
    /// a profile's home stay (Spaces.swift:548-565). On screen, the profile's
    /// home comes first. A space signed in with the others has no store of
    /// its own to erase. Rules naming it stay and are passed over
    /// (SpaceRoutes.swift:19-20). History and bookmarks stay.
    pub fn delete_space(&mut self, id: &str) -> Option<Removal> {
        let s = self.space(id)?;
        let profile = s.profile_id().to_string();
        if s.is_first() || self.home_of(&profile) == id {
            return None;
        }
        let own_store = !s.shares_sign_ins;
        if self.current == id {
            let home = self.home_of(&profile).to_string();
            self.enter(&home);
        }
        self.spaces.retain(|s| s.id != id);
        self.last.retain(|_, s| s != id);
        let mut removal = Removal { spaces: vec![id.to_string()], ..Removal::default() };
        if own_store {
            removal.webview_profiles.push(space_webview_profile(id));
        }
        self.erasing.extend(removal.webview_profiles.iter().cloned());
        Some(removal)
    }

    /// A profile's spaces as sync merged them, in that order: the ones not
    /// in `list` deleted as Delete Space does (never the home), the others
    /// taken as they are. An id another profile has is left out.
    pub fn adopt(&mut self, profile: &str, list: Vec<Space>) -> Vec<Removal> {
        let wanted: Vec<String> = list.iter().map(|s| s.id.trim().to_uppercase()).collect();
        let gone: Vec<String> = self.of_profile(profile).filter(|s| !wanted.contains(&s.id)).map(|s| s.id.clone()).collect();
        let removals = gone.iter().filter_map(|id| self.delete_space(id)).collect();
        let kept: Vec<Space> = self.of_profile(profile).filter(|s| !wanted.contains(&s.id)).cloned().collect();
        let at = self.spaces.iter().position(|s| s.profile_id() == profile).unwrap_or(self.spaces.len());
        self.spaces.retain(|s| s.profile_id() != profile);
        let owner = (profile != PERSONAL_ID).then(|| profile.to_string());
        let mut mine: Vec<Space> = Vec::new();
        for mut s in list {
            s.id = s.id.trim().to_uppercase();
            s.profile = owner.clone();
            // The first space is Personal's alone; an id in use elsewhere is someone else's.
            let free = !s.id.is_empty() && !self.spaces.iter().chain(&mine).any(|t| t.id == s.id);
            if free && (!s.is_first() || owner.is_none()) {
                mine.push(s);
            }
        }
        // What couldn't be deleted (the home) stays, at the end.
        mine.extend(kept);
        self.spaces.splice(at..at, mine);
        removals
    }

    // ---- profiles ----

    /// A new profile and its home space, which shares the profile's own
    /// store: signed out of everything, nothing in its history
    /// (Profiles.swift:127-141). Not switched to: the app opens it.
    pub fn add_profile(&mut self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let id = self.new_id();
        let mut space = self.home_space_for(&id, name);
        // Two fresh ids in one go: the space's must differ from the profile's.
        while space.id == id {
            space.id = self.new_id();
        }
        self.profiles.push(Profile { id: id.clone(), name: name.to_string(), home: space.id.clone() });
        self.spaces.push(space);
        Some(id)
    }

    pub fn rename_profile(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        match self.profiles.iter_mut().find(|p| p.id == id) {
            Some(p) if !name.is_empty() => {
                p.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Upstream has no way to reorder profiles; the order matters only for
    /// profiles in one window. Personal stays first (Profiles.swift:43, 60).
    pub fn move_profile(&mut self, id: &str, to: usize) -> bool {
        let Some(from) = self.profiles.iter().position(|p| p.id == id) else { return false };
        if from == 0 || to == 0 || to >= self.profiles.len() || to == from {
            return false;
        }
        let p = self.profiles.remove(from);
        self.profiles.insert(to, p);
        true
    }

    /// A profile and all it holds, gone: its spaces with their tabs, its
    /// cookies, sign-ins, site data, history and bookmarks. Personal stays
    /// (Profiles.swift:150-185). If it is on screen, Personal comes, at its
    /// last space: never left without a window (Profiles.swift:155-160).
    pub fn delete_profile(&mut self, id: &str) -> Option<Removal> {
        if id == PERSONAL_ID || self.profile(id).is_none() {
            return None;
        }
        if self.current_profile() == id {
            self.open_profile(PERSONAL_ID);
        }
        let mut removal = Removal { profile: Some(id.to_string()), ..Removal::default() };
        for s in self.spaces.iter().filter(|s| s.profile_id() == id) {
            removal.spaces.push(s.id.clone());
            // A space with sign-ins of its own has a store of its own; the
            // home shares the profile's.
            if !s.shares_sign_ins && self.home_of(id) != s.id {
                removal.webview_profiles.push(space_webview_profile(&s.id));
            }
        }
        removal.webview_profiles.extend(webview_profile(id));
        self.spaces.retain(|s| s.profile_id() != id);
        self.profiles.retain(|p| p.id != id);
        self.last.remove(id);
        self.erasing.extend(removal.webview_profiles.iter().cloned());
        Some(removal)
    }

    // ---- routes ----

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// One site to one space, in place of any rule the site had
    /// (SpaceRoutes.swift:110-118). The site is cleaned as typed; false for
    /// anything that isn't a host or a space that isn't there.
    pub fn set_route(&mut self, typed: &str, space: &str) -> bool {
        let (Some(site), true) = (site(typed), self.space(space).is_some()) else { return false };
        match self.routes.iter_mut().find(|r| r.site == site) {
            Some(r) => r.space = space.to_string(),
            None => self.routes.push(Route { site, space: space.to_string() }),
        }
        true
    }

    pub fn remove_route(&mut self, site: &str) -> bool {
        let before = self.routes.len();
        self.routes.retain(|r| r.site != site);
        self.routes.len() != before
    }

    /// The space a rule sends an address to: only while the setting and
    /// spaces are both on, and only a space still there (SpaceRoutes.swift:96-107).
    pub fn route(&self, address: &str) -> Option<&Space> {
        if !(self.on && self.sends_links) {
            return None;
        }
        self.space(&routes::find(address, &self.routes)?.space)
    }

    /// The space a link from a page in `from` (or from another app, with
    /// the current space) goes to, when it is another one
    /// (SpaceRoutes.swift:153-167). Private tabs are the app's to leave out.
    pub fn sends_to(&self, address: &str, from: &str) -> Option<&Space> {
        self.route(address).filter(|s| s.id != from)
    }

    // ---- page engine profiles to erase ----

    /// WebView2 profiles of deleted spaces and profiles not yet removed:
    /// the app tries again at launch (Spaces.swift:259-282).
    pub fn erasing(&self) -> &[String] {
        &self.erasing
    }

    /// The app removed it (or found it gone).
    pub fn erased(&mut self, name: &str) {
        self.erasing.retain(|n| n != name);
    }

    // ---- ids ----

    /// A fresh id in upstream's form (uppercase UUID v4), never one in use.
    /// Randomness from std's per-process random hash keys: no dependency.
    fn new_id(&self) -> String {
        loop {
            let id = random_uuid();
            if !self.spaces.iter().any(|s| s.id == id) && !self.profiles.iter().any(|p| p.id == id) {
                return id;
            }
        }
    }
}

pub fn random_uuid() -> String {
    use std::hash::BuildHasher;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let keys = std::collections::hash_map::RandomState::new();
    let hi = keys.hash_one((nanos, n, 1u8));
    let lo = keys.hash_one((nanos, n, 2u8));
    let hi = (hi & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
    let lo = (lo & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
    format!(
        "{:08X}-{:04X}-{:04X}-{:04X}-{:012X}",
        hi >> 32,
        (hi >> 16) & 0xFFFF,
        hi & 0xFFFF,
        lo >> 48,
        lo & 0xFFFF_FFFF_FFFF
    )
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests;
