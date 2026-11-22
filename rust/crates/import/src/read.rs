//! What a browser kept: its bookmarks and where it has been
//! (Import.swift `bookmarkRead`, `places`), Chromium's and Firefox's alike.

use crate::browsers::{Browser, Engine, read_capped};
use crate::sqlite::Db;
use searchwin_bookmarks::{Node, from_chrome};
use searchwin_history::Entry;
use std::collections::HashMap;
use std::path::Path;
use url::Url;

/// At most this many places from each profile, as upstream.
pub const HISTORY_LIMIT: usize = 3000;

/// Microseconds from 1601 (Chromium's clock) to 1970.
const WEBKIT_TO_UNIX_US: i64 = 11_644_473_600_000_000;

/// The bookmarks of one profile, or every one's run together, and whether
/// every profile asked for read cleanly: false when there was none to read,
/// or a file there didn't read. A profile with no bookmarks file has none and
/// reads cleanly. Replace takes nothing out unless this is true.
pub fn bookmarks(browser: &Browser, profile: Option<&str>) -> (Vec<Node>, bool) {
    let mut out = Vec::new();
    let mut complete = browser.profiles(profile).next().is_some();
    for p in browser.profiles(profile) {
        let read = match browser.engine {
            Engine::Chromium => {
                let file = p.path.join("Bookmarks");
                if !file.exists() {
                    continue;
                }
                read_capped(&file, 64 << 20).and_then(|bytes| from_chrome(&bytes))
            }
            Engine::Firefox => firefox_bookmarks(&p.path.join("places.sqlite")),
        };
        match read {
            Some(nodes) => out.extend(nodes),
            None => complete = false,
        }
    }
    (out, complete)
}

/// Where the browser has been, newest first, at most `limit`: a page kept
/// by two profiles comes once, its visits added up.
pub fn history(browser: &Browser, profile: Option<&str>, limit: usize) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for p in browser.profiles(profile) {
        let rows = match browser.engine {
            Engine::Chromium => chromium_history(&p.path.join("History"), limit),
            Engine::Firefox => firefox_history(&p.path.join("places.sqlite"), limit),
        };
        for entry in rows {
            match at.get(&entry.url) {
                Some(&i) => {
                    let kept = &mut out[i];
                    kept.count = kept.count.saturating_add(entry.count);
                    kept.last = kept.last.max(entry.last);
                    if kept.title.is_empty() {
                        kept.title = entry.title;
                    }
                }
                None => {
                    at.insert(entry.url.clone(), out.len());
                    out.push(entry);
                }
            }
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.last));
    out.truncate(limit);
    out
}

fn chromium_history(file: &Path, limit: usize) -> Vec<Entry> {
    let sql = format!(
        "SELECT url, title, visit_count, last_visit_time FROM urls \
         WHERE hidden = 0 AND visit_count > 0 ORDER BY last_visit_time DESC LIMIT {limit}"
    );
    places(file, &sql, |stamp| (stamp - WEBKIT_TO_UNIX_US) / 1000)
}

fn firefox_history(file: &Path, limit: usize) -> Vec<Entry> {
    // Microseconds since 1970: no 1601 offset, unlike Chromium's.
    let sql = format!(
        "SELECT url, title, visit_count, last_visit_date FROM moz_places \
         WHERE hidden = 0 AND visit_count > 0 AND last_visit_date IS NOT NULL \
         ORDER BY last_visit_date DESC LIMIT {limit}"
    );
    places(file, &sql, |stamp| stamp / 1000)
}

/// Rows of (url, title, count, stamp) from a copy of `file`; only http and
/// https; a missing date is now.
fn places(file: &Path, sql: &str, to_ms: impl Fn(i64) -> i64) -> Vec<Entry> {
    if !file.is_file() {
        return Vec::new();
    }
    let Ok(db) = Db::snapshot(file) else { return Vec::new() };
    let now = searchwin_history::now_ms();
    let mut out = Vec::new();
    let _ = db.rows(sql, |row| {
        let Some(url) = row.text(0).and_then(|u| web(&u)) else { return };
        let stamp = row.int(3);
        let last = if stamp > 0 { u64::try_from(to_ms(stamp)).unwrap_or(0) } else { now };
        out.push(Entry {
            url,
            title: row.text(1).unwrap_or_default(),
            count: u32::try_from(row.int(2).max(1)).unwrap_or(u32::MAX),
            last,
        });
    });
    out
}

/// Firefox keeps bookmarks as rows in moz_bookmarks pointing at pages in
/// moz_places, each with a parent: the tree is rebuilt from the flat list.
/// The toolbar's come first at the top, then the menu's, then Other and
/// Mobile as folders — the order Chromium's come in. The tags folder is
/// Firefox's bookkeeping and is left out; each folder is taken once and no
/// deeper than 64, so a damaged file can't loop.
fn firefox_bookmarks(file: &Path) -> Option<Vec<Node>> {
    struct Raw {
        id: i64,
        kind: i64,
        parent: i64,
        title: String,
        url: Option<String>,
        guid: String,
    }
    let db = Db::snapshot(file).ok()?;
    let mut items = Vec::new();
    db.rows(
        "SELECT b.id, b.type, b.parent, b.title, p.url, b.guid FROM moz_bookmarks b \
         LEFT JOIN moz_places p ON b.fk = p.id WHERE b.type IN (1, 2) ORDER BY b.position ASC",
        |row| {
            items.push(Raw {
                id: row.int(0),
                kind: row.int(1),
                parent: row.int(2),
                title: row.text(3).unwrap_or_default(),
                url: row.text(4),
                guid: row.text(5).unwrap_or_default(),
            })
        },
    )
    .ok()?;
    let mut by_parent: HashMap<i64, Vec<&Raw>> = HashMap::new();
    let mut by_guid: HashMap<&str, i64> = HashMap::new();
    for item in &items {
        by_parent.entry(item.parent).or_default().push(item);
        by_guid.insert(&item.guid, item.id);
    }
    let mut taken = std::collections::HashSet::new();
    fn children(
        parent: i64,
        depth: usize,
        by_parent: &HashMap<i64, Vec<&Raw>>,
        taken: &mut std::collections::HashSet<i64>,
    ) -> Vec<Node> {
        if depth >= 64 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for item in by_parent.get(&parent).map(Vec::as_slice).unwrap_or_default() {
            if item.kind == 1 {
                let Some(url) = item.url.as_deref().and_then(web) else { continue };
                let title = item.title.trim();
                let title = if title.is_empty() { Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default() } else { title.to_string() };
                out.push(Node::site(&title, &url));
            } else if item.kind == 2 && item.guid != "tags________" && taken.insert(item.id) {
                let title = if item.title.trim().is_empty() { "Folder" } else { item.title.trim() };
                out.push(Node::folder(title, children(item.id, depth + 1, by_parent, taken)));
            }
        }
        out
    }
    let mut out = Vec::new();
    for guid in ["toolbar_____", "menu________"] {
        if let Some(&id) = by_guid.get(guid) {
            out.extend(children(id, 0, &by_parent, &mut taken));
        }
    }
    for (guid, name) in [("unfiled_____", "Other bookmarks"), ("mobile______", "Mobile bookmarks")] {
        if let Some(&id) = by_guid.get(guid) {
            let kids = children(id, 0, &by_parent, &mut taken);
            if !kids.is_empty() {
                out.push(Node::folder(name, kids));
            }
        }
    }
    Some(out)
}

/// An http or https address, without any password written into it.
pub(crate) fn web(text: &str) -> Option<String> {
    let mut url = Url::parse(text.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some(url.into())
}
