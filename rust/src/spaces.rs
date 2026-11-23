//! Spaces and profiles in the window; crates/spaces is the model. Which row
//! of tabs is on screen and which store its pages use, the chips at the
//! column's foot, what Settings may ask, and clearing away what a deleted
//! space or profile leaves.
//!
//! Off by default, as upstream: with no spaces.json there is one space, its
//! row is session.json, its pages use the store there always was, and
//! nothing here draws or runs.
//!
//! Keys (upstream has ⌃1–9 for spaces and ⌘1–9 for tabs; on Windows both
//! would be Ctrl): Ctrl+1–9 stay the tabs, Ctrl+Alt+1–9 are the spaces, and
//! the mouse's side buttons over the column go to the space before or after.
//! AltGr is Ctrl+Alt to Windows and types characters, so it is left alone.
//!
//! A space left behind keeps its row of tabs but not their pages: they are
//! let go as a tab asleep lets go of its own, and the row comes back asleep
//! but for the tab in front, as a launch brings it. A parked space costs its
//! list and nothing more. Private tabs keep nothing, so they are closed
//! rather than taken along.

use std::cell::RefCell;
use std::path::PathBuf;

use searchwin_history::now_ms;
use searchwin_spaces::{COLOUR_NAMES, ICON_NAMES, ICONS, Spaces};
use serde_json::{Value, json};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::{data_folder, state};

/// Posted when the space on screen is to change. A row of tabs is never
/// swapped inside an event of one of its own pages (Ctrl+Alt+2 pressed in
/// a page, a switch flipped in Settings, a link sent): only from the loop.
pub const WM_SETTLE: u32 = WM_APP + 3;

thread_local! {
    /// The profile on screen's folder, where history.json and bookmarks.json
    /// are. A cell of its own, not the window's state: both are read from
    /// inside that.
    static FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn path() -> PathBuf {
    searchwin_spaces::spaces_path(&data_folder())
}

/// Where the profile on screen keeps its own files: the data folder itself
/// for Personal, as there always was.
pub fn folder() -> PathBuf {
    FOLDER.with(|f| f.borrow().clone()).unwrap_or_else(data_folder)
}

fn set_folder(profile: &str) {
    let folder = searchwin_spaces::profile_folder(&data_folder(), profile);
    let _ = std::fs::create_dir_all(&folder);
    FOLDER.with(|f| *f.borrow_mut() = Some(folder));
}

/// A name the engine made a folder of: never a path.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// spaces.json as the window opens; none is one space, switched off.
/// WebView2 stores a deletion left behind (the engine was gone before it
/// could let them go) are removed now, before the engine starts and holds
/// them again (Spaces.swift:259-282).
pub fn load() -> Spaces {
    let mut spaces = Spaces::load(&path());
    if !spaces.current_profile().eq(searchwin_spaces::PERSONAL_ID) {
        set_folder(spaces.current_profile());
    }
    if !spaces.erasing().is_empty() {
        let engine = data_folder().join("Pages").join("EBWebView");
        for name in spaces.erasing().to_vec() {
            // The engine keeps a named profile in WV2Profile_<name, lowercased>.
            let store = engine.join(format!("WV2Profile_{}", name.to_lowercase()));
            let gone = !plain_name(&name)
                || match std::fs::remove_dir_all(store) {
                    Ok(()) => true,
                    Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                };
            if gone {
                spaces.erased(&name);
            }
        }
        let _ = spaces.save(&path());
    }
    spaces
}

fn save() {
    let written = state(|s| s.spaces.save(&path()));
    if let Some(Err(error)) = written {
        crate::crash_log(&format!("spaces not saved: {error}"));
    }
}

pub fn uses_spaces() -> bool {
    state(|s| s.spaces.uses_spaces()).unwrap_or(false)
}

/// The WebView2 profile the tabs on screen use; None is the default one,
/// the store there always was.
pub fn store() -> Option<String> {
    state(|s| s.spaces.webview_profile_for(&s.row_of)).flatten()
}

/// A WebView2 profile page::erase removed.
pub fn erased(name: &str) {
    state(|s| s.spaces.erased(name));
    save();
}

pub fn request_settle() {
    if let Some(window) = state(|s| s.window) {
        unsafe {
            let _ = PostMessageW(Some(window), WM_SETTLE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Something moved the model to another space, or deleted one: written,
/// and the window follows from the loop.
pub fn changed() {
    save();
    request_settle();
    crate::repaint_column();
    crate::sync::poke();
}

/// Ctrl+Alt+1–9: the Nth space the window can reach. False with spaces off,
/// so the keys stay whatever they are for the page.
pub fn select_number(n: usize) -> bool {
    match state(|s| s.spaces.uses_spaces().then(|| s.spaces.select_number(n).is_some())).flatten() {
        Some(moved) => {
            if moved {
                changed();
            }
            true
        }
        None => false,
    }
}

/// The side buttons: the space before (-1) or after (1), no wrapping.
pub fn step(by: isize) -> bool {
    let moved = state(|s| {
        let moved = if by < 0 { s.spaces.previous_space() } else { s.spaces.next_space() };
        moved.is_some()
    });
    if moved == Some(true) {
        changed();
    }
    moved.is_some() && uses_spaces()
}

/// A chip in the column, clicked.
pub fn go_to(id: &str) {
    if state(|s| s.spaces.switch_to(id).is_some()) == Some(true) {
        changed();
    }
}

/// The space a rule sends a link to, when it isn't the one on screen.
pub fn sends_to(url: &str) -> Option<String> {
    state(|s| s.spaces.sends_to(url, &s.row_of).map(|sp| sp.id.clone())).flatten()
}

/// A link sent to its space (SpaceRoutes.swift:153-167): the window goes
/// there, with its sign-ins, and the link opens in front in its row. A
/// space of another profile is reached through that profile.
pub fn send(url: &str, space: &str) {
    let went = state(|s| {
        if s.spaces.switch_to(space).is_none() {
            let profile = s.spaces.profile_of(space).to_string();
            if profile != s.spaces.current_profile() {
                s.spaces.open_profile(&profile);
                s.spaces.switch_to(space);
            }
        }
        let went = s.spaces.current_id() == space;
        if went {
            s.sent.push(url.to_string());
        }
        went
    });
    if went == Some(true) {
        changed();
    }
}

/// Puts the space chosen on screen if it isn't already, then clears away
/// what a deletion left: session files, a profile's folder, WebView2 stores.
pub fn settle() {
    let Some((from, to)) = state(|s| (s.row_of.clone(), s.spaces.current_id().to_string())) else { return };
    if from != to {
        swap(&to);
    }
    let removals = state(|s| std::mem::take(&mut s.removals)).unwrap_or_default();
    if !removals.is_empty() {
        let base = data_folder();
        let mut stores = Vec::new();
        for removal in removals {
            state(|s| {
                for space in &removal.spaces {
                    s.rows.remove(space);
                }
            });
            removal.erase_files(&base);
            stores.extend(removal.webview_profiles);
        }
        crate::page::erase(stores);
        save();
    }
    // Links sent here from a page in the space left open in front.
    let sent = state(|s| std::mem::take(&mut s.sent)).unwrap_or_default();
    if !sent.is_empty() {
        crate::open_in_tabs(&sent);
    }
}

fn swap(to: &str) {
    let split_on = crate::split::on();
    // Private tabs keep nothing: they close rather than go with the row.
    state(|s| {
        s.tabs.set_now(now_ms());
        for id in std::mem::take(&mut s.private).into_keys() {
            s.splits.remove(id);
            if let Some(tab) = s.tabs.get_mut(id) {
                tab.url = None;
                tab.title.clear();
            }
            s.tabs.close(id);
        }
        s.session_owed = true;
    });
    // The row leaving is written, and history and bookmarks while the
    // profile is still theirs.
    crate::save();
    let pages = state(|s| s.tabs.iter().map(|t| t.id).collect::<Vec<_>>()).unwrap_or_default();
    crate::page::new_epoch();
    for id in pages {
        crate::page::discard(id);
        crate::veil::forget(id);
    }
    let base = data_folder();
    let profile = state(|s| {
        let now = now_ms();
        let ids: Vec<u64> = s.tabs.iter().map(|t| t.id).collect();
        if split_on {
            s.tabs.splits = searchwin_split::to_session(&s.splits, &ids);
        }
        s.rows.row(&base, to, now);
        let arriving = s.rows.remove(to).unwrap_or_default();
        let mut leaving = std::mem::replace(&mut s.tabs, arriving);
        let front = leaving.selected_id();
        for id in ids.into_iter().filter(|&id| id != front) {
            leaving.mark_asleep(id);
        }
        let from = std::mem::replace(&mut s.row_of, to.to_string());
        // A deleted space's row goes with it.
        if s.spaces.space(&from).is_some() {
            s.rows.insert(&from, leaving);
        }
        s.tabs.set_now(now);
        s.splits = if split_on { crate::split::from_row(&s.tabs) } else { searchwin_split::Splits::new() };
        s.scroll = 0;
        s.hover = None;
        let profile = s.spaces.profile_of(to).to_string();
        let moved = searchwin_spaces::profile_folder(&base, &profile) != folder();
        if moved {
            s.history = None;
            s.history_owed = false;
        }
        moved.then_some(profile)
    })
    .flatten();
    if let Some(profile) = profile {
        set_folder(&profile);
        crate::bookmarks::unload();
    }
    crate::arrived();
}

// ---- the column's chips ----

/// A space as its chip shows it.
pub struct Chip {
    pub id: String,
    pub letter: String,
    pub tint: Option<(u8, u8, u8)>,
    pub current: bool,
}

pub fn rgb(colour: usize) -> (u8, u8, u8) {
    let [r, g, b] = searchwin_spaces::COLOURS[colour];
    let byte = |x: f32| (x * 255.0).round() as u8;
    (byte(r), byte(g), byte(b))
}

/// The chips at the column's foot, the spaces the window can reach in
/// order; none with spaces off.
pub fn chips(spaces: &Spaces) -> Vec<Chip> {
    if !spaces.uses_spaces() {
        return Vec::new();
    }
    let current = spaces.current_id();
    spaces
        .reachable()
        .into_iter()
        .map(|s| Chip {
            id: s.id.clone(),
            letter: s.name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "•".into()),
            tint: s.tint().map(rgb),
            current: s.id == current,
        })
        .collect()
}

/// The space's colour washed faintly over the column (Spaces.swift:69-84):
/// how much, and of what. None with spaces off or no colour.
pub fn wash(spaces: &Spaces, dark: bool) -> Option<((u8, u8, u8), u32)> {
    if !spaces.uses_spaces() {
        return None;
    }
    let colour = spaces.current().tint()?;
    let amount = searchwin_spaces::wash(colour, dark)?;
    Some((rgb(colour), (amount * 100.0).round() as u32))
}

// ---- Settings ----

/// Everything Settings shows about spaces and profiles.
fn snapshot(spaces: &Spaces) -> Value {
    let list: Vec<Value> = spaces
        .all()
        .iter()
        .map(|s| {
            json!({
                "id": s.id, "name": s.name, "colour": s.colour, "icon": s.symbol(),
                "profile": s.profile_id(), "first": s.is_first(),
                "home": spaces.home_of(s.profile_id()) == s.id,
                "own_store": !s.shares_sign_ins,
            })
        })
        .collect();
    let profiles: Vec<Value> = spaces
        .profiles()
        .iter()
        .map(|p| json!({ "id": p.id, "name": p.name, "personal": p.is_personal(), "home": p.home }))
        .collect();
    let routes: Vec<Value> = spaces
        .routes()
        .iter()
        .map(|r| json!({ "site": r.site, "space": r.space, "label": spaces.label(&r.space) }))
        .collect();
    json!({
        "on": spaces.uses_spaces(),
        "sends_links": spaces.sends_links(),
        "one_window": spaces.profiles_in_one_window(),
        "current": spaces.current_id(),
        "profile": spaces.current_profile(),
        "spaces": list,
        "profiles": profiles,
        "routes": routes,
        "colours": COLOUR_NAMES,
        "icons": ICONS,
        "icon_names": ICON_NAMES,
    })
}

/// What search://settings asks about spaces and profiles. Every answer is
/// the whole picture again, with `error` when a change was refused.
pub fn message(op: &str, asked: &Value) -> Value {
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |key: &str| asked.get(key).and_then(Value::as_bool);
    let number = |key: &str| asked.get(key).and_then(Value::as_u64).map(|n| n as usize);
    let (space, name) = (text("space"), text("name"));
    let done = state(|s| {
        let sp = &mut s.spaces;
        let ok = match op {
            "spaces.get" => return None,
            "spaces.set" => match (text("key").as_str(), flag("value")) {
                ("on", Some(on)) => {
                    sp.set_uses_spaces(on);
                    true
                }
                ("sends_links", Some(on)) => {
                    sp.set_sends_links(on);
                    true
                }
                ("one_window", Some(on)) => {
                    sp.set_profiles_in_one_window(on);
                    true
                }
                _ => false,
            },
            // A new space has cookies of its own unless asked to share (Spaces.swift:469-486).
            "spaces.add" => !name.trim().is_empty() && {
                sp.add_space(&name, None, flag("shares").unwrap_or(false));
                true
            },
            "spaces.rename" => sp.rename_space(&space, &name),
            "spaces.colour" => number("colour").is_some_and(|c| sp.set_colour(&space, c)),
            "spaces.icon" => sp.set_icon(&space, &text("icon")),
            "spaces.move" => number("to").is_some_and(|to| sp.move_space(&space, to)),
            "spaces.open" => {
                sp.switch_to(&space);
                true
            }
            "spaces.delete" => match sp.delete_space(&space) {
                Some(removal) => {
                    s.removals.push(removal);
                    true
                }
                None => false,
            },
            "spaces.route" => sp.set_route(&text("site"), &space),
            "spaces.unroute" => sp.remove_route(&text("site")),
            "profiles.add" => sp.add_profile(&name).is_some(),
            "profiles.rename" => sp.rename_profile(&text("profile"), &name),
            "profiles.open" => {
                sp.open_profile(&text("profile"));
                true
            }
            "profiles.delete" => match sp.delete_profile(&text("profile")) {
                Some(removal) => {
                    s.removals.push(removal);
                    true
                }
                None => false,
            },
            _ => false,
        };
        Some(ok)
    })
    .flatten();
    if done.is_some() {
        changed();
    }
    let mut answer = state(|s| snapshot(&s.spaces)).unwrap_or(Value::Null);
    if done == Some(false) {
        answer["error"] = json!(format!("{op} refused"));
    }
    answer
}

// ---- the bench ----

pub fn bench(spaces: &Spaces, row_of: &str) -> Value {
    let mut value = snapshot(spaces);
    value["row"] = json!(row_of);
    value["row_file"] = json!(searchwin_spaces::session_file_name(row_of));
    value["store"] = json!(spaces.webview_profile_for(row_of));
    value["reachable"] = json!(spaces.reachable().iter().map(|s| s.id.clone()).collect::<Vec<_>>());
    value["erasing"] = json!(spaces.erasing());
    value["folder"] = json!(folder().to_string_lossy());
    for key in ["colours", "icons", "icon_names"] {
        if let Some(map) = value.as_object_mut() {
            map.remove(key);
        }
    }
    value
}
