//! Reading a Chrome/Edge `Bookmarks` file: JSON with `roots.bookmark_bar`,
//! `roots.other` and `roots.synced`, each a folder of `{type, name, url?,
//! children?}`. Upstream imports the Netscape HTML export instead
//! (BookmarksFile.swift); the rules carried over are the same: only http and
//! https are kept, an empty title becomes the host, the bookmarks bar's pages
//! go to the top level, every other folder stays a folder (empty ones too).

use crate::{Bookmarks, Node};
use serde_json::Value;
use url::Url;

/// The file's bookmarks, folders and all; None for a file that isn't one.
/// Chrome's "Other bookmarks" and "Mobile bookmarks" (synced) come as folders
/// of those names, only when they hold something.
pub fn from_chrome(json: &[u8]) -> Option<Vec<Node>> {
    let doc: Value = serde_json::from_slice(json).ok()?;
    let roots = doc.get("roots")?.as_object()?;
    let mut out = Vec::new();
    if let Some(bar) = roots.get("bookmark_bar") {
        out.extend(children(bar, 0));
    }
    for (key, name) in [("other", "Other bookmarks"), ("synced", "Mobile bookmarks")] {
        let kids = roots.get(key).map(|v| children(v, 0)).unwrap_or_default();
        if !kids.is_empty() {
            out.push(Node::folder(name, kids));
        }
    }
    Some(Bookmarks::from_roots(out).roots().to_vec())
}

fn children(folder: &Value, depth: usize) -> Vec<Node> {
    let Some(list) = folder.get("children").and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::new();
    for item in list {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("").trim();
        match item.get("type").and_then(Value::as_str) {
            Some("url") => {
                let Some(href) = item.get("url").and_then(Value::as_str) else { continue };
                let Ok(url) = Url::parse(href) else { continue };
                if !matches!(url.scheme(), "http" | "https") {
                    continue;
                }
                let title = if name.is_empty() { url.host_str().unwrap_or(href) } else { name };
                out.push(Node::site(title, url.as_str()));
            }
            // Past 64 deep nobody files; the pages join the folder above.
            Some("folder") if depth >= 64 => out.extend(children(item, depth + 1)),
            Some("folder") => out.push(Node::folder(if name.is_empty() { "Folder" } else { name }, children(item, depth + 1))),
            _ => {}
        }
    }
    out
}
