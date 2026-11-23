//! What each kind of thing is, as items, and back. Everything read back from
//! the folder is checked here: the folder is anyone's who can write in it.

use std::collections::BTreeSet;

use searchwin_bookmarks::Node;
use searchwin_prefs::Prefs;
use searchwin_spaces::{COLOURS, ICONS, PERSONAL_ID, Space};
pub use searchwin_tabs::PinDef;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Items, is_id, ordered, text, value};

/// Longest name or title taken from the folder.
const TEXT_MAX: usize = 1000;

/// Control characters and direction overrides out, the length capped: a name
/// from the folder is shown, never run, and can't pretend to read otherwise.
pub fn clean(text: &str) -> String {
    let bidi = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}');
    text.chars().filter(|&c| !c.is_control() && !bidi(c)).take(TEXT_MAX).collect()
}

/// Past this depth a folder is no one's filing (bookmarks' own MAX_DEPTH):
/// deeper ones go to the top, and the stack stays shallow.
const DEPTH_MAX: usize = 64;

/// The places the app itself keeps as bookmarks (bookmarks::keepable):
/// never javascript:, data: and the like.
pub fn safe_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https" | "file"))
}

// ---- settings ----

/// The settings that follow you: how the app looks and behaves, nothing that
/// belongs to this PC. A new setting syncs only once it is added here.
/// Never `custom_engine`: whoever can write in the folder could send every
/// search to an address of their choosing.
pub const SETTINGS: [&str; 10] = [
    "engine",
    "shield",
    "sleep",
    "tabs",
    "close_goes_back",
    "reader_look",
    "split_view",
    "float_away",
    "peek",
    "peek_with",
];

pub fn settings(prefs: &Prefs) -> Items {
    let all = prefs.to_value();
    SETTINGS.iter().filter_map(|k| all.get(*k).map(|v| (format!("set:{k}"), text(v)))).collect()
}

/// A setting's value from the folder, if it is one this build takes: a known
/// key and a value Prefs accepts.
fn setting(key: &str, text: &str) -> Option<Value> {
    let v: Value = value(text)?;
    (SETTINGS.contains(&key) && Prefs::default().set(key, v.clone()).is_ok()).then_some(v)
}

/// The settings to change here: those the merged items have differently
/// from `here`, each already checked.
pub fn settings_to_apply(live: &Items, here: &Items) -> Vec<(String, Value)> {
    SETTINGS
        .iter()
        .filter_map(|k| {
            let key = format!("set:{k}");
            let text = live.get(&key).filter(|t| here.get(&key) != Some(*t))?;
            setting(k, text).map(|v| (k.to_string(), v))
        })
        .collect()
}

// ---- bookmarks ----

/// A bookmark or a folder (no url), without its place: the folder that holds
/// it lists it.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Bookmark {
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

const ROOT: &str = "root";

fn bookmark(text: &str) -> Option<Bookmark> {
    let b: Bookmark = value(text)?;
    b.url.as_deref().is_none_or(safe_url).then(|| Bookmark { title: clean(&b.title), url: b.url })
}

/// A bookmarks tree as items: each bookmark and folder, and each folder's
/// list of what it holds, the top level under "root".
pub fn bookmarks(roots: &[Node]) -> Items {
    let mut out = Items::new();
    fn walk(nodes: &[Node], folder: &str, out: &mut Items) {
        let nodes: Vec<&Node> = nodes.iter().filter(|n| is_id(&n.id)).collect();
        if !nodes.is_empty() {
            out.insert(format!("kids:{folder}"), text(&nodes.iter().map(|n| &n.id).collect::<Vec<_>>()));
        }
        for n in nodes {
            out.insert(format!("bm:{}", n.id), text(&Bookmark { title: n.title.clone(), url: n.url.clone() }));
            if n.is_folder() {
                walk(n.children.as_deref().unwrap_or_default(), &n.id, out);
            }
        }
    }
    walk(roots, ROOT, &mut out);
    out
}

/// The tree the merged items say there is. A bookmark no folder lists any
/// more (its folder deleted on one PC while it was added on another, or two
/// folders moved into each other) goes at the end of the top level rather
/// than being lost. One whose item this build can't read (a newer version's,
/// a damaged one) stays as this PC has it (`local`).
pub fn apply_bookmarks(live: &Items, local: &[Node]) -> Vec<Node> {
    type Nodes = std::collections::BTreeMap<String, Bookmark>;
    let mut mine = Nodes::new();
    fn walk(nodes: &[Node], into: &mut Nodes) {
        for n in nodes {
            into.insert(n.id.clone(), Bookmark { title: n.title.clone(), url: n.url.clone() });
            walk(n.children.as_deref().unwrap_or_default(), into);
        }
    }
    walk(local, &mut mine);
    let nodes: Nodes = live
        .iter()
        .filter_map(|(k, t)| {
            let id = k.strip_prefix("bm:").filter(|id| is_id(id))?;
            let b = bookmark(t).or_else(|| (!understood(k, t)).then(|| mine.remove(id)).flatten())?;
            Some((id.to_string(), b))
        })
        .collect();
    let kids = |folder: &str| live.get(&format!("kids:{folder}")).and_then(|t| value::<Vec<String>>(t)).unwrap_or_default();
    let mut placed = BTreeSet::new();
    fn build(folder: &str, depth: usize, nodes: &Nodes, kids: &dyn Fn(&str) -> Vec<String>, placed: &mut BTreeSet<String>) -> Vec<Node> {
        let mut out = Vec::new();
        if depth >= DEPTH_MAX {
            return out;
        }
        for id in kids(folder) {
            let Some(b) = nodes.get(&id) else { continue };
            if placed.insert(id.clone()) {
                out.push(node(&id, b, depth, nodes, kids, placed));
            }
        }
        out
    }
    fn node(id: &str, b: &Bookmark, depth: usize, nodes: &Nodes, kids: &dyn Fn(&str) -> Vec<String>, placed: &mut BTreeSet<String>) -> Node {
        let children = b.url.is_none().then(|| build(id, depth + 1, nodes, kids, placed));
        Node { id: id.to_string(), title: b.title.clone(), url: b.url.clone(), children }
    }
    let mut roots = build(ROOT, 0, &nodes, &kids, &mut placed);
    // What no folder reached, or a folder too deep: each at the top, with what it holds.
    for (id, b) in &nodes {
        if placed.insert(id.clone()) {
            roots.push(node(id, b, 0, &nodes, &kids, &mut placed));
        }
    }
    roots
}

// ---- spaces ----

/// The parts of a space that sync: what it is called and looks like, and
/// whether it has sign-ins of its own. Never its cookies, sign-ins, tabs or
/// downloads folder.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
struct Synced {
    name: String,
    colour: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shares_sign_ins: Option<bool>,
}

const SPACES_ORDER: &str = "order:spaces";

fn synced(text: &str) -> Option<Synced> {
    let s: Synced = value(text)?;
    Some(Synced {
        name: clean(&s.name),
        colour: if s.colour < COLOURS.len() { s.colour } else { 0 },
        icon: s.icon.filter(|i| ICONS.contains(&i.as_str())),
        shares_sign_ins: s.shares_sign_ins,
    })
}

/// One profile's spaces, in order, as items. Whether a space has sign-ins of
/// its own is what was agreed when it was first synced (`base`), never
/// changed after: a store can't change under a space's open tabs.
pub fn spaces(list: &[&Space], base: &Items) -> Items {
    let mut out = Items::new();
    for s in list.iter().filter(|s| is_id(&s.id)) {
        let key = format!("space:{}", s.id);
        let agreed = base.get(&key).and_then(|t| synced(t)).and_then(|a| a.shares_sign_ins);
        let item = Synced {
            name: s.name.clone(),
            colour: s.colour,
            icon: s.icon.clone(),
            shares_sign_ins: Some(agreed.unwrap_or(s.shares_sign_ins)),
        };
        out.insert(key, text(&item));
    }
    if !list.is_empty() {
        out.insert(SPACES_ORDER.into(), text(&list.iter().map(|s| &s.id).collect::<Vec<_>>()));
    }
    out
}

/// The spaces the merged items say `profile` has: names, colours and icons as
/// merged, in the merged order; what is only this PC's (the downloads
/// folder) kept; whether one has its own sign-ins never changed once it is
/// made. `home` is never taken away.
pub fn apply_spaces(live: &Items, local: &[&Space], profile: &str, home: &str) -> Vec<Space> {
    let found: std::collections::BTreeMap<String, Synced> = live
        .iter()
        .filter_map(|(k, t)| {
            let id = k.strip_prefix("space:").filter(|id| is_id(id))?;
            Some((id.to_string(), synced(t)?))
        })
        .collect();
    let mut ids: BTreeSet<String> = found.keys().cloned().collect();
    ids.insert(home.to_string());
    // A space whose id can't be synced, or whose item this build can't read,
    // is never synced away either.
    let unread = |s: &&&Space| !is_id(&s.id) || live.get(&format!("space:{}", s.id)).is_some_and(|t| synced(t).is_none());
    ids.extend(local.iter().filter(unread).map(|s| s.id.clone()));
    let order = live.get(SPACES_ORDER).and_then(|t| value::<Vec<String>>(t)).unwrap_or_default();
    let fallback: Vec<String> = local.iter().map(|s| s.id.clone()).collect();
    ordered(&ids, &order, &fallback)
        .into_iter()
        .map(|id| {
            let here = local.iter().find(|s| s.id == id);
            let mut space = here.map(|s| (*s).clone()).unwrap_or_else(|| Space {
                id: id.clone(),
                name: String::new(),
                colour: 0,
                icon: None,
                shares_sign_ins: false,
                downloads: None,
                profile: (profile != PERSONAL_ID).then(|| profile.to_string()),
            });
            if let Some(s) = found.get(&id) {
                space.name = s.name.clone();
                space.colour = s.colour;
                space.icon = s.icon.clone();
                if here.is_none() {
                    space.shares_sign_ins = s.shares_sign_ins.unwrap_or(false);
                }
            }
            if space.name.trim().is_empty() {
                space.name = "Space".into();
            }
            space
        })
        .collect()
}

/// Whether this build can read an item: only those can be found missing here
/// and so deleted everywhere, and only those are applied.
pub fn understood(key: &str, text: &str) -> bool {
    if let Some(k) = key.strip_prefix("set:") {
        return setting(k, text).is_some();
    }
    if key.starts_with("bm:") {
        return bookmark(text).is_some();
    }
    if key.starts_with("space:") {
        return synced(text).is_some();
    }
    if key.starts_with("pin:") {
        return pin(text).is_some();
    }
    if key.starts_with("title:pin:") {
        return value::<String>(text).is_some();
    }
    if key.starts_with("kids:") || key.starts_with("order:") {
        return value::<Vec<String>>(text).is_some();
    }
    false
}

// ---- pins ----

/// The parts of a pin that sync. Its title is the page's, which changes as
/// you browse: it goes once, beside it, for a PC that makes the pin anew.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Pin {
    letter: String,
    home: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// The most pins a space takes from the folder that this PC doesn't have yet.
pub const PINS_MAX: usize = 64;

fn pin(text: &str) -> Option<Pin> {
    let p: Pin = value(text)?;
    safe_url(&p.home).then(|| Pin { letter: clean(&p.letter).chars().take(2).collect(), home: p.home, name: p.name.map(|n| clean(&n)) })
}

/// One space's pins, in order, as items. A pin's title is the one already
/// agreed, if there is one.
pub fn pins(space: &str, defs: &[PinDef], base: &Items) -> Items {
    let mut out = Items::new();
    let defs: Vec<&PinDef> = defs.iter().filter(|d| is_id(&d.id)).collect();
    for d in &defs {
        out.insert(format!("pin:{space}:{}", d.id), text(&Pin { letter: d.letter.clone(), home: d.home.clone(), name: d.name.clone() }));
        let title = format!("title:pin:{}", d.id);
        let agreed = base.get(&title).cloned().unwrap_or_else(|| text(&d.title));
        out.insert(title, agreed);
    }
    if !defs.is_empty() {
        out.insert(format!("order:pins:{space}"), text(&defs.iter().map(|d| &d.id).collect::<Vec<_>>()));
    }
    out
}

/// The pins the merged items say `space` has, in the merged order. One whose
/// item this build can't read stays as this PC has it (`local`). Titles: the
/// one agreed, else this PC's, else the page's address.
pub fn apply_pins(live: &Items, space: &str, local: &[PinDef]) -> Vec<PinDef> {
    let prefix = format!("pin:{space}:");
    let mut found = std::collections::BTreeMap::new();
    for (key, t) in live.range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)) {
        let id = &key[prefix.len()..];
        if !is_id(id) {
            continue;
        }
        let here = local.iter().find(|d| d.id == id);
        let def = match pin(t) {
            Some(p) => PinDef { id: id.into(), letter: p.letter, home: p.home, title: String::new(), name: p.name },
            None if !understood(key, t) => match here {
                Some(d) => d.clone(),
                None => continue,
            },
            None => continue,
        };
        found.insert(id.to_string(), def);
    }
    let ids: BTreeSet<String> = found.keys().cloned().collect();
    let order = live.get(&format!("order:pins:{space}")).and_then(|t| value::<Vec<String>>(t)).unwrap_or_default();
    let fallback: Vec<String> = local.iter().map(|d| d.id.clone()).collect();
    // More pins than a column holds is a file gone wrong: past PINS_MAX, only
    // the ones this PC already has. None of those is ever unpinned for it.
    let mut room = PINS_MAX;
    ordered(&ids, &order, &fallback)
        .into_iter()
        .filter(|id| {
            let ours = local.iter().any(|d| d.id == *id);
            let take = ours || room > 0;
            if take && !ours {
                room -= 1;
            }
            take
        })
        .filter_map(|id| {
            let mut def = found.remove(&id)?;
            let agreed = live.get(&format!("title:pin:{id}")).and_then(|t| value::<String>(t)).map(|t| clean(&t));
            let here = local.iter().find(|d| d.id == id).map(|d| d.title.clone());
            def.title = agreed.or(here).filter(|t| !t.is_empty()).unwrap_or_else(|| def.home.clone());
            Some(def)
        })
        .collect()
}
