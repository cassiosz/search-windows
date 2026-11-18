//! Hiding things on a site (Curtain) and reading mode, in the window. The
//! store and its file are crates/curtain; what runs in the page is
//! assets/js. Nothing here runs until Ctrl+Shift+H, U or R is pressed, but for
//! the list of what is hidden, read when the first page is made.
//!
//! The scripts post back through the same channel any web page can post on,
//! so what they say is never taken on trust: a page is heard only once one of
//! them is in it (page.rs checks that), only for what that script says, its
//! site is worked out again from the page's own address, and a selector that
//! could reach outside its one rule is refused. A page forging a pick can at
//! worst hide something on itself, and only while you are pointing.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use searchwin_curtain::{Curtain, host_of};
use serde_json::{Value, json};

use crate::page::{self, TabId};

const PICKER: &str = include_str!("../assets/js/picker.js");
const HIDDEN: &str = include_str!("../assets/js/hidden.js");
const READER: &str = include_str!("../assets/js/reader.js");

/// What a document was given, as page.rs keeps it: picker.js, hidden.js, reader.js.
pub const PICKER_IN: u8 = 1;
pub const LIST_IN: u8 = 2;
pub const READER_IN: u8 = 4;

/// The longest selector kept; the picker's are far shorter.
const SELECTOR_MAX: usize = 500;
const LABEL_MAX: usize = 200;
const NOTE_MAX: usize = 100;

/// The store, and the sites it has something for: the crate answers by site
/// only, and the document-start script needs them all.
struct Store {
    curtain: Curtain,
    hosts: BTreeSet<String>,
}

thread_local! {
    static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
    /// What a private tab took off its page: shown gone there and written
    /// nowhere (upstream's passingVeils). Forgotten when the tab closes.
    static PASSING: RefCell<HashMap<TabId, Vec<String>>> = RefCell::new(HashMap::new());
}

fn store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|cell| {
        let mut cell = cell.borrow_mut();
        f(cell.get_or_insert_with(|| {
            let path = crate::data_folder().join("hidden.json");
            let hosts = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|sites| Some(sites.as_object()?.keys().cloned().collect()))
                .unwrap_or_default();
            Store { curtain: Curtain::load_from(path), hosts }
        }))
    })
}

fn with<R>(f: impl FnOnce(&mut Curtain) -> R) -> R {
    store(|s| f(&mut s.curtain))
}

fn rule(selector: &str) -> String {
    format!("{selector} {{ display: none !important; }}")
}

/// The script every document starts with: what you hid, on whichever site the
/// document is, in a stylesheet of its own, one rule at a time so a rule the
/// engine refuses takes no other with it. picker.js finds that sheet again as
/// window.__officeVeilSheet to change it live. Every site is in it, so it never
/// has to change as a page moves to another site (set then, it would be late),
/// only when the list does. Only the top document of a website: frames of other
/// sites inside it, and our own pages, are left alone. Empty with nothing hidden.
pub fn start_script() -> String {
    // Written out by hand: the sites are already in order, and a JSON map would sort them again.
    let sites: Vec<String> = store(|s| {
        s.hosts
            .iter()
            .filter_map(|host| {
                let rules: Vec<String> = s.curtain.list(host).iter().map(|v| rule(&v.selector)).collect();
                (!rules.is_empty()).then(|| format!("{}:{}", json!(host), json!(rules)))
            })
            .collect()
    });
    if sites.is_empty() {
        return String::new();
    }
    format!(
        "(()=>{{const m={{{}}};const h=location.hostname.toLowerCase().replace(/^www\\./,'');\
         if(window!==window.top||!/^https?:$/.test(location.protocol)||!Object.hasOwn(m,h))return;\
         const v=new CSSStyleSheet();for(const r of m[h])try{{v.insertRule(r,v.cssRules.length)}}catch(e){{}}\
         document.adoptedStyleSheets=[...document.adoptedStyleSheets,v];window.__officeVeilSheet=v}})()",
        sites.join(",")
    )
}

/// What is hidden on `host` in tab `id` now: what is kept, and what a private tab took off.
fn live_css(id: TabId, host: &str) -> String {
    let mut css = with(|c| c.css(host));
    PASSING.with(|p| {
        for selector in p.borrow().get(&id).into_iter().flatten() {
            if !css.is_empty() {
                css.push('\n');
            }
            css += &rule(selector);
        }
    });
    css
}

/// Puts the page in front of you in step with the list at once, and the next
/// page of every tab too.
fn redress(id: TabId, host: &str) {
    let css = live_css(id, host);
    page::inject(id, 0, &format!("window.__officeVeil&&window.__officeVeil.css({})", json!(css)), |_| {});
    page::refresh_veils();
}

/// A selector that stays inside its one rule: no block, no end of rule, no
/// comment, no markup, no line break.
fn acceptable(selector: &str) -> bool {
    !selector.trim().is_empty()
        && selector.len() <= SELECTOR_MAX
        && !selector.contains(['{', '}', ';', '<'])
        && !selector.contains("/*")
        && !selector.chars().any(char::is_control)
}

/// Words from a page, kept short and on one line.
fn plain(text: &str, most: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(most).collect()
}

/// The front tab's id and its website's host, if it is on one.
fn front_site() -> Option<(TabId, String)> {
    let (id, url) = crate::front_tab()?;
    Some((id, host_of(&url?)?))
}

/// Ctrl+Shift+H: point at anything on the page and it goes, for good, on
/// this site. Again, or Esc, stops pointing.
pub fn toggle_picker() {
    let Some((id, _)) = front_site() else { return };
    if page::picking(id) {
        stop_picker(id);
    } else {
        start_picker(id);
    }
}

fn start_picker(id: TabId) {
    page::set_picking(id, true);
    page::inject(id, PICKER_IN, &format!("{PICKER}\n;window.__officeVeil.on()"), |_| {});
}

fn stop_picker(id: TabId) {
    page::set_picking(id, false);
    page::inject(id, 0, "window.__officeVeil&&window.__officeVeil.off()", |_| {});
}

/// Esc while pointing in the tab in front: pointing stops. False when it wasn't.
pub fn escape() -> bool {
    let Some((id, _)) = crate::front_tab() else { return false };
    if !page::picking(id) {
        return false;
    }
    stop_picker(id);
    true
}

/// Ctrl+Shift+U: what is hidden on this site, each one able to come back.
pub fn toggle_list() {
    let Some((id, host)) = front_site() else { return };
    let mut veils: Vec<Value> =
        with(|c| c.list(&host).iter().map(|v| json!({ "selector": v.selector, "label": v.label, "note": v.note })).collect());
    PASSING.with(|p| {
        for selector in p.borrow().get(&id).into_iter().flatten() {
            veils.push(json!({ "selector": selector, "label": selector }));
        }
    });
    // The list peeks through picker.js, so it goes in too, asleep.
    let js = format!("{PICKER}\n{HIDDEN}\n;window.__officeHidden.toggle({})", Value::Array(veils));
    page::inject(id, PICKER_IN | LIST_IN, &js, |_| {});
}

/// Ctrl+Shift+R: the article and nothing around it. Again, the page as it was,
/// by loading it again. On a page that isn't a website, it is the reload it
/// is in every other browser.
pub fn toggle_reader() {
    let Some((id, _)) = front_site() else {
        page::reload();
        return;
    };
    if page::injected(id) & READER_IN != 0 {
        page::reload();
        return;
    }
    let look = crate::internal::prefs(|p| p.reader_look.key());
    let js = format!("window.__officeReaderLook={};\n{READER}", json!(look));
    page::inject(id, READER_IN, &js, |_| {});
}

/// A message from a web page in tab `id` (page.rs has checked it holds one of
/// our scripts, `injected`, and that `source` is where it is now).
pub fn message(id: TabId, source: &str, json: &str, injected: u8) {
    let Some(host) = host_of(source) else { return };
    let Ok(asked) = serde_json::from_str::<Value>(json) else { return };
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default();
    let given = |script: u8| injected & script != 0;
    match text("op") {
        // The host the message names is never used: the page's own address says.
        "curtain.pick" if given(PICKER_IN) && page::picking(id) => {
            picked(id, &host, text("selector"), &plain(text("label"), LABEL_MAX), &plain(text("note"), NOTE_MAX))
        }
        "curtain.off" if given(PICKER_IN) => page::set_picking(id, false),
        "curtain.restore" if given(LIST_IN) => restore(id, &host, Some(text("selector"))),
        "curtain.restoreAll" if given(LIST_IN) => restore(id, &host, None),
        "curtain.hideSomething" if given(LIST_IN) && crate::front_tab().is_some_and(|(front, _)| front == id) => {
            start_picker(id)
        }
        "reader.look" if given(READER_IN) => {
            // Kept with the settings, so the next article opens in the look chosen last.
            // A word that isn't a look is refused by prefs and changes nothing.
            let _ = crate::internal::change("reader_look", json!(text("look")));
        }
        // curtain.trouble, reader.trouble, and anything else: nothing to do.
        _ => {}
    }
}

fn picked(id: TabId, host: &str, selector: &str, label: &str, note: &str) {
    if !acceptable(selector) {
        return;
    }
    // A private tab writes nothing down, hidden.json included: off this page only.
    if crate::is_private(id) {
        PASSING.with(|p| {
            let mut passing = p.borrow_mut();
            let list = passing.entry(id).or_default();
            if !list.iter().any(|s| s == selector) {
                list.push(selector.to_string());
            }
        });
    } else {
        let added = store(|s| {
            s.hosts.insert(host.to_string());
            let note = (!note.is_empty()).then_some(note);
            s.curtain.add_with(host, selector, label, note).then(|| s.curtain.save())
        });
        if let Some(Err(error)) = added {
            crate::crash_log(&format!("hidden things not saved: {error}"));
        }
    }
    redress(id, host);
}

/// One thing back, or everything (`selector` None), on this site. The next
/// page of the site starts without it too.
fn restore(id: TabId, host: &str, selector: Option<&str>) {
    let was_passing = PASSING.with(|p| {
        let mut passing = p.borrow_mut();
        let list = passing.entry(id).or_default();
        let before = list.len();
        match selector {
            Some(selector) => list.retain(|s| s != selector),
            None => list.clear(),
        }
        before != list.len() && selector.is_some()
    });
    if !was_passing {
        let saved = with(|c| {
            let changed = match selector {
                Some(selector) => c.remove(host, selector),
                None => {
                    let any = !c.list(host).is_empty();
                    c.clear(host);
                    any
                }
            };
            changed.then(|| c.save())
        });
        if let Some(Err(error)) = saved {
            crate::crash_log(&format!("hidden things not saved: {error}"));
        }
    }
    redress(id, host);
}

/// A tab closed: what it took off its pages goes with it.
pub fn forget(id: TabId) {
    PASSING.with(|p| p.borrow_mut().remove(&id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_stay_inside_their_rule() {
        assert!(acceptable("#ad"));
        assert!(acceptable(r#"div[data-testid="promo \"x\""] > p:nth-of-type(2)"#));
        for bad in ["", "  ", "a{x}", "a}", "a;b", "a/*", "<style>", "a\nb", "a\rb", "a\u{0}b"] {
            assert!(!acceptable(bad), "{bad:?}");
        }
        assert!(!acceptable(&"a".repeat(SELECTOR_MAX + 1)));
    }

    #[test]
    fn page_words_are_short_and_on_one_line() {
        assert_eq!(plain("Cookie\nbar\u{7}", 100), "Cookiebar");
        assert_eq!(plain("abcdef", 3), "abc");
    }
}
