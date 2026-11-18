//! Where you have been, so the field can finish the address for you, and the
//! rows it offers as you type. Port of upstream Sources/Search/History.swift,
//! with Fuzzy.swift in `fuzzy`. Pure logic: no window, no WebView2.
//!
//! Private tabs never record. That is the caller's job: it simply never calls
//! `record_visit` or `set_title` for a private tab (upstream's History is
//! likewise only told about normal tabs).
//!
//! Time is passed in as milliseconds since the Unix epoch, so every rule is
//! testable; `suggest`/`complete` read the clock for you.

pub mod fuzzy;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use url::{Host, Url};

/// Places kept in the file, and in memory once a session has run long enough
/// to be cut (History.swift `cap`).
pub const CAP: usize = 2_000;
/// How far memory may run past `CAP` before it is cut back to it, and how many
/// of the newest places the cut keeps whatever they score (`slack`).
pub const SLACK: usize = 250;
/// Visits counted at most: far more than anyone makes, and room to add.
const MOST_VISITS: u32 = 1_000_000_000;
/// Frecency's month, in milliseconds (`fading`).
const FADING_MS: f64 = 30.0 * 86_400_000.0;

/// Somewhere to start on the first day, before there is any history to go on.
/// Ranked below anything actually visited, and dropped from the list the
/// moment you have been there yourself (History.swift `known`).
const KNOWN: [(&str, &str); 47] = [
    ("google.com", "Google"), ("mail.google.com", "Gmail"),
    ("drive.google.com", "Google Drive"), ("calendar.google.com", "Google Calendar"),
    ("maps.google.com", "Google Maps"), ("youtube.com", "YouTube"),
    ("github.com", "GitHub"), ("figma.com", "Figma"), ("vercel.com", "Vercel"),
    ("notion.so", "Notion"), ("linear.app", "Linear"), ("slack.com", "Slack"),
    ("discord.com", "Discord"), ("x.com", "X"), ("linkedin.com", "LinkedIn"),
    ("instagram.com", "Instagram"), ("reddit.com", "Reddit"),
    ("news.ycombinator.com", "Hacker News"), ("stackoverflow.com", "Stack Overflow"),
    ("claude.ai", "Claude"), ("chatgpt.com", "ChatGPT"),
    ("dribbble.com", "Dribbble"), ("behance.net", "Behance"),
    ("awwwards.com", "Awwwards"), ("mobbin.com", "Mobbin"),
    ("siteinspire.com", "SiteInspire"), ("are.na", "Are.na"),
    ("pinterest.com", "Pinterest"), ("framer.com", "Framer"),
    ("webflow.com", "Webflow"), ("developer.apple.com", "Apple Developer"),
    ("swift.org", "Swift"), ("npmjs.com", "npm"), ("supabase.com", "Supabase"),
    ("stripe.com", "Stripe"), ("shopify.com", "Shopify"),
    ("cloudflare.com", "Cloudflare"), ("netlify.com", "Netlify"),
    ("apple.com", "Apple"), ("spotify.com", "Spotify"), ("netflix.com", "Netflix"),
    ("wikipedia.org", "Wikipedia"), ("deepl.com", "DeepL"), ("loom.com", "Loom"),
    ("amazon.fr", "Amazon"), ("leboncoin.fr", "leboncoin"), ("lemonde.fr", "Le Monde"),
];

/// The same, prepared once for the looser search (History.swift `knownText`).
fn known_findable() -> &'static [String] {
    static TEXT: OnceLock<Vec<String>> = OnceLock::new();
    TEXT.get_or_init(|| KNOWN.iter().map(|(host, title)| format!("{} {host}", title.to_lowercase())).collect())
}

/// A visit as the history file keeps it: a JSON array of these. Like
/// upstream's file but `last` is Unix milliseconds, and the key is derived on
/// load rather than stored.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub last: u64,
}

/// A place with what suggestions need, worked out once when it enters memory:
/// suggestions run on every keystroke, so neither URL parsing nor address
/// formatting belongs in that loop.
struct Page {
    entry: Entry,
    /// What you could type to get here, as the row shows it.
    address: String,
    /// `address` lowercased, without scheme or www.
    searchable: String,
    /// Lowercased title, a space, then `searchable`.
    findable: String,
    /// Host without www.
    host: String,
    homepage: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Somewhere you have actually been.
    Visited,
    /// One of the well-known addresses the field knows from the start.
    Known,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// What you could type to get here, kept reversible for completion.
    pub key: String,
    pub title: String,
    pub url: String,
    pub kind: Kind,
}

/// One line of the History panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub url: String,
    pub address: String,
    pub title: String,
    pub last: u64,
    pub count: u32,
}

#[derive(Default)]
pub struct History {
    pages: Vec<Page>,
    index: HashMap<String, usize>,
    /// Pages per host, so a known site is dropped once you have been there.
    hosts: HashMap<String, u32>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A page as it is kept: scheme http(s) only, without a name and password
/// written into its address (the history file is no place for a password) and
/// without its fragment (URL identity ignores it, History.swift `identity`).
/// The `url` crate already lowercases scheme and host and gives an empty path
/// its slash, and keeps path and query as written.
fn clean(text: &str) -> Option<Url> {
    let mut url = Url::parse(text.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Some(url)
}

fn bare(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// A server on this machine or network almost never has a certificate, so the
/// field gives it http (Address.swift); such an address may drop `http://`.
fn local(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local"),
        Some(Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_unspecified(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// An address as the field edits it: without the scheme the field would put
/// back itself, and without the lone slash of a front door
/// (Address.swift `editable`).
fn editable(url: &Url) -> String {
    let full = url.as_str();
    if url.scheme() == "https" || local(url) {
        let short = &full[url.scheme().len() + 3..];
        if url.path() == "/" && url.query().is_none() {
            return short.strip_suffix('/').unwrap_or(short).to_string();
        }
        return short.to_string();
    }
    full.to_string()
}

/// Lowercased, without scheme or www (History.swift `stripped`).
fn stripped(address: &str) -> String {
    let text = address.trim().to_lowercase();
    let text = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")).unwrap_or(&text);
    bare(text).to_string()
}

impl Page {
    fn new(entry: Entry, url: &Url) -> Page {
        let address = editable(url);
        let searchable = stripped(&address);
        let host = bare(url.host_str().unwrap_or("")).to_string();
        let findable = format!("{} {}", entry.title.to_lowercase(), searchable);
        Page { homepage: url.path() == "/", entry, address, searchable, findable, host }
    }

    fn retitle(&mut self, title: &str) {
        self.entry.title = title.to_string();
        self.findable = format!("{} {}", title.to_lowercase(), self.searchable);
    }

    /// Often, and lately. A month-old visit counts for about a third of a
    /// fresh one (History.swift `score`).
    fn score(&self, now: u64) -> f64 {
        let age_ms = now.saturating_sub(self.entry.last) as f64;
        f64::from(self.entry.count) * (-age_ms / FADING_MS).exp()
    }

    /// The front-door credits have no title of their own, and in a list of
    /// where you have been they are a second copy of every line.
    fn hidden(&self) -> bool {
        self.entry.title.is_empty() && self.homepage
    }
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    /// Reads the file. A missing or corrupt file is an empty history: never a
    /// panic, never an error to show.
    pub fn load(path: &Path) -> History {
        let entries = std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<Vec<Entry>>(&data).ok())
            .unwrap_or_default();
        History::from_entries(entries)
    }

    /// Entries from a file, or from another browser (counted as it was
    /// counted there). Keys written by an older version can meet under the new
    /// rule: they are merged, never trusted to be unique.
    pub fn from_entries(entries: Vec<Entry>) -> History {
        let mut history = History::new();
        for mut entry in entries {
            let Some(url) = clean(&entry.url) else { continue };
            entry.url = url.as_str().to_string();
            // A count from a file edited by hand is held to a sane range, so
            // adding to it can't overflow.
            entry.count = entry.count.min(MOST_VISITS);
            match history.index.get(&entry.url) {
                Some(&at) => history.merge(at, entry),
                None => history.insert(Page::new(entry, &url)),
            }
        }
        history
    }

    /// The same page twice: its visits added up, the latest date, and a title
    /// if either had one (History.swift `merged`).
    fn merge(&mut self, at: usize, other: Entry) {
        let page = &mut self.pages[at];
        page.entry.count = page.entry.count.saturating_add(other.count).min(MOST_VISITS);
        page.entry.last = page.entry.last.max(other.last);
        if page.entry.title.is_empty() && !other.title.is_empty() {
            page.retitle(&other.title);
        }
    }

    fn insert(&mut self, page: Page) {
        *self.hosts.entry(page.host.clone()).or_insert(0) += 1;
        self.index.insert(page.entry.url.clone(), self.pages.len());
        self.pages.push(page);
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Writes the file atomically: a temp file beside it, then a rename, so a
    /// crash mid-write leaves the old history. Only the best `CAP` places go
    /// in, by score: what goes is what has been visited least and longest ago.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let now = now_ms();
        let mut best: Vec<&Page> = self.pages.iter().collect();
        best.sort_by(|a, b| b.score(now).total_cmp(&a.score(now)));
        best.truncate(CAP);
        let list: Vec<&Entry> = best.into_iter().map(|p| &p.entry).collect();
        let data = serde_json::to_vec(&list).map_err(std::io::Error::other)?;
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

    // MARK: writing

    /// A visit. Only http(s) pages are kept; returns whether this one was.
    pub fn record_visit(&mut self, url: &str, title: &str, at_ms: u64) -> bool {
        let Some(url) = clean(url) else { return false };
        // Reading a deep page is also, in the way that matters here, another
        // visit to the site. Without this, typing three letters offers the
        // article you happened to open last week rather than the front page.
        if url.path() != "/" {
            let mut home = url.clone();
            home.set_path("/");
            home.set_query(None);
            self.visit(&home, "", at_ms);
        }
        self.visit(&url, title, at_ms);
        self.prune(at_ms);
        true
    }

    fn visit(&mut self, url: &Url, title: &str, at_ms: u64) {
        match self.index.get(url.as_str()).copied() {
            Some(at) => {
                let page = &mut self.pages[at];
                page.entry.count = (page.entry.count + 1).min(MOST_VISITS);
                page.entry.last = at_ms;
                if !title.is_empty() && page.entry.title != title {
                    page.retitle(title);
                }
            }
            None => {
                let entry = Entry { url: url.as_str().to_string(), title: title.to_string(), count: 1, last: at_ms };
                self.insert(Page::new(entry, url));
            }
        }
    }

    fn find(&self, url: &str) -> Option<usize> {
        clean(url).and_then(|u| self.index.get(u.as_str()).copied())
    }

    /// A page's title usually lands a beat after the page does. Returns
    /// whether anything changed (so the caller knows the file is owed).
    pub fn set_title(&mut self, url: &str, title: &str) -> bool {
        let Some(at) = self.find(url) else { return false };
        if title.is_empty() || self.pages[at].entry.title == title {
            return false;
        }
        self.pages[at].retitle(title);
        true
    }

    /// Forgets one place. Returns whether it was there.
    pub fn remove(&mut self, url: &str) -> bool {
        let Some(at) = self.find(url) else { return false };
        let gone = self.pages.swap_remove(at);
        self.index.remove(&gone.entry.url);
        if let Some(moved) = self.pages.get(at) {
            self.index.insert(moved.entry.url.clone(), at);
        }
        if let Some(n) = self.hosts.get_mut(&gone.host) {
            *n -= 1;
            if *n == 0 {
                self.hosts.remove(&gone.host);
            }
        }
        true
    }

    pub fn clear(&mut self) {
        *self = History::new();
    }

    /// Memory is cut back to `CAP` places once it is `SLACK` past it: otherwise
    /// a session that never restarts holds, and the field scans on every key,
    /// every page it has ever seen. The newest `SLACK` places stay whatever
    /// they score; the rest of the cap goes to the highest score
    /// (History.swift `prune`).
    fn prune(&mut self, now: u64) {
        if self.pages.len() <= CAP + SLACK {
            return;
        }
        let mut by_time: Vec<usize> = (0..self.pages.len()).collect();
        by_time.sort_by(|&a, &b| self.pages[b].entry.last.cmp(&self.pages[a].entry.last));
        let mut keep = vec![false; self.pages.len()];
        for &i in &by_time[..SLACK] {
            keep[i] = true;
        }
        let mut rest = by_time.split_off(SLACK);
        rest.sort_by(|&a, &b| self.pages[b].score(now).total_cmp(&self.pages[a].score(now)));
        for &i in &rest[..CAP - SLACK] {
            keep[i] = true;
        }
        let old = std::mem::take(&mut self.pages);
        self.index.clear();
        self.hosts.clear();
        for (page, kept) in old.into_iter().zip(keep) {
            if kept {
                self.insert(page);
            }
        }
    }

    // MARK: panel

    /// Everywhere you have been, newest first, for the window that shows it.
    /// `matching` finds a word in the address or the title.
    pub fn entries(&self, matching: &str) -> Vec<Trace> {
        let needle = matching.trim().to_lowercase();
        let mut found: Vec<&Page> = self
            .pages
            .iter()
            .filter(|p| !p.hidden())
            .filter(|p| needle.is_empty() || p.findable.contains(&needle))
            .collect();
        found.sort_by_key(|p| std::cmp::Reverse(p.entry.last));
        found
            .into_iter()
            .map(|p| Trace {
                url: p.entry.url.clone(),
                address: p.address.clone(),
                title: p.entry.title.clone(),
                last: p.entry.last,
                count: p.entry.count,
            })
            .collect()
    }

    // MARK: reading

    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        self.suggest_at(query, limit, now_ms())
    }

    /// Best matches first: `score = fit x ln(2 + popularity)`
    /// (History.swift `suggestions`).
    pub fn suggest_at(&self, query: &str, limit: usize, now: u64) -> Vec<Suggestion> {
        let needle = stripped(query);
        // An empty field proposes nothing: a list of guesses in front of
        // someone who has not yet said what they want is in the way.
        if needle.is_empty() || limit == 0 {
            return Vec::new();
        }
        let words: Vec<&str> = needle.split_whitespace().collect();
        let known_weight = 1.5f64.ln();
        let known = known_findable();
        // Only the best few are kept while scanning, as indexes; only those
        // become suggestions.
        let mut best = Top { limit, picks: Vec::with_capacity(limit + 1) };

        for (i, page) in self.pages.iter().enumerate() {
            if holds(&words, &page.findable)
                && let Some(fit) = fit(&needle, &words, &page.searchable, &page.findable)
            {
                best.offer(fit * (2.0 + page.score(now)).ln(), page.address.len(), Source::Visit(i));
            }
        }
        for (i, (host, _)) in KNOWN.iter().enumerate() {
            if !self.hosts.contains_key(*host)
                && holds(&words, &known[i])
                && let Some(fit) = fit(&needle, &words, host, &known[i])
            {
                best.offer(fit * known_weight, host.len(), Source::Known(i));
            }
        }

        // Room left: a typo (#762), fit 1. Slowest, so only then, from three
        // letters.
        if best.picks.len() < limit && needle.len() >= 3 {
            for (i, page) in self.pages.iter().enumerate() {
                if fuzzy::matches(&words, &page.findable) == Some(0) {
                    best.offer((2.0 + page.score(now)).ln(), page.address.len(), Source::Visit(i));
                }
            }
            for (i, (host, _)) in KNOWN.iter().enumerate() {
                if !self.hosts.contains_key(*host) && fuzzy::matches(&words, &known[i]) == Some(0) {
                    best.offer(known_weight, host.len(), Source::Known(i));
                }
            }
        }

        best.picks.into_iter()
            .map(|pick| match pick.source {
                Source::Visit(i) => {
                    let p = &self.pages[i];
                    Suggestion { key: p.address.clone(), title: p.entry.title.clone(), url: p.entry.url.clone(), kind: Kind::Visited }
                }
                Source::Known(i) => {
                    let (host, title) = KNOWN[i];
                    Suggestion { key: host.to_string(), title: title.to_string(), url: format!("https://{host}/"), kind: Kind::Known }
                }
            })
            .collect()
    }

    pub fn complete(&self, typed: &str) -> Option<String> {
        self.complete_at(typed, now_ms())
    }

    /// What the field should draw greyed out after the caret: the rest of the
    /// best match, or nothing if it doesn't carry on from what was typed
    /// (History.swift `completion`).
    pub fn complete_at(&self, typed: &str, now: u64) -> Option<String> {
        if typed.is_empty() {
            return None;
        }
        let options = self.suggest_at(typed, 5, now);
        // One letter finishes only the top row, and only when it is the front
        // door of a site you have been to: "x" becomes x.com, as in Safari and
        // Chrome (#696). A room inside a site, a name the app merely knows, or
        // a lower row waits for a second letter.
        let single = typed.chars().count() == 1;
        if single && options.first()?.kind != Kind::Visited {
            return None;
        }
        for option in options.iter().take(if single { 1 } else { options.len() }) {
            for key in endings(option) {
                if key.len() < typed.len() || !key.is_char_boundary(typed.len()) {
                    continue;
                }
                if single && key.contains(['/', '?', '#']) {
                    continue;
                }
                let (head, rest) = key.split_at(typed.len());
                if !head.eq_ignore_ascii_case(typed) {
                    continue;
                }
                // Only the host is case-blind: a path typed in another case is
                // another page (upstream checks the identity of the result).
                let path_at = head.find(['/', '?', '#']).unwrap_or(head.len());
                if head[path_at..] != typed[path_at..] {
                    continue;
                }
                return if rest.is_empty() { None } else { Some(rest.to_string()) };
            }
        }
        None
    }
}


/// What a row can be finished as. Its address as shown; and for a www site,
/// the name without the www, as the field always finished it: "app" still
/// becomes apple.com, and the site sends you on to its www.
fn endings(option: &Suggestion) -> Vec<&str> {
    let mut found = vec![option.key.as_str()];
    if option.key.len() > 4 && option.key[..4].eq_ignore_ascii_case("www.") {
        found.push(&option.key[4..]);
    }
    found
}

enum Source {
    Visit(usize),
    Known(usize),
}

/// The best few matches so far, in order.
struct Top {
    limit: usize,
    picks: Vec<Pick>,
}

impl Top {
    fn offer(&mut self, score: f64, key_len: usize, source: Source) {
        let pick = Pick { score, key_len, source };
        if self.picks.len() == self.limit && self.picks.last().is_some_and(|last| !pick.ahead_of(last)) {
            return;
        }
        let at = self.picks.iter().position(|b| pick.ahead_of(b)).unwrap_or(self.picks.len());
        self.picks.insert(at, pick);
        self.picks.truncate(self.limit);
    }
}

struct Pick {
    score: f64,
    key_len: usize,
    source: Source,
}

impl Pick {
    /// Higher first, then the shorter key, then whichever came first: the
    /// order a stable sort of every match would have left them in.
    fn ahead_of(&self, other: &Pick) -> bool {
        if self.score == other.score { self.key_len < other.key_len } else { self.score > other.score }
    }
}

/// Every word somewhere in the page.
fn holds(words: &[&str], findable: &str) -> bool {
    words.iter().all(|w| fuzzy::contains(findable, w))
}

/// 4 address starts with it, 3 a word of the host does, 2 every word starts a
/// word of title or address, 1 found inside a word. Title and path from three
/// letters: "ui" is in nearly every page (History.swift `fit`).
fn fit(needle: &str, words: &[&str], address: &str, findable: &str) -> Option<f64> {
    if address.starts_with(needle) {
        return Some(4.0);
    }
    let end = address.find(['/', '?', '#']).unwrap_or(address.len());
    if fuzzy::starts_word(needle, &address[..end]) {
        return Some(3.0);
    }
    if needle.len() < 3 {
        return None;
    }
    Some(if words.iter().all(|w| fuzzy::starts_word(w, findable)) { 2.0 } else { 1.0 })
}
