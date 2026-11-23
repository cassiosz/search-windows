//! Search's own pages, search://settings, history, bookmarks and downloads, each in a tab
//! of its own as upstream made them (InternalPages.swift): a tab you can leave
//! open, come back to and close like any other.
//!
//! They are web pages served from inside the exe, under a scheme WebView2 is
//! told about, and they talk to the window through WebView2's message
//! channel. Only a page whose address is search:// is ever answered: a web
//! page posting the same message gets nothing, and never learns why.
//!
//! Nothing here runs until one of them is opened, but for the settings,
//! which are read the first time something asks (the engine, Shield, sleep).

use std::cell::RefCell;

use searchwin_prefs::{Engine, Prefs};
use serde_json::{Value, json};

use crate::omnibox::INTERNAL_SCHEME;

/// History is long; a page of it is plenty to look through, and the search box finds the rest.
const HISTORY_ROWS: usize = 500;

const COMMON_CSS: &str = include_str!("../assets/pages/common.css");
const COMMON_JS: &str = include_str!("../assets/pages/common.js");
const SETTINGS: &str = include_str!("../assets/pages/settings.html");
const HISTORY: &str = include_str!("../assets/pages/history.html");
const BOOKMARKS: &str = include_str!("../assets/pages/bookmarks.html");
const DOWNLOADS: &str = include_str!("../assets/pages/downloads.html");
const PASSWORDS: &str = include_str!("../assets/pages/passwords.html");
const IMPORT: &str = include_str!("../assets/pages/import.html");

/// Our pages run only their own inline code, load nothing, and can't be put in another page's frame.
pub const HEADERS: &str = "Content-Type: text/html; charset=utf-8\r\n\
Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; frame-ancestors 'none'\r\n\
X-Frame-Options: DENY\r\n\
Cache-Control: no-store";

thread_local! {
    static PREFS: RefCell<Option<Prefs>> = const { RefCell::new(None) };
}

fn prefs_file() -> std::path::PathBuf {
    crate::data_folder().join("prefs.json")
}

/// The settings, read from the file the first time they are needed.
pub fn prefs<R>(f: impl FnOnce(&Prefs) -> R) -> R {
    PREFS.with(|cell| f(cell.borrow_mut().get_or_insert_with(|| Prefs::load(&prefs_file()))))
}

pub fn change(key: &str, value: Value) -> Result<(), String> {
    let changed = PREFS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let prefs = cell.get_or_insert_with(|| Prefs::load(&prefs_file()));
        prefs.set(key, value)?;
        Ok::<Prefs, String>(prefs.clone())
    })?;
    // A setting is written as it changes: there are few, and they change rarely.
    changed.save(&prefs_file()).map_err(|e| {
        crate::crash_log(&format!("settings not saved: {e}"));
        e.to_string()
    })
}

/// A setting changed, from Settings or from another PC, and what it changes
/// at once.
pub fn set_pref(key: &str, value: Value) -> Result<(), String> {
    change(key, value)?;
    // Split View takes effect at once: the splits there are come out, or go.
    if key == "split_view" {
        crate::split::turned(prefs(|p| p.split_view));
    }
    // Peek's listener goes into every page's next document, or out of it.
    if key == "peek" || key == "peek_with" {
        crate::page::refresh_peek();
    }
    crate::sync::poke();
    Ok(())
}

pub fn is_internal(url: &str) -> bool {
    url.get(..INTERNAL_SCHEME.len()).is_some_and(|scheme| scheme.eq_ignore_ascii_case(INTERNAL_SCHEME))
        && url[INTERNAL_SCHEME.len()..].starts_with(':')
}

/// The page at a search:// address, or None for one there isn't.
pub fn page(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok().filter(|u| u.scheme() == INTERNAL_SCHEME)?;
    if !matches!(url.path(), "" | "/") {
        return None;
    }
    let html = match url.host_str()? {
        "settings" => SETTINGS,
        "history" => HISTORY,
        "bookmarks" => BOOKMARKS,
        "downloads" => DOWNLOADS,
        "passwords" => PASSWORDS,
        "import" => IMPORT,
        _ => return None,
    };
    Some(html.replacen("<!--common-->", &format!("<style>{COMMON_CSS}</style><script>{COMMON_JS}</script>"), 1))
}

/// The host of one of our pages: "settings", "passwords".
fn page_name(source: &str) -> Option<String> {
    url::Url::parse(source).ok().filter(|u| u.scheme() == INTERNAL_SCHEME).and_then(|u| u.host_str().map(str::to_string))
}

/// A message posted by a page, from `source`, in tab `tab`. The answer goes
/// back to that page; None is no answer at all, which is all a web page
/// ever gets. Passwords are asked about only by search://passwords, and
/// bringing things over only by search://import: no other page of ours
/// has a reason to.
pub fn message(tab: crate::page::TabId, source: &str, json: &str) -> Option<String> {
    if !is_internal(source) {
        return None;
    }
    let asked: Value = serde_json::from_str(json).ok()?;
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let from = page_name(source).unwrap_or_default();
    let mut answer = match asked.get("op").and_then(Value::as_str)? {
        op if op.starts_with("passwords.") => match from.as_str() {
            "passwords" | "import" => crate::passwords::message(tab, &from, op, &asked),
            _ => return None,
        },
        op if op.starts_with("import.") => match from.as_str() {
            "import" => crate::import::message(tab, op, &asked),
            _ => return None,
        },
        "settings.get" => settings(None),
        "settings.set" => {
            let value = asked.get("value").cloned().unwrap_or(Value::Null);
            settings(set_pref(&text("key"), value).err())
        }
        op if op.starts_with("sync.") => match from.as_str() {
            "settings" => crate::sync::message(op),
            _ => return None,
        },
        op if op.starts_with("spaces.") || op.starts_with("profiles.") => crate::spaces::message(op, &asked),
        "history.list" => {
            let mut entries = crate::history_list(&text("query"));
            let more = entries.len() > HISTORY_ROWS;
            entries.truncate(HISTORY_ROWS);
            let entries: Vec<Value> = entries
                .iter()
                .map(|e| json!({ "url": e.url, "address": e.address, "title": e.title, "last": e.last }))
                .collect();
            json!({ "entries": entries, "more": more })
        }
        "history.remove" => json!({ "removed": crate::history_remove(&text("url")) }),
        "history.clear" => {
            crate::history_clear();
            json!({ "cleared": true })
        }
        op if op.starts_with("bookmarks.") => crate::bookmarks::message(op, &asked),
        op if op.starts_with("downloads.") => crate::downloads::message(op, &asked),
        _ => json!({ "error": "unknown op" }),
    };
    // Answered later (crate::page::post_to), once a check or a dialog is done.
    if answer.is_null() {
        return None;
    }
    answer["id"] = asked.get("id").cloned().unwrap_or(Value::Null);
    Some(answer.to_string())
}

/// Every setting, the engines to choose from, and whether a custom engine can search.
fn settings(error: Option<String>) -> Value {
    let (prefs, custom_ok) = prefs(|p| (p.to_value(), crate::engine::accepts(&crate::engine::written(&p.custom_engine))));
    let engines: Vec<Value> = Engine::ALL.iter().map(|e| json!({ "id": e.key(), "name": e.name() })).collect();
    let mut value = json!({ "prefs": prefs, "engines": engines, "custom_ok": custom_ok });
    if let Some(error) = error {
        value["error"] = json!(error);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_scheme_is_ours() {
        assert!(is_internal("search://settings"));
        assert!(is_internal("SEARCH://history"));
        assert!(!is_internal("https://search.example/"));
        assert!(!is_internal("searchx://settings"));
        assert!(!is_internal("search"));
    }

    #[test]
    fn pages_that_exist() {
        let settings = page("search://settings").unwrap();
        assert!(settings.contains("<style>") && settings.contains("window.chrome.webview"));
        assert!(!settings.contains("<!--common-->"));
        assert!(page("search://history/").is_some());
        assert!(page("search://bookmarks").is_some() && page("search://downloads/").is_some());
        assert!(page("search://nowhere").is_none());
        assert!(page("search://settings/../../etc").is_none());
        assert!(page("https://settings/").is_none());
    }

    #[test]
    fn web_pages_get_no_answer() {
        let asked = r#"{"op":"history.clear","id":1}"#;
        assert_eq!(message(1, "https://example.com/", asked), None);
        assert_eq!(message(1, "file:///C:/x.html", asked), None);
        assert_eq!(message(1, "about:blank", asked), None);
        for op in [
            "passwords.list",
            "passwords.reveal",
            "passwords.copy",
            "passwords.export",
            "passwords.import",
            "passwords.delete",
            "passwords.save",
            "import.list",
            "import.run",
            "downloads.list",
            "downloads.open",
            "downloads.show",
            "bookmarks.list",
            "bookmarks.import",
            "bookmarks.remove",
            "spaces.get",
            "spaces.delete",
            "profiles.delete",
            "settings.set",
            "sync.on",
            "sync.off",
            "sync.now",
        ] {
            let asked = json!({ "op": op, "id": 1, "row": 1, "node": "x", "from": "chrome", "host": "a.com", "user": "u" }).to_string();
            assert_eq!(message(1, "https://example.com/", &asked), None);
            assert_eq!(message(1, "searchx://passwords/", &asked), None);
        }
    }

    #[test]
    fn passwords_and_imports_answer_only_their_own_page() {
        for op in ["passwords.list", "passwords.reveal", "passwords.export", "import.list", "import.run"] {
            let asked = json!({ "op": op, "id": 1 }).to_string();
            assert_eq!(message(1, "search://settings", &asked), None, "{op}");
            assert_eq!(message(1, "search://history", &asked), None, "{op}");
        }
        assert_eq!(message(1, "search://passwords", r#"{"op":"import.run","id":1}"#), None);
        assert_eq!(message(1, "search://history", r#"{"op":"sync.off","id":1}"#), None);
        assert!(page("search://passwords").is_some() && page("search://import").is_some());
    }
}
