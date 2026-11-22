//! search://import (upstream's Import sheet): bookmarks, history, open tabs,
//! and Arc's spaces and pins, from another browser on this PC.
//!
//! Nothing is looked at until the page is opened and asks; the reading is
//! done on a thread of its own (databases are copied first, see
//! crates/import), and what was read is put in place from the window's loop.
//! import.json, beside the settings, remembers what came from where, so
//! bringing again adds only what is new. Arc's spaces become spaces here;
//! Spaces is turned on only when you tick it.
//!
//! Passwords aren't read from another browser (it locks them to itself): the
//! page says where each one exports a CSV, which search://passwords' import
//! takes (crate::passwords).

use std::cell::RefCell;
use std::sync::Mutex;

use searchwin_bookmarks::Node;
use searchwin_history::{Entry, History, now_ms};
use searchwin_import::arc::{Pin, Sidebar};
use searchwin_import::{Added, Bases, Engine, Records, Snapshot};
use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::page::{self, TabId};

/// Posted when a reading is done: it is put in place from the loop.
pub const WM_IMPORT: u32 = WM_APP + 9;

/// What was asked to come over.
struct Job {
    tab: TabId,
    id: Value,
    browser: String,
    profile: Option<String>,
    bookmarks: bool,
    history: bool,
    tabs: bool,
    arc: bool,
    spaces_on: bool,
}

/// What the thread read.
#[derive(Default)]
struct Read {
    found: bool,
    bookmarks: Vec<Node>,
    history: Vec<Entry>,
    tabs: Snapshot,
    arc: Option<Sidebar>,
}

static DONE: Mutex<Option<(Job, Read)>> = Mutex::new(None);

thread_local! {
    static RUNNING: RefCell<bool> = const { RefCell::new(false) };
}

fn records_file() -> std::path::PathBuf {
    crate::data_folder().join("import.json")
}

/// This user's browsers, or a test run's made-up ones (SEARCHWIN_IMPORT_BASES).
fn bases() -> Option<Bases> {
    Bases::from_env()
}

/// What search://import asks. Null is "answered later".
pub fn message(tab: TabId, op: &str, asked: &Value) -> Value {
    let flag = |key: &str| asked.get(key).and_then(Value::as_bool).unwrap_or(false);
    match op {
        "import.list" => list(),
        "import.run" => {
            if RUNNING.with(|r| r.replace(true)) {
                return json!({ "error": "busy" });
            }
            let job = Job {
                tab,
                id: asked.get("id").cloned().unwrap_or(Value::Null),
                browser: asked.get("browser").and_then(Value::as_str).unwrap_or_default().to_string(),
                profile: asked.get("profile").and_then(Value::as_str).map(str::to_string),
                bookmarks: flag("bookmarks"),
                history: flag("history"),
                tabs: flag("tabs"),
                arc: flag("arc"),
                spaces_on: flag("spaces_on"),
            };
            let window = crate::window().map_or(0, |w| w.0 as isize);
            std::thread::spawn(move || {
                let read = read(&job);
                if let Ok(mut slot) = DONE.lock() {
                    *slot = Some((job, read));
                }
                unsafe {
                    let _ = PostMessageW(Some(HWND(window as _)), WM_IMPORT, WPARAM(0), LPARAM(0));
                }
            });
            Value::Null
        }
        _ => json!({ "error": "unknown op" }),
    }
}

fn list() -> Value {
    let Some(bases) = bases() else { return json!({ "browsers": [] }) };
    let records = Records::load(&records_file());
    let browsers: Vec<Value> = searchwin_import::discover(&bases)
        .iter()
        .map(|b| {
            let record = records.of(&b.name).map(|r| {
                json!({ "date": r.date, "bookmarks": r.bookmarks, "places": r.places, "tabs": r.tabs, "spaces": r.spaces, "pinned": r.pinned })
            });
            json!({
                "name": b.name,
                "firefox": b.engine == Engine::Firefox,
                "profiles": b.profiles.iter().map(|p| json!({ "id": p.id, "name": p.name })).collect::<Vec<_>>(),
                "usual": b.usual,
                "arc": b.arc_sidebar.is_some(),
                "hint": searchwin_import::password_hint(b),
                "record": record,
            })
        })
        .collect();
    json!({ "browsers": browsers })
}

/// Off the window's thread: the browser's files, read from copies.
fn read(job: &Job) -> Read {
    let Some(browser) = bases().and_then(|b| searchwin_import::discover(&b).into_iter().find(|x| x.name == job.browser)) else {
        return Read::default();
    };
    let profile = job.profile.as_deref();
    Read {
        found: true,
        bookmarks: if job.bookmarks { searchwin_import::bookmarks(&browser, profile).0 } else { Vec::new() },
        history: if job.history { searchwin_import::history(&browser, profile, searchwin_import::HISTORY_LIMIT) } else { Vec::new() },
        tabs: if job.tabs { searchwin_import::open_tabs(&browser, profile) } else { Snapshot::default() },
        arc: if job.arc { searchwin_import::arc::sidebar(&browser, profile) } else { None },
    }
}

/// On WM_IMPORT: what was read goes in, and the page is told how much.
pub fn act() {
    let Some((job, read)) = DONE.lock().ok().and_then(|mut slot| slot.take()) else { return };
    RUNNING.with(|r| *r.borrow_mut() = false);
    let mut answer = if read.found { put_in_place(&job, read) } else { json!({ "error": "not found" }) };
    answer["id"] = job.id.clone();
    page::post_to(job.tab, "import", &answer.to_string());
}

fn put_in_place(job: &Job, read: Read) -> Value {
    let name = job.browser.as_str();
    let mut records = Records::load(&records_file());
    let mut added = Added::default();
    let mut already = 0;
    if !read.bookmarks.is_empty() {
        (added.bookmarks, already) = crate::bookmarks::take(read.bookmarks, name);
    }
    let fresh = records.fresh_history(name, read.history);
    if !fresh.is_empty() && add_history(fresh.clone()) {
        added.places = fresh.len();
    }
    let mut tabs = read.tabs;
    tabs.retain(|t| records.first_time(name, "tab", &t.url));
    if tabs.tabs() > 0 {
        added.tabs = crate::state(|s| {
            s.tabs.set_now(now_ms());
            s.session_owed = true;
            tabs.open_into(&mut s.tabs)
        })
        .unwrap_or(0);
    }
    if let Some(sidebar) = read.arc {
        (added.spaces, added.pinned) = arc(name, sidebar, &mut records, job.spaces_on);
    } else if job.spaces_on {
        turn_spaces_on();
    }
    let answer = json!({
        "bookmarks": added.bookmarks,
        "already": already,
        "places": added.places,
        "tabs": added.tabs,
        "spaces": added.spaces,
        "pinned": added.pinned,
    });
    records.note(name, added, now_ms());
    if let Err(error) = records.save(&records_file()) {
        crate::crash_log(&format!("import record not saved: {error}"));
    }
    crate::repaint_column();
    crate::save_soon();
    answer
}

/// Places brought into the history on screen, each counted as it was there.
/// The history is written first and read back whole (hidden front doors and
/// all), so nothing kept is lost; if that can't be done, nothing comes.
fn add_history(fresh: Vec<Entry>) -> bool {
    let file = crate::spaces::folder().join("history.json");
    crate::state(|s| {
        if crate::history(s).save(&file).is_err() {
            return false;
        }
        let Some(mut all) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok()) else { return false };
        all.extend(fresh);
        s.history = Some(History::from_entries(all));
        s.history_owed = true;
        true
    })
    .unwrap_or(false)
}

fn turn_spaces_on() {
    crate::state(|s| s.spaces.set_uses_spaces(true));
    crate::spaces::changed();
}

/// Arc's spaces become spaces (one of the same name is used again), their
/// pins its pinned tabs, the favourites pinned in the row on screen.
/// Folders of pins come as their pins: a row of tabs has no folders.
/// Returns (spaces made, pins brought).
fn arc(name: &str, sidebar: Sidebar, records: &mut Records, spaces_on: bool) -> (usize, usize) {
    fn flat(pins: Vec<Pin>, out: &mut Vec<(String, String)>) {
        for pin in pins {
            match pin {
                Pin::Item(item) => out.push((item.url, item.title)),
                Pin::Folder { items, .. } => flat(items, out),
            }
        }
    }
    let Some((original, row_of)) = crate::state(|s| (s.spaces.current_id().to_string(), s.row_of.clone())) else { return (0, 0) };
    let (mut made, mut pinned) = (0, 0);
    let favourites: Vec<(String, String)> = sidebar
        .favorites
        .into_iter()
        .filter(|f| records.first_time(name, "favorite", &f.url))
        .map(|f| (f.url, f.title))
        .collect();
    pinned += pin_into(&row_of, &row_of, favourites);
    for space in sidebar.spaces {
        let mut pins = Vec::new();
        flat(space.pinned, &mut pins);
        pins.retain(|(url, _)| records.first_time(name, "pin", &format!("{}\n{url}", space.name)));
        let id = crate::state(|s| {
            if let Some(there) = s.spaces.all().iter().find(|x| x.name == space.name) {
                return there.id.clone();
            }
            made += 1;
            // Arc's other profiles sign in apart: a space with cookies of its own.
            s.spaces.add_space(&space.name, Some(space.symbol), space.profile == "Default")
        });
        if let Some(id) = id {
            pinned += pin_into(&id, &row_of, pins);
        }
    }
    // Making a space goes to it; the window stays where it was.
    crate::state(|s| {
        s.spaces.switch_to(&original);
        if spaces_on {
            s.spaces.set_uses_spaces(true);
        }
    });
    if made > 0 || spaces_on {
        crate::spaces::changed();
    }
    (made, pinned)
}

/// Pinned tabs added to space `space`'s row: the one on screen as it is,
/// any other in its file, asleep until it is shown.
fn pin_into(space: &str, row_of: &str, pins: Vec<(String, String)>) -> usize {
    if pins.is_empty() {
        return 0;
    }
    let add = |tabs: &mut searchwin_tabs::Tabs| {
        for (url, title) in &pins {
            let id = tabs.open_in_background(url, None);
            if let Some(t) = tabs.get_mut(id) {
                t.title = title.clone();
            }
            tabs.pin(id);
        }
    };
    if space == row_of {
        crate::state(|s| {
            s.tabs.set_now(now_ms());
            add(&mut s.tabs);
            s.session_owed = true;
        });
    } else {
        let path = searchwin_spaces::session_path(&crate::data_folder(), space);
        let mut tabs = searchwin_tabs::load(&path).into_tabs(now_ms());
        add(&mut tabs);
        if let Err(error) = searchwin_tabs::save(&path, &searchwin_tabs::Shape::from_tabs(&tabs)) {
            crate::crash_log(&format!("imported pins not saved: {error}"));
            return 0;
        }
    }
    pins.len()
}

/// Whether a reading is under way (the bench's).
pub fn bench() -> Value {
    // What came from each browser, without the long lists of what was seen.
    let mut records = serde_json::to_value(Records::load(&records_file())).unwrap_or(Value::Null);
    for record in records.as_object_mut().into_iter().flat_map(|m| m.values_mut()) {
        if let Some(record) = record.as_object_mut() {
            record.retain(|key, _| !matches!(key.as_str(), "visits" | "seen" | "bookmarkIDs"));
        }
    }
    json!({ "running": RUNNING.with(|r| *r.borrow()), "records": records })
}
