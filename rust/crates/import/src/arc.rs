//! Arc's sidebar: its spaces, what is pinned in each, and the favourites
//! above them, from StorableSidebar.json beside its profiles — one plain JSON
//! file, nothing signed (ImportArc.swift). This reads it and says what is
//! there; the app makes the spaces and pins.
//!
//! The file keeps everything in flat lists of an id followed by its object:
//! spaces, and items — a tab, a folder ("list"), or a container — joined by
//! parentID and childrenIds. A space names its containers the same way, a
//! label then an id: "pinned" is taken; "unpinned" is Arc's Today, which
//! lives a day and is left behind. Read as damaged files are: each item once,
//! no deeper than 64, at most 5000 things, only http and https, no password
//! in an address, and a file larger than any real sidebar isn't read.

use crate::browsers::{Browser, read_capped};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pin {
    Item(Item),
    Folder { title: String, items: Vec<Pin> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Space {
    pub name: String,
    /// Arc's own, kept for the day it is wanted as it was.
    pub emoji: Option<String>,
    /// One of searchwin-spaces' ICONS, nearest to Arc's emoji or the name.
    pub symbol: &'static str,
    /// The Chromium profile folder it signs in with: "Default", or another's
    /// ("Profile 1"), which is a space with sign-ins of its own.
    pub profile: String,
    /// In Arc's order, folders kept as folders.
    pub pinned: Vec<Pin>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sidebar {
    /// The icons at the top, shared by every space of their profile.
    pub favorites: Vec<Item>,
    /// The same by profile, for giving each space only its own profile's.
    pub favorites_by_profile: HashMap<String, Vec<Item>>,
    pub spaces: Vec<Space>,
}

impl Sidebar {
    /// Pinned things in every space, folders' contents counted.
    pub fn pinned_count(&self) -> usize {
        self.spaces.iter().map(|s| count(&s.pinned)).sum()
    }
}

pub fn count(pins: &[Pin]) -> usize {
    pins.iter().map(|p| if let Pin::Folder { items, .. } = p { count(items) } else { 1 }).sum()
}

/// Arc's sidebar for one profile, or every one for None. None for any other
/// browser, or when the file is missing or doesn't read.
pub fn sidebar(browser: &Browser, profile: Option<&str>) -> Option<Sidebar> {
    let data = read_capped(browser.arc_sidebar.as_deref()?, 32 << 20)?;
    parse(&data, profile)
}

pub fn parse(data: &[u8], wanted: Option<&str>) -> Option<Sidebar> {
    let top: Value = serde_json::from_slice(data).ok()?;
    let containers = top.get("sidebar")?.get("containers")?.as_array()?;
    let main = containers.iter().filter_map(Value::as_object).find(|c| c.contains_key("spaces"))?;
    let mut items: HashMap<&str, &Map<String, Value>> = HashMap::new();
    for item in main.get("items").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(item), Some(id)) = (item.as_object(), item.get("id").and_then(Value::as_str)) {
            items.insert(id, item);
        }
    }
    let in_profile = |name: &str| wanted.is_none_or(|w| w == name);
    let mut walk = Walk { items: &items, taken: HashSet::new(), total: 0 };

    let mut spaces = Vec::new();
    for space in main.get("spaces").and_then(Value::as_array).into_iter().flatten() {
        let Some(space) = space.as_object() else { continue };
        let profile = profile_name(space.get("profile"));
        if !in_profile(&profile) {
            continue;
        }
        let name = named(space.get("title")).unwrap_or_else(|| "Space".into());
        let icon = space.get("customInfo").and_then(|c| c.get("iconType"));
        let emoji = icon.and_then(|i| i.get("emoji_v2")).and_then(Value::as_str).map(String::from);
        let arc_icon = icon.and_then(|i| i.get("icon")).and_then(Value::as_str);
        let pinned = container("pinned", space.get("newContainerIDs")).or_else(|| container("pinned", space.get("containerIDs")));
        spaces.push(Space {
            symbol: symbol(emoji.as_deref(), arc_icon, &name),
            emoji,
            name,
            profile,
            pinned: pinned.map(|id| walk.nodes(id, 0)).unwrap_or_default(),
        });
    }

    // The favourites: a profile followed by its container's id.
    let mut out = Sidebar { spaces, ..Sidebar::default() };
    let tops = main.get("topAppsContainerIDs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for (i, entry) in tops.iter().enumerate() {
        if entry.is_string() {
            continue;
        }
        let profile = profile_name(Some(entry));
        let Some(id) = tops.get(i + 1).and_then(Value::as_str) else { continue };
        if !in_profile(&profile) {
            continue;
        }
        for node in walk.nodes(id, 0) {
            if let Pin::Item(item) = node {
                out.favorites.push(item.clone());
                out.favorites_by_profile.entry(profile.clone()).or_default().push(item);
            }
        }
    }
    Some(out)
}

struct Walk<'a> {
    items: &'a HashMap<&'a str, &'a Map<String, Value>>,
    taken: HashSet<&'a str>,
    total: usize,
}

impl<'a> Walk<'a> {
    /// What a container or folder holds, in order.
    fn nodes(&mut self, id: &str, depth: usize) -> Vec<Pin> {
        let Some(children) = self.items.get(id).and_then(|i| i.get("childrenIds")).and_then(Value::as_array) else {
            return Vec::new();
        };
        if depth >= 64 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for child in children {
            let Some(child) = child.as_str() else { continue };
            if self.total >= 5000 {
                break;
            }
            let Some((&key, item)) = self.items.get_key_value(child) else { continue };
            if !self.taken.insert(key) {
                continue;
            }
            let Some(data) = item.get("data") else { continue };
            if let Some(tab) = data.get("tab") {
                let Some(url) = tab.get("savedURL").and_then(Value::as_str).and_then(crate::read::web) else { continue };
                self.total += 1;
                let title = named(item.get("title"))
                    .or_else(|| named(tab.get("savedTitle")))
                    .unwrap_or_else(|| crate::session::titled("", Some(&url)));
                out.push(Pin::Item(Item { title, url }));
            } else if data.get("list").is_some() {
                let inside = self.nodes(key, depth + 1);
                if !inside.is_empty() {
                    out.push(Pin::Folder { title: named(item.get("title")).unwrap_or_else(|| "Folder".into()), items: inside });
                }
            }
        }
        out
    }
}

/// The id after `label` in a space's list of containers. Arc has written
/// the label as a string and, lately, as an object keyed by it.
fn container<'a>(label: &str, list: Option<&'a Value>) -> Option<&'a str> {
    let list = list?.as_array()?;
    list.windows(2).find_map(|pair| {
        let matches = pair[0].as_str() == Some(label) || pair[0].get(label).is_some();
        if matches { pair[1].as_str() } else { None }
    })
}

/// "Default", or the folder of the profile a space or favourites belong to:
/// {"default": {}} or {"custom": {"_0": {"directoryBasename": …}}}.
fn profile_name(value: Option<&Value>) -> String {
    let Some(custom) = value.and_then(|v| v.get("custom")) else { return "Default".into() };
    let inner = custom.get("_0").unwrap_or(custom);
    inner
        .get("directoryBasename")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Default")
        .to_string()
}

fn named(text: Option<&Value>) -> Option<String> {
    let trimmed = text?.as_str()?.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(300).collect())
}

/// Arc's icon, else its emoji, else a word of the space's name, as the
/// nearest of the space pictures; a briefcase failing all three.
pub fn symbol(emoji: Option<&str>, arc_icon: Option<&str>, name: &str) -> &'static str {
    if let Some(icon) = arc_icon {
        let icon = icon.to_lowercase();
        if let Some((_, s)) = BY_ARC_ICON.iter().find(|(k, _)| *k == icon) {
            return s;
        }
    }
    if let Some(emoji) = emoji {
        // The same emoji with or without the mark asking for its coloured
        // form (U+FE0F), which Arc writes on some.
        let bare: String = emoji.chars().filter(|&c| c != '\u{FE0F}').collect();
        if let Some((s, _)) = BY_EMOJI.iter().find(|(_, list)| list.contains(&bare.as_str())) {
            return s;
        }
    }
    // Whole words, or the start of one for the longer keys: "ai" is a word
    // here, not the middle of "maison".
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for (s, keys) in BY_WORD {
        if keys.iter().any(|k| words.iter().any(|w| w == k || (k.chars().count() >= 4 && w.starts_with(k)))) {
            return s;
        }
    }
    "briefcase"
}

const BY_ARC_ICON: &[(&str, &str)] = &[
    ("construction", "chevron.left.forwardslash.chevron.right"),
    ("people", "heart"),
    ("star", "sparkles"),
    ("bulb", "lightbulb"),
    ("leaf", "leaf"),
    ("pizza", "cup.and.saucer"),
];

const BY_EMOJI: &[(&str, &[&str])] = &[
    ("briefcase", &["💼", "👔", "📊", "📈", "🗂"]),
    ("building.2", &["🏢", "🏛", "🏙"]),
    ("house", &["🏠", "🏡", "🛋"]),
    ("desktopcomputer", &["🖥"]),
    ("laptopcomputer", &["💻"]),
    ("terminal", &["⌨", "🧑‍💻", "👨‍💻", "👩‍💻"]),
    ("chevron.left.forwardslash.chevron.right", &["🧑‍🔧", "🛠", "🔧", "⚙"]),
    ("sparkles", &["✨", "🤖", "🪄", "⭐", "🌟"]),
    ("brain.head.profile", &["🧠"]),
    ("lightbulb", &["💡"]),
    ("gamecontroller", &["🎮", "🕹", "👾"]),
    ("beach.umbrella", &["🏖", "🌴", "⛱", "🏝"]),
    ("cup.and.saucer", &["☕", "🍵", "🫖"]),
    ("music.note", &["🎵", "🎶", "🎧", "🎸", "🎹"]),
    ("film", &["🎬", "🍿", "📺", "🎥"]),
    ("paintpalette", &["🎨", "🖌", "🖍", "✏"]),
    ("camera", &["📷", "📸"]),
    ("book", &["📚", "📖", "📕", "📘", "📰"]),
    ("graduationcap", &["🎓", "🏫", "✍"]),
    ("cart", &["🛒", "🛍", "💸", "💰", "💳"]),
    ("airplane", &["✈", "🧳", "🌍", "🌎", "🌏", "🗺"]),
    ("dumbbell", &["🏋", "💪", "🏃", "⚽", "🏀"]),
    ("leaf", &["🌿", "🌱", "🍃", "🦋", "🌸", "🌳"]),
    ("heart", &["❤", "💖", "💕", "🥰", "😍"]),
];

const BY_WORD: &[(&str, &[&str])] = &[
    ("briefcase", &["work", "job", "office", "boulot", "travail", "client"]),
    ("house", &["home", "perso", "personal", "maison", "family", "famille"]),
    ("chevron.left.forwardslash.chevron.right", &["code", "dev", "github", "eng"]),
    ("sparkles", &["ai", "gpt", "claude", "llm"]),
    ("graduationcap", &["school", "study", "studies", "course", "cours", "uni", "école"]),
    ("book", &["read", "lecture", "news", "research"]),
    ("paintpalette", &["design", "art", "creative", "figma"]),
    ("music.note", &["music", "musique"]),
    ("gamecontroller", &["game", "gaming", "jeu"]),
    ("cart", &["shop", "achat", "money", "finance"]),
    ("airplane", &["travel", "trip", "voyage"]),
    ("dumbbell", &["sport", "gym", "fitness"]),
    ("film", &["film", "movie", "video", "watch"]),
];
