//! Bookmarks, in the window: Ctrl+Shift+B, the rows they add under the field,
//! and search://bookmarks. The list and its file are crates/bookmarks; this is
//! only when to read it, when to write it, and what the page may ask.
//!
//! Nothing is read until something needs it (Ctrl+Shift+B, a letter typed in
//! the field, the page): a launch never waits for bookmarks.json.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use searchwin_bookmarks::{Bookmarks, Node, Place, Toggled};
use searchwin_history::fuzzy;
use serde_json::{Value, json};

thread_local! {
    static BOOKMARKS: RefCell<Option<Bookmarks>> = const { RefCell::new(None) };
    /// Changed since last written; written with the session, once changes settle.
    static OWED: Cell<bool> = const { Cell::new(false) };
    /// The one Ctrl+Shift+B found kept already: the page opens on it, to rename or file it.
    static EDITING: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The profile on screen's bookmarks: Personal's are where they always were.
fn file() -> PathBuf {
    crate::spaces::folder().join("bookmarks.json")
}

/// Another profile came on screen: its bookmarks are read the next time
/// they are wanted. The window's save has written these already.
pub fn unload() {
    save();
    BOOKMARKS.with(|cell| *cell.borrow_mut() = None);
    EDITING.with(|e| *e.borrow_mut() = None);
}

fn with<R>(f: impl FnOnce(&mut Bookmarks) -> R) -> R {
    BOOKMARKS.with(|cell| f(cell.borrow_mut().get_or_insert_with(|| Bookmarks::load(&file()))))
}

/// A change made: written once changes settle, as the session and history are.
fn changed() {
    OWED.with(|o| o.set(true));
    crate::save_soon();
}

/// Called from the window's save: the file, if anything changed.
pub fn save() {
    if !OWED.with(|o| o.replace(false)) {
        return;
    }
    let written = BOOKMARKS.with(|cell| cell.borrow().as_ref().map(|b| b.save(&file())));
    if let Some(Err(error)) = written {
        crate::crash_log(&format!("bookmarks not saved: {error}"));
    }
}

/// Only places to go are kept: a web page or a file, never our own pages or a blank tab.
fn keepable(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https" | "file"))
}

/// Ctrl+Shift+B. As upstream (Browser.bookmarkCurrent), a private tab's page
/// can be kept too: you asked for it by name. A page kept already is never
/// taken away by the same keys; its row opens in search://bookmarks, to
/// rename or file it.
pub fn bookmark_front() {
    let Some((url, title)) = crate::front_page() else { return };
    if !keepable(&url) {
        return;
    }
    match with(|b| b.toggle(&url, &title)) {
        Toggled::Added(_) => changed(),
        Toggled::Already(id) => {
            EDITING.with(|e| *e.borrow_mut() = Some(id));
            crate::open_page("search://bookmarks");
            // Already open: it is told to look again, and finds the row to edit.
            crate::page::post_to_internal("bookmarks", &json!({ "event": "changed" }).to_string());
        }
    }
}

/// Whether this address is kept, for `bench state`.
pub fn contains(url: &str) -> bool {
    with(|b| b.contains(url))
}

/// The rows under the field for what is typed (Bookmarks.places), and apart,
/// those found only by forgiving a typo (upstream's `slips`), which go under
/// where you have been.
pub fn places(typed: &str, limit: usize) -> (Vec<Place>, Vec<Place>) {
    with(|b| {
        let found = b.places(typed, limit);
        if found.len() >= limit {
            return (found, Vec::new());
        }
        let needle = typed.trim().to_lowercase();
        let words: Vec<&str> = needle.split_whitespace().collect();
        if needle.len() < 3 || words.is_empty() {
            return (found, Vec::new());
        }
        let mut slips = Vec::new();
        sites(b.roots(), &mut |title, url| {
            if slips.len() + found.len() < limit
                && !found.iter().any(|p| p.url == url)
                && fuzzy::matches(&words, &format!("{} {}", title.to_lowercase(), url.to_lowercase())) == Some(0)
            {
                slips.push(Place { title: title.to_string(), url: url.to_string() });
            }
        });
        (found, slips)
    })
}

/// The grey rest of a kept address for what is typed, when history has none.
pub fn complete(typed: &str) -> Option<String> {
    with(|b| searchwin_history::complete_from(typed, b.urls().iter().map(String::as_str)))
}

fn sites(nodes: &[Node], each: &mut impl FnMut(&str, &str)) {
    for node in nodes {
        match (&node.url, &node.children) {
            (Some(url), _) if keepable(url) => each(&node.title, url),
            (None, Some(kids)) => sites(kids, each),
            _ => {}
        }
    }
}

/// What search://bookmarks asks. `asked` is the page's message, already known
/// to come from one of our pages; the answer goes back to it.
pub fn message(op: &str, asked: &Value) -> Value {
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    // "id" is the question's own number (common.js ask), so a row is "node".
    let id = text("node");
    let folder = asked.get("folder").and_then(Value::as_str).map(str::to_string);
    let done = |ok: bool| {
        if ok {
            changed();
        }
        json!({ "ok": ok })
    };
    match op {
        "bookmarks.list" => list(&text("query")),
        "bookmarks.rename" => done(with(|b| b.rename(&id, &text("title")))),
        "bookmarks.update" => {
            let url = text("url");
            // Only a place to go: a javascript: or search: address kept here would run when clicked.
            if !keepable(&url) {
                return json!({ "ok": false, "error": "not an address" });
            }
            done(with(|b| b.update(&id, None, Some(&url))))
        }
        "bookmarks.remove" => done(with(|b| b.remove(&id))),
        "bookmarks.folder" => {
            let made = with(|b| b.add_folder(folder.as_deref(), &text("title")));
            changed();
            json!({ "ok": true, "made": made })
        }
        "bookmarks.move" => {
            let index = asked.get("index").and_then(Value::as_u64).map(|i| i as usize);
            done(with(|b| b.move_to(&id, folder.as_deref(), index)))
        }
        "bookmarks.open" => {
            let urls = with(|b| b.urls_in(&id));
            crate::open_in_tabs(&urls);
            json!({ "opened": urls.len() })
        }
        "bookmarks.import" => import(&text("from")),
        _ => json!({ "error": "unknown op" }),
    }
}

/// The whole tree for the page, or the hits for what was searched, with the
/// folders a row can be moved to and the row Ctrl+Shift+B asked to edit.
fn list(query: &str) -> Value {
    let editing = EDITING.with(|e| e.borrow_mut().take());
    with(|b| {
        let folders: Vec<Value> =
            b.folders().iter().map(|(n, depth)| json!({ "id": n.id, "title": n.title, "depth": depth })).collect();
        let mut value = json!({
            "tree": serde_json::to_value(b.roots()).unwrap_or(Value::Null),
            "folders": folders,
            "edit": editing,
        });
        if !query.trim().is_empty() {
            let found: Vec<Value> = b
                .search(query)
                .into_iter()
                .filter_map(|f| {
                    let node = b.get(&f.id)?;
                    Some(json!({ "id": node.id, "title": node.title, "url": node.url, "path": f.path, "folder": node.is_folder(), "parent": b.parent_of(&f.id) }))
                })
                .collect();
            value["found"] = Value::Array(found);
        }
        value
    })
}

/// Chrome's or Edge's own Bookmarks file, read only now that you asked.
/// SEARCHWIN_IMPORT_FILE is a test hook: a fixture read in place of the
/// real profile, so a test run never opens your browser's files.
fn import(from: &str) -> Value {
    let (name, below) = match from {
        "chrome" => ("Chrome", ["Google", "Chrome", "User Data", "Default", "Bookmarks"]),
        "edge" => ("Edge", ["Microsoft", "Edge", "User Data", "Default", "Bookmarks"]),
        _ => return json!({ "error": "unknown browser" }),
    };
    let path = match std::env::var_os("SEARCHWIN_IMPORT_FILE").filter(|p| !p.is_empty()) {
        Some(fixture) => PathBuf::from(fixture),
        None => {
            let Some(base) = std::env::var_os("LOCALAPPDATA") else { return json!({ "error": "not found" }) };
            below.iter().fold(PathBuf::from(base), |path, part| path.join(part))
        }
    };
    let Ok(data) = std::fs::read(&path) else { return json!({ "error": "not found" }) };
    let Some(incoming) = searchwin_bookmarks::from_chrome(&data) else { return json!({ "error": "unreadable" }) };
    let (added, already) = take(incoming, name);
    json!({ "added": added, "already": already })
}

/// Another browser's bookmarks, kept apart under its name (crate::import too).
pub fn take(incoming: Vec<searchwin_bookmarks::Node>, from: &str) -> (usize, usize) {
    let (added, already) = with(|b| b.take(incoming, from));
    if added > 0 {
        changed();
    }
    (added, already)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_places_to_go_are_kept() {
        assert!(keepable("https://a.test/"));
        assert!(keepable("file:///C:/x.txt"));
        assert!(!keepable("search://bookmarks"));
        assert!(!keepable("javascript:alert(1)"));
        assert!(!keepable("about:blank"));
        assert!(!keepable(""));
    }
}
