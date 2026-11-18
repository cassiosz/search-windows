//! Folders and sites, kept in a small file. Port of the model half of
//! upstream Sources/Search/Bookmarks.swift (and of BookmarksBar.swift, which
//! is only the top of this same list). Pure logic: no window, no WebView2.
//!
//! The file is upstream's own shape: a JSON array of
//! `{id, title, url?, children?}`, a folder being the one without a `url`.
//! Ids are UUID-looking strings so a file written by the Mac app loads here
//! and the other way round.
//!
//! Mutators return whether/what changed and never save: the caller calls
//! `save` (as upstream's `save()` after each change).

mod chrome;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub use chrome::from_chrome;

/// A site or a folder (Bookmarks.swift `Bookmark`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Node {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// None for a folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Node>>,
}

impl Node {
    /// A site. An empty title becomes its address as the field shows it
    /// (`Bookmark.site`).
    pub fn site(title: &str, url: &str) -> Node {
        let title = if title.trim().is_empty() { pretty(url) } else { title.to_string() };
        Node { id: new_id(), title, url: Some(url.to_string()), children: None }
    }

    pub fn folder(title: &str, children: Vec<Node>) -> Node {
        Node { id: new_id(), title: title.to_string(), url: None, children: Some(children) }
    }

    pub fn is_folder(&self) -> bool {
        self.url.is_none()
    }

    /// A site whose name was emptied: on the bar it is its icon alone. A
    /// folder always has a name (`unnamed`).
    pub fn unnamed(&self) -> bool {
        !self.is_folder() && self.title.trim().is_empty()
    }

    /// What a list shows: its name, or for an unnamed one its address.
    pub fn label(&self) -> String {
        if self.unnamed() { pretty(self.url.as_deref().unwrap_or("")) } else { self.title.clone() }
    }

    pub fn host(&self) -> Option<String> {
        let url = Url::parse(self.url.as_deref()?).ok()?;
        url.host_str().map(str::to_lowercase)
    }
}

/// An address as a list reads it: no scheme, no `www.`, no lone trailing `/`.
fn pretty(url: &str) -> String {
    let s = url.split_once("://").map_or(url, |(_, rest)| rest);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.strip_suffix('/').unwrap_or(s).to_string()
}

/// A fresh id in the shape of a UUID v4 (so upstream can load the file). Not
/// secret: the clock and a counter through a mixer.
fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mix = |mut x: u64| {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let a = mix(t ^ n.rotate_left(32));
    let b = mix(a ^ n);
    let variant = (((b >> 48) as u16) & 0x3FFF) | 0x8000;
    format!(
        "{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        (a as u16) & 0x0FFF,
        variant,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// Past this depth a file is no one's filing; deeper folders are dropped on
/// load (the importer in upstream stops at 64 for the same reason).
const MAX_DEPTH: usize = 64;

/// One hit of `search`: the node and the names of the folders above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub path: Vec<String>,
}

/// One row offered under the address field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub title: String,
    pub url: String,
}

/// What Ctrl+Shift+B did (`Browser.bookmarkCurrent`): upstream never removes
/// on this shortcut; a page kept already just gets its card (rename/file it).
/// The app calls `remove` itself if it wants Chrome's toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toggled {
    Added(String),
    Already(String),
}

/// A search-ready site, made once per change: the field asks on every key.
struct Site {
    name: String,
    title: String,
    address: String,
    url: String,
    findable: String,
}

#[derive(Default)]
pub struct Bookmarks {
    roots: Vec<Node>,
    kept: HashSet<String>,
    sites: Vec<Site>,
}

impl Bookmarks {
    pub fn new() -> Bookmarks {
        Bookmarks::default()
    }

    /// Reads the file. Missing is empty; corrupt is empty too, but the bad
    /// file is moved aside to `.bad` so the next save doesn't destroy it
    /// (upstream `Store.quarantine`). Never a panic.
    pub fn load(path: &Path) -> Bookmarks {
        let Ok(data) = std::fs::read(path) else { return Bookmarks::new() };
        match serde_json::from_slice::<Vec<Node>>(&data) {
            Ok(list) => Bookmarks::from_roots(list),
            Err(_) => {
                let mut bad = path.as_os_str().to_owned();
                bad.push(".bad");
                let _ = std::fs::rename(path, bad);
                Bookmarks::new()
            }
        }
    }

    /// A list from a file or an import, made sound: every node gets a unique
    /// id, a node with a url is a site whatever else it carries, and folders
    /// nested past `MAX_DEPTH` are dropped.
    pub fn from_roots(mut roots: Vec<Node>) -> Bookmarks {
        let mut seen = HashSet::new();
        fix(&mut roots, &mut seen, 0);
        let mut b = Bookmarks { roots, kept: HashSet::new(), sites: Vec::new() };
        b.rebuild();
        b
    }

    /// Atomic: a temp file beside it, then a rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let data = serde_json::to_vec(&self.roots).map_err(std::io::Error::other)?;
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

    fn rebuild(&mut self) {
        self.kept = urls_of(&self.roots).into_iter().map(canonical).collect();
        self.sites = sites(&self.roots);
    }

    // reading

    pub fn roots(&self) -> &[Node] {
        &self.roots
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// How many sites, folders not counted (`count`).
    pub fn count(&self) -> usize {
        urls_of(&self.roots).len()
    }

    /// Every address, in order, folders opened.
    pub fn urls(&self) -> Vec<String> {
        urls_of(&self.roots).into_iter().map(str::to_string).collect()
    }

    /// Every address under a folder, in order, nested folders opened: the
    /// Bookmarks menu's "open whole folder in new tabs". Empty for an unknown
    /// id; for a site, just its own address. Only places to go (http, https,
    /// file), never a bookmarklet's `javascript:`.
    pub fn urls_in(&self, id: &str) -> Vec<String> {
        let Some(node) = self.get(id) else { return Vec::new() };
        urls_of(std::slice::from_ref(node)).into_iter().filter(|u| is_place(u)).map(str::to_string).collect()
    }

    /// Every folder with its depth, for "move to folder" lists.
    pub fn folders(&self) -> Vec<(&Node, usize)> {
        fn walk<'a>(nodes: &'a [Node], depth: usize, out: &mut Vec<(&'a Node, usize)>) {
            for n in nodes.iter().filter(|n| n.is_folder()) {
                out.push((n, depth));
                walk(n.children.as_deref().unwrap_or(&[]), depth + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, 0, &mut out);
        out
    }

    /// Every site and folder whose title or address holds all the words, any
    /// order, with the folders it sits in (`matches`, the rule of
    /// chrome.bookmarks.search). For the bookmarks page.
    pub fn search(&self, text: &str) -> Vec<Found> {
        let lowered = text.to_lowercase();
        let words: Vec<&str> = lowered.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }
        fn walk(nodes: &[Node], path: &mut Vec<String>, words: &[&str], out: &mut Vec<Found>) {
            for n in nodes {
                let hay = format!("{} {}", n.title, n.url.as_deref().unwrap_or("")).to_lowercase();
                if words.iter().all(|w| hay.contains(w)) {
                    out.push(Found { id: n.id.clone(), path: path.clone() });
                }
                if let Some(kids) = &n.children {
                    path.push(n.title.clone());
                    walk(kids, path, words, out);
                    path.pop();
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, &mut Vec::new(), &words, &mut out);
        out
    }

    /// The sites for what is typed in the address field, best `limit` first:
    /// every word, in any order, in the name or the address (`places`). Two
    /// letters and more. Address starting with the text first, then the name
    /// (or a word of it), then a word anywhere; among equals the shorter
    /// address. Only places to go: http, https, file.
    /// (Left out: upstream's tolerance for a typo, History's Fuzzy.)
    pub fn places(&self, typed: &str, limit: usize) -> Vec<Place> {
        let needle = typed.trim().to_lowercase();
        let words: Vec<&str> = needle.split_whitespace().collect();
        let Some(first) = words.first() else { return Vec::new() };
        if needle.chars().count() < 2 {
            return Vec::new();
        }
        let bare = pretty(&needle);
        let mut best: Vec<(&Site, u8)> = Vec::new();
        for site in &self.sites {
            if !words.iter().all(|w| site.findable.contains(w)) {
                continue;
            }
            let score = if site.address.starts_with(&bare) {
                3
            } else if site.title.starts_with(&needle) || site.title.split(' ').any(|w| w.starts_with(first)) {
                2
            } else {
                1
            };
            let at = best
                .iter()
                .position(|(s, sc)| score > *sc || (score == *sc && site.address.len() < s.address.len()))
                .unwrap_or(best.len());
            if at >= limit {
                continue;
            }
            best.insert(at, (site, score));
            best.truncate(limit);
        }
        best.into_iter().map(|(s, _)| Place { title: s.name.clone(), url: s.url.clone() }).collect()
    }

    /// The ids of the folders `id` sits in, outermost first; empty at the top
    /// level, None when it isn't here at all (`path(to:)`).
    pub fn path_to(&self, id: &str) -> Option<Vec<String>> {
        fn walk(nodes: &[Node], id: &str, above: &mut Vec<String>) -> bool {
            for n in nodes {
                if n.id == id {
                    return true;
                }
                if let Some(kids) = &n.children {
                    above.push(n.id.clone());
                    if walk(kids, id, above) {
                        return true;
                    }
                    above.pop();
                }
            }
            false
        }
        let mut above = Vec::new();
        walk(&self.roots, id, &mut above).then_some(above)
    }

    /// The folder `id` sits in, None at the top level (or if not here).
    pub fn parent_of(&self, id: &str) -> Option<String> {
        self.path_to(id)?.pop()
    }

    pub fn get(&self, id: &str) -> Option<&Node> {
        find(&self.roots, &|n| n.id == id)
    }

    /// The one kept for this address, wherever it is filed.
    pub fn find_url(&self, url: &str) -> Option<&Node> {
        let url = canonical(url);
        find(&self.roots, &|n| n.url.as_deref().is_some_and(|u| canonical(u) == url))
    }

    pub fn contains(&self, url: &str) -> bool {
        self.kept.contains(&canonical(url))
    }

    // changing

    /// The page, at the end of the list; None if kept already. The title is
    /// the page's: renaming and filing is the card's job (`add`).
    pub fn add(&mut self, url: &str, title: &str) -> Option<String> {
        if self.contains(url) {
            return None;
        }
        let node = Node::site(title, &canonical(url));
        let id = node.id.clone();
        self.roots.push(node);
        self.rebuild();
        Some(id)
    }

    /// Ctrl+Shift+B (`bookmarkCurrent`).
    pub fn toggle(&mut self, url: &str, title: &str) -> Toggled {
        match self.add(url, title) {
            Some(id) => Toggled::Added(id),
            None => Toggled::Already(self.find_url(url).map(|n| n.id.clone()).unwrap_or_default()),
        }
    }

    /// A new folder under `parent` (top level for None or a missing folder),
    /// at the end. Returns its id. An empty name is "Folder".
    pub fn add_folder(&mut self, parent: Option<&str>, title: &str) -> String {
        let title = if title.trim().is_empty() { "Folder" } else { title.trim() };
        self.insert(Node::folder(title, Vec::new()), parent, None)
    }

    /// A node filed under `parent` at `index`, or at the top level for None
    /// or a folder that isn't there (`insert`, chrome.bookmarks.create).
    pub fn insert(&mut self, node: Node, parent: Option<&str>, index: Option<usize>) -> String {
        let id = node.id.clone();
        let mut roots = std::mem::take(&mut self.roots);
        if let Err(node) = place(node, parent, index, &mut roots) {
            let _ = place(node, None, index, &mut roots);
        }
        self.roots = roots;
        self.rebuild();
        id
    }

    /// A site, or a folder with everything in it.
    pub fn remove(&mut self, id: &str) -> bool {
        let mut roots = std::mem::take(&mut self.roots);
        let gone = detach(id, &mut roots).is_some();
        self.roots = roots;
        if gone {
            self.rebuild();
        }
        gone
    }

    /// Takes a site or whole folder out of where it sits and puts it in
    /// another folder (top level for None) at `index` among what is there, or
    /// at the end. The index counts the list as it read before the move, so
    /// dropping a row just below itself leaves it. A folder into itself or
    /// into its own children is refused (it would erase it).
    pub fn move_to(&mut self, id: &str, folder: Option<&str>, index: Option<usize>) -> bool {
        if folder == Some(id) {
            return false;
        }
        let mut index = index;
        if let (Some(at), Some(from)) = (index, self.siblings(folder).iter().position(|n| n.id == id))
            && from < at
        {
            index = Some(at - 1); // it leaves a gap above the place it goes to
        }
        let mut working = self.roots.clone();
        let Some(node) = detach(id, &mut working) else { return false };
        if folder.is_some_and(|f| holds(f, &node)) {
            return false;
        }
        if place(node, folder, index, &mut working).is_err() {
            return false;
        }
        self.roots = working;
        self.rebuild();
        true
    }

    fn siblings(&self, folder: Option<&str>) -> &[Node] {
        match folder {
            None => &self.roots,
            Some(id) => find(&self.roots, &|n| n.id == id).and_then(|n| n.children.as_deref()).unwrap_or(&[]),
        }
    }

    /// A new name. A folder always has one, so an emptied folder name is
    /// refused; a site may be emptied (icon alone on the bar).
    pub fn rename(&mut self, id: &str, title: &str) -> bool {
        self.update(id, Some(title), None)
    }

    /// New title and/or address (`update`, chrome.bookmarks.update). A
    /// folder has no address to change.
    pub fn update(&mut self, id: &str, title: Option<&str>, url: Option<&str>) -> bool {
        let Some(node) = find_mut(&mut self.roots, id) else { return false };
        if let Some(t) = title {
            if node.is_folder() && t.trim().is_empty() {
                return false;
            }
            node.title = t.to_string();
        }
        if let (Some(u), false) = (url, node.is_folder()) {
            node.url = Some(u.to_string());
        }
        self.rebuild();
        true
    }

    /// Another browser's bookmarks, kept apart in a folder of its name unless
    /// there was nothing here yet. Again adds only what is new: a page already
    /// at that level is left, a folder of the same name is gone into, nothing
    /// you added or moved is taken away. Returns (added, already) in pages
    /// (`take`).
    pub fn take(&mut self, incoming: Vec<Node>, from: &str) -> (usize, usize) {
        let mut incoming = Bookmarks::from_roots(incoming).roots; // fresh unique ids
        let (mut added, mut already) = (0, 0);
        let mut roots = std::mem::take(&mut self.roots);
        // Upstream remembers which browser filled the top level; without that
        // memory, the test it falls back on: most of its pages already sit
        // here, so it is the one that did (`intoTop`).
        let theirs: Vec<String> = urls_of(&incoming).into_iter().map(canonical).collect();
        let shared = theirs.iter().filter(|u| self.kept.contains(*u)).count();
        let own_folder = roots.iter().any(|n| n.is_folder() && n.title == from);
        if !own_folder && (roots.is_empty() || (!theirs.is_empty() && shared * 2 >= theirs.len())) {
            merge(&mut incoming, &mut roots, &mut added, &mut already);
        } else {
            let at = roots.iter().position(|n| n.is_folder() && n.title == from).unwrap_or_else(|| {
                roots.push(Node::folder(from, Vec::new()));
                roots.len() - 1
            });
            let mut kids = roots[at].children.take().unwrap_or_default();
            merge(&mut incoming, &mut kids, &mut added, &mut already);
            roots[at].children = Some(kids);
        }
        self.roots = roots;
        self.rebuild();
        (added, already)
    }
}

// tree helpers

fn fix(nodes: &mut Vec<Node>, seen: &mut HashSet<String>, depth: usize) {
    if depth >= MAX_DEPTH {
        nodes.retain(|n| !n.is_folder());
    }
    for n in nodes.iter_mut() {
        if n.id.is_empty() || !seen.insert(n.id.clone()) {
            n.id = new_id();
            seen.insert(n.id.clone());
        }
        if n.url.is_some() {
            n.children = None;
        } else {
            fix(n.children.get_or_insert_with(Vec::new), seen, depth + 1);
        }
    }
}

fn urls_of(nodes: &[Node]) -> Vec<&str> {
    let mut out = Vec::new();
    for n in nodes {
        match (&n.url, &n.children) {
            (Some(u), _) => out.push(u.as_str()),
            (None, Some(kids)) => out.extend(urls_of(kids)),
            _ => {}
        }
    }
    out
}

fn is_place(url: &str) -> bool {
    let scheme = url.split_once(':').map_or("", |(s, _)| s).to_ascii_lowercase();
    matches!(scheme.as_str(), "http" | "https" | "file")
}

/// An address as `contains` compares it: the parsed form (upstream's
/// `absoluteString`), so "https://a.com" and "https://a.com/" are one page.
fn canonical(url: &str) -> String {
    Url::parse(url).map_or_else(|_| url.to_string(), |u| u.as_str().to_string())
}

fn sites(nodes: &[Node]) -> Vec<Site> {
    let mut out = Vec::new();
    for n in nodes {
        if let Some(kids) = &n.children {
            out.extend(sites(kids));
        } else if let Some(url) = n.url.as_deref().filter(|u| is_place(u)) {
            let name = if n.unnamed() { String::new() } else { n.title.clone() };
            let title = name.to_lowercase();
            let address = pretty(&url.to_lowercase());
            let findable = format!("{title} {address}");
            out.push(Site { name, title, address, url: url.to_string(), findable });
        }
    }
    out
}

fn find<'a>(nodes: &'a [Node], test: &dyn Fn(&Node) -> bool) -> Option<&'a Node> {
    for n in nodes {
        if test(n) {
            return Some(n);
        }
        if let Some(found) = n.children.as_deref().and_then(|k| find(k, test)) {
            return Some(found);
        }
    }
    None
}

fn find_mut<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Node> {
    for n in nodes {
        if n.id == id {
            return Some(n);
        }
        if let Some(kids) = n.children.as_mut()
            && let Some(found) = find_mut(kids, id)
        {
            return Some(found);
        }
    }
    None
}

fn detach(id: &str, nodes: &mut Vec<Node>) -> Option<Node> {
    if let Some(i) = nodes.iter().position(|n| n.id == id) {
        return Some(nodes.remove(i));
    }
    nodes.iter_mut().find_map(|n| n.children.as_mut().and_then(|k| detach(id, k)))
}

/// `node` into folder `id` (top level for None) at `index`, clamped, or at the
/// end. Gives the node back when there is no such folder.
fn place(node: Node, id: Option<&str>, index: Option<usize>, nodes: &mut Vec<Node>) -> Result<(), Node> {
    let Some(id) = id else {
        let at = index.unwrap_or(nodes.len()).min(nodes.len());
        nodes.insert(at, node);
        return Ok(());
    };
    let mut node = node;
    for n in nodes.iter_mut() {
        if n.id == id && n.is_folder() {
            let kids = n.children.get_or_insert_with(Vec::new);
            let at = index.unwrap_or(kids.len()).min(kids.len());
            kids.insert(at, node);
            return Ok(());
        }
        if let Some(kids) = n.children.as_mut() {
            match place(node, Some(id), index, kids) {
                Ok(()) => return Ok(()),
                Err(back) => node = back,
            }
        }
    }
    Err(node)
}

/// `id` is `node` itself or somewhere inside it.
fn holds(id: &str, node: &Node) -> bool {
    node.id == id || node.children.as_deref().unwrap_or(&[]).iter().any(|c| holds(id, c))
}

/// `incoming` into `nodes`, level by level: a folder into the folder of the
/// same name, a page only if the same address isn't already at that level.
fn merge(incoming: &mut Vec<Node>, nodes: &mut Vec<Node>, added: &mut usize, already: &mut usize) {
    for node in incoming.drain(..) {
        if node.is_folder() {
            match nodes.iter().position(|n| n.is_folder() && n.title == node.title) {
                Some(at) => {
                    let mut kids = nodes[at].children.take().unwrap_or_default();
                    let mut theirs = node.children.unwrap_or_default();
                    merge(&mut theirs, &mut kids, added, already);
                    nodes[at].children = Some(kids);
                }
                None => {
                    *added += urls_of(std::slice::from_ref(&node)).len();
                    nodes.push(node);
                }
            }
        } else if nodes.iter().any(|n| !n.is_folder() && n.url == node.url) {
            *already += 1;
        } else {
            *added += 1;
            nodes.push(node);
        }
    }
}
