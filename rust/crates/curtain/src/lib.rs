//! searchwin-curtain: what you have hidden on a site, and the stylesheet that keeps it hidden.
//!
//! Ported from Search (Office Commun), MIT: Curtain.swift. Everything is kept by site
//! as a list of selectors (Curtain.swift:27), written to `hidden.json` in the same
//! shape upstream uses: `{ "host": [ { "selector", "label", "note", "date" } ] }`.
//! `date` is Swift's default `Date` encoding: seconds since 2001-01-01 (a float).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One hidden thing (Curtain.swift:11-23). `selector` is its identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Veil {
    pub selector: String,
    /// What it was, in words (Curtain.swift:13).
    #[serde(default)]
    pub label: String,
    /// Size and corner when hidden (Curtain.swift:19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Seconds since 2001-01-01 UTC, as Swift's JSONEncoder writes a Date.
    #[serde(default)]
    pub date: f64,
}

/// Seconds between the Unix epoch and Swift's reference date (2001-01-01).
const SWIFT_EPOCH: f64 = 978_307_200.0;

fn swift_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(SWIFT_EPOCH)
        - SWIFT_EPOCH
}

/// The key a site is kept under: lowercase, no `www.`, and only for a website
/// (Curtain.swift:32-35; SiteStyles.swift:48 says styles and Dusk use the same key).
pub fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u.host_str()?.to_lowercase();
    Some(h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
}

#[derive(Debug, Default)]
pub struct Curtain {
    by_host: BTreeMap<String, Vec<Veil>>,
    path: Option<PathBuf>,
}

impl Curtain {
    /// Loads the file; missing or corrupt means empty (Curtain.swift:87-92). The
    /// app passes where its data lives, so SEARCHWIN_DATA moves this file with the rest.
    pub fn load_from(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let by_host = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Curtain { by_host, path: Some(path) }
    }

    /// An empty store that never touches disk (`save` is a no-op).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The things hidden on a host (already normalised, see [`host_of`]).
    pub fn list(&self, host: &str) -> &[Veil] {
        self.by_host.get(host).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Hide a selector with no description. Returns false if already there or empty.
    pub fn add(&mut self, host: &str, selector: &str) -> bool {
        self.add_with(host, selector, selector, None)
    }

    /// Hide with the label and note the picker measured (Curtain.swift:42-48).
    /// A duplicate selector is ignored.
    pub fn add_with(&mut self, host: &str, selector: &str, label: &str, note: Option<&str>) -> bool {
        if host.is_empty() || selector.trim().is_empty() {
            return false;
        }
        let list = self.by_host.entry(host.to_string()).or_default();
        if list.iter().any(|v| v.selector == selector) {
            return false;
        }
        list.push(Veil {
            selector: selector.to_string(),
            label: label.to_string(),
            note: note.map(str::to_string),
            date: swift_now(),
        });
        true
    }

    /// Put one back (Curtain.swift:51-55). An emptied host is dropped.
    pub fn remove(&mut self, host: &str, selector: &str) -> bool {
        let Some(list) = self.by_host.get_mut(host) else { return false };
        let before = list.len();
        list.retain(|v| v.selector != selector);
        let changed = list.len() != before;
        if list.is_empty() {
            self.by_host.remove(host);
        }
        changed
    }

    /// Put the last one back (Curtain.swift:59-64).
    pub fn undo(&mut self, host: &str) -> Option<Veil> {
        let list = self.by_host.get_mut(host)?;
        let last = list.pop();
        if list.is_empty() {
            self.by_host.remove(host);
        }
        last
    }

    /// Put everything back on a site (Curtain.swift:66-69).
    pub fn clear(&mut self, host: &str) {
        self.by_host.remove(host);
    }

    /// The stylesheet for a site (Curtain.swift:76-81): one rule per selector, so one the
    /// browser cannot parse does not take the rest down.
    pub fn css(&self, host: &str) -> String {
        self.css_without(host, None)
    }

    /// As [`css`](Self::css) with one selector spared: how the list previews a restore.
    pub fn css_without(&self, host: &str, spared: Option<&str>) -> String {
        self.list(host)
            .iter()
            .filter(|v| Some(v.selector.as_str()) != spared)
            .map(|v| format!("{} {{ display: none !important; }}", v.selector))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Writes the file atomically (temp file, then rename). No-op for `in_memory`.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        write_atomic(path, &serde_json::to_vec(&self.by_host).map_err(std::io::Error::other)?)
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A document-start script that puts `css` in `<style id="office-veil">` (Curtain.swift:143-158),
/// the id the picker's peek/unpeek also uses. The CSS travels as a JSON string literal
/// (with U+2028/9 and `<` escaped, as SiteStyles.swift:88-97 does), never as raw source.
pub fn style_script(css: &str) -> String {
    let lit = serde_json::to_string(css)
        .unwrap_or_else(|_| "\"\"".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .replace('<', "\\u003c");
    format!(
        "(function(){{var s=document.getElementById('office-veil');if(!s){{s=document.createElement('style');\
         s.id='office-veil';(document.head||document.documentElement).appendChild(s);}}s.textContent={lit};}})();"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swcurtain-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("hidden.json")
    }

    #[test]
    fn host_drops_www_and_lowercases() {
        // Curtain.swift:32-35
        assert_eq!(host_of("https://WWW.Example.com/a?b").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://news.example.com").as_deref(), Some("news.example.com"));
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("file:///c:/x.html"), None);
    }

    #[test]
    fn add_ignores_duplicates_and_keeps_order() {
        // Curtain.swift:43-44
        let mut c = Curtain::in_memory();
        assert!(c.add_with("a.com", "#x", "Banner", Some("10x10")));
        assert!(!c.add("a.com", "#x"));
        assert!(c.add("a.com", ".y"));
        assert!(!c.add("a.com", "  "));
        let s: Vec<_> = c.list("a.com").iter().map(|v| v.selector.as_str()).collect();
        assert_eq!(s, ["#x", ".y"]);
        assert!(c.list("b.com").is_empty());
    }

    #[test]
    fn remove_undo_clear() {
        // Curtain.swift:51-69: an emptied host disappears
        let mut c = Curtain::in_memory();
        c.add("a.com", "#x");
        c.add("a.com", "#y");
        assert!(c.remove("a.com", "#x"));
        assert!(!c.remove("a.com", "#x"));
        assert_eq!(c.undo("a.com").unwrap().selector, "#y");
        assert!(c.undo("a.com").is_none());
        assert!(c.by_host.is_empty());
        c.add("a.com", "#z");
        c.clear("a.com");
        assert!(c.by_host.is_empty());
    }

    #[test]
    fn css_one_rule_per_selector() {
        // Curtain.swift:76-81
        let mut c = Curtain::in_memory();
        c.add("a.com", "#x");
        c.add("a.com", "div.y");
        assert_eq!(
            c.css("a.com"),
            "#x { display: none !important; }\ndiv.y { display: none !important; }"
        );
        assert_eq!(c.css_without("a.com", Some("#x")), "div.y { display: none !important; }");
        assert_eq!(c.css("none.com"), "");
    }

    #[test]
    fn save_and_load_round_trip() {
        let p = tmp("rt");
        let mut c = Curtain::load_from(&p);
        c.add_with("a.com", "#x", "Banner", Some("1x1 · top left"));
        c.save().unwrap();
        assert!(!p.with_extension("json.tmp").exists());
        let d = Curtain::load_from(&p);
        assert_eq!(d.list("a.com"), c.list("a.com"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn reads_upstream_file_shape() {
        // Curtain.swift:11-20 with Swift's default Date (seconds since 2001)
        let p = tmp("up");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(
            &p,
            r##"{"a.com":[{"selector":"#x","label":"Cookie bar","note":"800x90 bottom centre","date":700000000.5}]}"##,
        )
        .unwrap();
        let c = Curtain::load_from(&p);
        assert_eq!(c.list("a.com")[0].label, "Cookie bar");
        assert_eq!(c.list("a.com")[0].date, 700000000.5);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn missing_or_corrupt_is_empty() {
        // Curtain.swift:88-91
        let p = tmp("bad");
        assert!(Curtain::load_from(&p).list("a.com").is_empty());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "not json{").unwrap();
        assert!(Curtain::load_from(&p).by_host.is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn style_script_cannot_be_broken_out_of() {
        let s = style_script("a{content:\"</style>\u{2028}\"} `${x}`");
        assert!(!s.contains("</style>"));
        assert!(!s.contains('\u{2028}'));
        assert!(s.contains("office-veil"));
    }
}
