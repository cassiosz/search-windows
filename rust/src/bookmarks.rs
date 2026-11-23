//! Bookmarks, in the window: Ctrl+Shift+B and its card, the rows they add
//! under the field, and search://bookmarks. The list and its file are
//! crates/bookmarks; this is only when to read it, when to write it, and what
//! the card and the page may ask.
//!
//! Nothing is read until something needs it (Ctrl+Shift+B, a letter typed in
//! the field, the page): a launch never waits for bookmarks.json.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use searchwin_bookmarks::{Bookmarks, Node, Place, Toggled};
use searchwin_history::fuzzy;
use serde_json::{Value, json};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, HSTRING, w};

thread_local! {
    static BOOKMARKS: RefCell<Option<Bookmarks>> = const { RefCell::new(None) };
    /// Changed since last written; written with the session, once changes settle.
    static OWED: Cell<bool> = const { Cell::new(false) };
    /// Ctrl+Shift+B's card, while it is up.
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// The profile on screen's bookmarks: Personal's are where they always were.
fn file() -> PathBuf {
    crate::spaces::folder().join("bookmarks.json")
}

/// Another profile came on screen: its bookmarks are read the next time
/// they are wanted. The window's save has written these already.
pub fn unload() {
    save();
    close_card();
    BOOKMARKS.with(|cell| *cell.borrow_mut() = None);
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
/// can be kept too: you asked for it by name. Kept now or before, its card
/// opens, to rename or file it; the same keys never take it away.
pub fn bookmark_front() {
    let Some((url, title)) = crate::front_page() else { return };
    if !keepable(&url) {
        return;
    }
    let id = match with(|b| b.toggle(&url, &title)) {
        Toggled::Added(id) => {
            changed();
            id
        }
        Toggled::Already(id) => id,
    };
    open_card(id);
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
/// folders a row can be moved to.
fn list(query: &str) -> Value {
    with(|b| {
        let folders: Vec<Value> =
            b.folders().iter().map(|(n, depth)| json!({ "id": n.id, "title": n.title, "depth": depth })).collect();
        let mut value = json!({
            "tree": serde_json::to_value(b.roots()).unwrap_or(Value::Null),
            "folders": folders,
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

// The card (upstream BookmarkCard): the page just kept, or kept before, with
// its name to change and a folder to file it in. Each change is kept as it is
// made, so OK, Enter, Esc and a click elsewhere all only close it. A window of
// its own over the page, as a peek's panel: unlike the accounts' list it takes
// the keyboard, for its name box. Made when wanted, gone when closed.

/// Something in the card a click reaches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spot {
    Folder,
    Remove,
    Done,
}

struct Card {
    window: HWND,
    edit: HWND,
    id: String,
    hot: Option<Spot>,
    /// The title's semibold, made with the card and deleted with it.
    bold: HFONT,
    /// The name box is naming a new folder ("New folder…"), not the bookmark.
    new_folder: bool,
}

// In device-independent pixels, as upstream's card.
const CARD_W: i32 = 280;
const CARD_PAD: i32 = 14;
const CARD_ROW: i32 = 28;
/// The labels' column, and the gap after it.
const LABEL_W: i32 = 42;
const LABEL_GAP: i32 = 10;
const NAME_TOP: i32 = 44;
const FOLDER_TOP: i32 = 80;
const BUTTONS_TOP: i32 = 122;
const CARD_H: i32 = BUTTONS_TOP + CARD_ROW + CARD_PAD;
/// Away from the page's corner, under the bar.
const CARD_MARGIN: i32 = 12;
const CHEVRON_GLYPH: &str = "\u{E70D}";
/// The menu's ids: the top level, then the folders by their place, then "New folder…".
const MENU_TOP: usize = 1;
const MENU_NEW: usize = 0xFFFF;

fn card<R>(f: impl FnOnce(&mut Card) -> R) -> Option<R> {
    CARD.with(|c| c.borrow_mut().as_mut().map(f))
}

fn words(pt: &'static str, en: &'static str) -> &'static str {
    if crate::portuguese() { pt } else { en }
}

/// The folders a bookmark can be filed in, for the menu: their ids, names
/// indented by depth, and which one holds `id` now (None: the top level).
fn choices(b: &Bookmarks, id: &str) -> (Vec<(String, String)>, Option<String>) {
    let folders = b.folders().iter().map(|(n, depth)| (n.id.clone(), format!("{}{}", "   ".repeat(*depth), n.title))).collect();
    (folders, b.parent_of(id))
}

fn spot_rect(spot: Spot, dpi: i32) -> RECT {
    let s = |v| crate::scale(v, dpi);
    let control = s(CARD_PAD + LABEL_W + LABEL_GAP);
    let (top, bottom) = (s(BUTTONS_TOP), s(BUTTONS_TOP + CARD_ROW));
    match spot {
        Spot::Folder => RECT { left: control, top: s(FOLDER_TOP), right: s(CARD_W - CARD_PAD), bottom: s(FOLDER_TOP + CARD_ROW) },
        Spot::Remove => RECT { left: s(CARD_PAD - 8), top, right: s(CARD_PAD + 72), bottom },
        Spot::Done => RECT { left: s(CARD_W - CARD_PAD - 72), top, right: s(CARD_W - CARD_PAD), bottom },
    }
}

fn name_rect(dpi: i32) -> RECT {
    let s = |v| crate::scale(v, dpi);
    RECT { left: s(CARD_PAD + LABEL_W + LABEL_GAP), top: s(NAME_TOP), right: s(CARD_W - CARD_PAD), bottom: s(NAME_TOP + CARD_ROW) }
}

fn spot_at(x: i32, y: i32) -> Option<Spot> {
    let dpi = crate::dpi();
    [Spot::Folder, Spot::Remove, Spot::Done].into_iter().find(|s| crate::inside(spot_rect(*s, dpi), x, y))
}

fn register_card() {
    if REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(card_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("SearchWinBookmarkCard"),
            // Floating over the page: a soft shadow under it.
            style: CS_DROPSHADOW,
            ..Default::default()
        });
    }
}

/// The card for `id`, at the page's top right corner, its name ready to type over.
fn open_card(id: String) {
    close_card();
    let (Some(main), Some(page), Some(look)) = (crate::window(), crate::card_on_screen(), crate::look()) else { return };
    let Some(node) = with(|b| b.get(&id).cloned()) else { return };
    register_card();
    let dpi = crate::dpi();
    let (width, height) = (crate::scale(CARD_W, dpi), crate::scale(CARD_H, dpi));
    let margin = crate::scale(CARD_MARGIN, dpi);
    let left = (page.right - width - margin).max(page.left);
    let instance = unsafe { GetModuleHandleW(None).ok().map(|h| h.into()) };
    // Owned by the window: above its pages, gone with it.
    let Ok(window) = (unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("SearchWinBookmarkCard"),
            w!("Bookmark"),
            WS_POPUP | WS_CLIPCHILDREN,
            left,
            page.top + margin,
            width,
            height,
            Some(main),
            None,
            instance,
            None,
        )
    }) else {
        return;
    };
    // Inside the rounded box drawn for it, one line tall, centred.
    let boxed = name_rect(dpi);
    let line = crate::scale(20, dpi);
    let inset = crate::scale(8, dpi);
    let top = boxed.top + (boxed.bottom - boxed.top - line) / 2;
    let edit = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            &HSTRING::from(node.title.as_str()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(crate::ES_AUTOHSCROLL),
            boxed.left + inset,
            top,
            boxed.right - boxed.left - 2 * inset,
            line,
            Some(window),
            None,
            instance,
            None,
        )
    }
    .unwrap_or_default();
    // The title a weight above the rest, as upstream's semibold.
    let bold = unsafe { CreateFontW(-crate::scale(crate::FONT, dpi), 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, w!("Segoe UI Variable Text")) };
    CARD.with(|c| *c.borrow_mut() = Some(Card { window, edit, id, hot: None, bold, new_folder: false }));
    let cue = HSTRING::from(node.url.as_deref().map(searchwin_bookmarks::pretty).unwrap_or_default());
    unsafe {
        let round = DWMWCP_ROUND;
        let dark = BOOL::from(crate::is_dark());
        let _ = DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(look.font.0 as usize)), Some(LPARAM(0)));
        // Emptied, the box shows what the lists will read.
        SendMessageW(edit, crate::EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        SendMessageW(edit, crate::EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        let _ = ShowWindow(window, SW_SHOW);
        let _ = SetFocus(Some(edit));
    }
}

/// Puts the card away. Its changes are kept already.
pub fn close_card() {
    if let Some(window) = card(|c| c.window) {
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
}

fn edit_text(edit: HWND) -> String {
    let mut buffer = [0u16; 1024];
    let n = unsafe { GetWindowTextW(edit, &mut buffer) };
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}

fn set_edit_text(edit: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(edit, &HSTRING::from(text));
        SendMessageW(edit, crate::EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
}

/// The name box changed: the bookmark's name follows as it is typed. An empty
/// name is kept as empty (the lists then read the address), as upstream.
fn typed() {
    let Some((edit, id, new_folder)) = card(|c| (c.edit, c.id.clone(), c.new_folder)) else { return };
    let text = edit_text(edit);
    // The box set back to the name it has is no change to write.
    if !new_folder && with(|b| b.get(&id).is_some_and(|n| n.title != text) && b.rename(&id, &text)) {
        changed();
    }
}

/// Enter or Esc while the card is up: false with none. Naming a new folder,
/// Enter makes it and Esc goes back to the bookmark's name; otherwise both close.
pub fn card_key(enter: bool) -> bool {
    let Some((edit, new_folder)) = card(|c| (c.edit, c.new_folder)) else { return false };
    if !new_folder {
        close_card();
    } else if enter {
        new_folder_named(&edit_text(edit));
    } else {
        name_mode();
    }
    true
}

/// The name box back to the bookmark's name.
fn name_mode() {
    let Some((window, edit, id)) = card(|c| {
        c.new_folder = false;
        (c.window, c.edit, c.id.clone())
    }) else {
        return;
    };
    let (title, url) = with(|b| b.get(&id).map(|n| (n.title.clone(), n.url.clone().unwrap_or_default()))).unwrap_or_default();
    unsafe {
        let cue = HSTRING::from(searchwin_bookmarks::pretty(&url));
        SendMessageW(edit, crate::EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        let _ = InvalidateRect(Some(window), None, false);
    }
    set_edit_text(edit, &title);
}

/// "New folder…": the name box names the folder, until Enter or Esc.
fn folder_mode() {
    let Some((window, edit)) = card(|c| {
        c.new_folder = true;
        (c.window, c.edit)
    }) else {
        return;
    };
    let cue = HSTRING::from(words("Nome da nova pasta", "New folder's name"));
    unsafe {
        SendMessageW(edit, crate::EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
        let _ = InvalidateRect(Some(window), None, false);
    }
    set_edit_text(edit, "");
    unsafe {
        let _ = SetFocus(Some(edit));
    }
}

fn new_folder_named(name: &str) {
    let Some(id) = card(|c| c.id.clone()) else { return };
    // Enter on an empty box is a change of mind, not a folder called "Folder".
    if !name.trim().is_empty() && with(|b| b.file_in_new_folder(&id, name)).is_some() {
        changed();
    }
    name_mode();
}

/// Into folder `to` (None: the top level).
fn file_in(to: Option<&str>) {
    let Some((window, id)) = card(|c| (c.window, c.id.clone())) else { return };
    if with(|b| b.move_to(&id, to, None)) {
        changed();
    }
    unsafe {
        let _ = InvalidateRect(Some(window), None, false);
    }
}

fn remove_carded() {
    let Some(id) = card(|c| c.id.clone()) else { return };
    close_card();
    if with(|b| b.remove(&id)) {
        changed();
    }
}

/// Where it is filed, and every other folder to file it in, under the folder's box.
fn folder_menu(window: HWND) {
    let Some(id) = card(|c| c.id.clone()) else { return };
    let (folders, here) = with(|b| choices(b, &id));
    let at = spot_rect(Spot::Folder, crate::dpi());
    let mut corner = POINT { x: at.left, y: at.bottom + crate::scale(4, crate::dpi()) };
    let chosen = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | check(here.is_none()), MENU_TOP, &HSTRING::from(words("Nível superior", "Top level")));
        if !folders.is_empty() {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        for (i, (folder, label)) in folders.iter().enumerate() {
            let _ = AppendMenuW(menu, MF_STRING | check(here.as_deref() == Some(folder)), MENU_TOP + 1 + i, &HSTRING::from(label.as_str()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_NEW, &HSTRING::from(words("Nova pasta…", "New folder…")));
        let _ = ClientToScreen(window, &mut corner);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTBUTTON, corner.x, corner.y, None, window, None).0 as usize;
        let _ = DestroyMenu(menu);
        chosen
    };
    match chosen {
        0 => {}
        MENU_TOP => file_in(None),
        MENU_NEW => folder_mode(),
        n => {
            if let Some((folder, _)) = folders.get(n - MENU_TOP - 1) {
                file_in(Some(folder));
            }
        }
    }
}

fn paint_card(window: HWND) {
    let Some(look) = crate::look() else { return };
    let Some((id, hot, new_folder, bold)) = card(|c| (c.id.clone(), c.hot, c.new_folder, c.bold)) else { return };
    let (folder, gone) = with(|b| (b.parent_of(&id).and_then(|f| b.get(&f).map(|n| n.title.clone())), b.get(&id).is_none()));
    let p = look.palette;
    let dpi = crate::dpi();
    let s = |v| crate::scale(v, dpi);
    let radius = s(8);
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(window, &mut client);
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(window, &mut paint);
        crate::buffered(hdc, client, |canvas, area| {
            crate::fill(canvas, area, p.card);
            let head = RECT { left: s(CARD_PAD), top: s(CARD_PAD), right: s(CARD_W - CARD_PAD), bottom: s(CARD_PAD + 20) };
            let title = if gone { words("Removido", "Removed") } else { words("Favorito adicionado", "Bookmarked") };
            crate::text(canvas, title, head, bold, p.text, false);
            let label = |top: i32, words: &str| {
                let r = RECT { left: s(CARD_PAD), top: s(top), right: s(CARD_PAD + LABEL_W), bottom: s(top + CARD_ROW) };
                crate::text(canvas, words, r, look.small_font, p.dim, false);
            };
            label(NAME_TOP, if new_folder { words("Pasta", "Folder") } else { words("Nome", "Name") });
            label(FOLDER_TOP, words("Pasta", "Folder"));
            crate::rounded(canvas, name_rect(dpi), p.field, p.border, radius);
            let boxed = spot_rect(Spot::Folder, dpi);
            let fill = if hot == Some(Spot::Folder) { p.bar_hover } else { p.field };
            crate::rounded(canvas, boxed, fill, p.border, radius);
            let chevron = RECT { left: boxed.right - s(26), ..boxed };
            let inner = RECT { left: boxed.left + s(8), right: chevron.left, ..boxed };
            let here = folder.as_deref().unwrap_or(words("Nível superior", "Top level"));
            crate::text(canvas, here, inner, look.font, p.text, false);
            crate::text(canvas, CHEVRON_GLYPH, chevron, look.caption_glyphs, p.dim, true);
            let remove = spot_rect(Spot::Remove, dpi);
            if hot == Some(Spot::Remove) {
                crate::rounded(canvas, remove, p.bar_hover, p.bar_hover, radius);
            }
            let red = if crate::is_dark() { (255, 128, 120) } else { (196, 43, 28) };
            // Its words on the labels' edge; the hover reaches past them.
            let words_at = RECT { left: remove.left + s(8), ..remove };
            crate::text(canvas, words("Remover", "Remove"), words_at, look.font, red, false);
            let done = spot_rect(Spot::Done, dpi);
            let accent = if hot == Some(Spot::Done) { crate::mix(p.accent, p.text, 12) } else { p.accent };
            crate::rounded(canvas, done, accent, accent, radius);
            let on_accent = if crate::is_dark() { (0, 0, 0) } else { (255, 255, 255) };
            crate::text(canvas, "OK", done, look.font, on_accent, true);
        });
        let _ = EndPaint(window, &paint);
    }
}

extern "system" fn card_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_card(window);
                LRESULT(0)
            }
            // A click elsewhere: it goes, from the message loop, not inside its own activation.
            WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_DESTROY => {
                let gone = CARD.with(|c| {
                    let mut c = c.borrow_mut();
                    if c.as_ref().is_some_and(|c| c.window == window) { c.take() } else { None }
                });
                if let Some(gone) = gone {
                    let _ = DeleteObject(gone.bold.into());
                }
                LRESULT(0)
            }
            WM_COMMAND if ((wparam.0 >> 16) & 0xffff) as u32 == crate::EN_CHANGE => {
                typed();
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                let Some(look) = crate::look() else { return DefWindowProcW(window, message, wparam, lparam) };
                let hdc = HDC(wparam.0 as _);
                SetTextColor(hdc, crate::color(look.palette.text));
                SetBkColor(hdc, crate::color(look.palette.field));
                SetDCBrushColor(hdc, crate::color(look.palette.field));
                LRESULT(GetStockObject(DC_BRUSH).0 as isize)
            }
            WM_MOUSEMOVE => {
                let spot = spot_at(x, y);
                if card(|c| std::mem::replace(&mut c.hot, spot) != spot) == Some(true) {
                    let _ = InvalidateRect(Some(window), None, false);
                }
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: window, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
                LRESULT(0)
            }
            crate::WM_MOUSELEAVE => {
                if card(|c| c.hot.take().is_some()) == Some(true) {
                    let _ = InvalidateRect(Some(window), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                match spot_at(x, y) {
                    Some(Spot::Folder) => folder_menu(window),
                    Some(Spot::Remove) => remove_carded(),
                    Some(Spot::Done) => close_card(),
                    None => {}
                }
                LRESULT(0)
            }
            // Enter and Esc typed in the name box reach card_key from the message loop, as every key does.
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// The card as a script sees it.
pub fn card_bench() -> Value {
    let Some((window, edit, id, new_folder)) = card(|c| (c.window, c.edit, c.id.clone(), c.new_folder)) else {
        return json!({ "open": false });
    };
    let (title, folder) = with(|b| (b.get(&id).map(|n| n.title.clone()), b.parent_of(&id).and_then(|f| b.get(&f).map(|n| n.title.clone()))));
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(window, &mut r);
    }
    json!({
        "open": true,
        "id": id,
        "title": title,
        "folder": folder,
        "text": edit_text(edit),
        "new_folder": new_folder,
        "focused": unsafe { GetFocus() } == edit,
        "rect": [r.left, r.top, r.right - r.left, r.bottom - r.top],
    })
}

/// What only the mouse reaches, by name: "name <text>" types into the box,
/// "folder top|<n>" files it (the menu's folders, from 0), "new <name>" files it
/// in a new folder, "remove" and "done" are its buttons.
pub fn card_act(what: &str) -> bool {
    let Some((edit, id)) = card(|c| (c.edit, c.id.clone())) else { return false };
    let (verb, rest) = what.trim().split_once(' ').unwrap_or((what.trim(), ""));
    match verb {
        // Through the box itself, so its EN_CHANGE renames, as typing does.
        "name" => set_edit_text(edit, rest),
        "folder" if rest == "top" => file_in(None),
        "folder" => {
            let (folders, _) = with(|b| choices(b, &id));
            let Some((folder, _)) = rest.parse::<usize>().ok().and_then(|n| folders.get(n)) else { return false };
            file_in(Some(folder));
        }
        "new" => {
            folder_mode();
            set_edit_text(edit, rest);
            card_key(true);
        }
        "remove" => remove_carded(),
        "done" => close_card(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_menu_lists_every_folder_indented_and_where_it_is() {
        let mut b = Bookmarks::new();
        let site = b.add("https://a.test/", "A").unwrap();
        let work = b.add_folder(None, "Work");
        let deep = b.add_folder(Some(&work), "Deep");
        let (folders, here) = choices(&b, &site);
        assert_eq!(folders, vec![(work.clone(), "Work".to_string()), (deep.clone(), "   Deep".to_string())]);
        assert_eq!(here, None, "at the top level");
        assert!(b.move_to(&site, Some(&deep), None));
        assert_eq!(choices(&b, &site).1, Some(deep));
    }

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
