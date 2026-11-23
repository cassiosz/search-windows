//! Chrome extensions (docs/extensoes.md). Off until Settings turns them on,
//! and on with none added they still cost nothing: WebView2 is told about
//! extensions only when the app starts with at least one in the list, since
//! the switch is fixed for the life of the page engine. So the first one
//! added waits for the next launch.
//!
//! WebView2 keeps extensions per profile and shows none of their buttons.
//! Each profile a page is made in is brought in line with the list the
//! first time this session (sync); InPrivate is left without them, as Chrome
//! leaves incognito. An extension's popup is opened here, in a small window
//! of ours under the bar's button, as a page of its own (page::peek).

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use searchwin_extensions::Extension;
use serde_json::{Value, json};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, HSTRING, Interface, PWSTR, w};

use crate::page::{self, TabId};

/// Chrome's own bounds for a popup, in CSS pixels.
const POPUP_MOST: (i32, i32) = (800, 600);
const POPUP_LEAST: i32 = 25;
/// Measured again this long after the page loads: most popups fill in after.
const REMEASURE_MS: u32 = 250;
const REMEASURE: usize = 1;

/// The popup page's own size: its widest line, and its height at that width.
const MEASURE: &str = "(()=>{const d=document.documentElement;d.style.width='max-content';\
const w=d.getBoundingClientRect().width;d.style.width='';return [Math.ceil(w),Math.ceil(d.getBoundingClientRect().height)]})()";

#[derive(Default)]
struct State {
    /// The page engine was started with extensions: fixed until the app closes.
    active: bool,
    /// Profiles brought in line with the list this session.
    synced: HashSet<String>,
    popup: Option<Popup>,
    registered: bool,
    /// The list as last read or written.
    list: Option<Vec<Extension>>,
    /// An extension's popup can be opened: the bar asks on every paint.
    has_popups: bool,
    /// When the popup last went: a click on the button that closed it doesn't open it again.
    closed_at: Option<std::time::Instant>,
}

struct Popup {
    window: HWND,
    page: TabId,
    from: TabId,
    /// The button it hangs from, on screen.
    anchor: RECT,
    shown: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn root() -> PathBuf {
    crate::data_folder().join("Extensions")
}

fn list_file() -> PathBuf {
    root().join("extensions.json")
}

fn list() -> Vec<Extension> {
    if let Some(list) = with(|s| s.list.clone()) {
        return list;
    }
    // Only the folders this app names (1, 2, 3...): a hand-edited entry can't reach elsewhere.
    let mut list = searchwin_extensions::load(&list_file());
    list.retain(|e| searchwin_extensions::is_dir_name(&e.dir));
    with(|s| s.list = Some(list.clone()));
    list
}

/// Whether the button has anything to open, worked out again when the list or the switch changes.
fn recount() {
    let any = with(|s| s.active) && turned_on() && list().iter().any(|e| e.on && e.popup.is_some() && !e.id.is_empty());
    with(|s| s.has_popups = any);
}

/// Written, and kept as written. None left removes the file: the next
/// launch is as if there had never been any.
fn store(list: Vec<Extension>) -> std::io::Result<()> {
    let saved = if list.is_empty() {
        std::fs::remove_file(list_file()).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
    } else {
        std::fs::create_dir_all(root()).and_then(|_| searchwin_extensions::save(&list_file(), &list))
    };
    with(|s| s.list = Some(list));
    recount();
    saved
}

fn turned_on() -> bool {
    crate::internal::prefs(|p| p.extensions)
}

/// Asked once, as the page engine is made: whether it is made with
/// extensions. With none added, the answer costs one look for a file.
pub fn start() -> bool {
    let root = root();
    if !list_file().is_file() {
        // The last one removed while WebView2 still held its files: gone now.
        if root.is_dir() {
            let _ = std::fs::remove_dir_all(&root);
        }
        return false;
    }
    let list = list();
    // Folders left by a removal WebView2 was still holding on to. A list
    // that won't read sweeps nothing: it may be the only record of them.
    if !list.is_empty() {
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if searchwin_extensions::is_dir_name(&name) && entry.path().is_dir() && !list.iter().any(|e| e.dir == name) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let active = turned_on() && !list.is_empty();
    with(|s| s.active = active);
    recount();
    active
}

/// A page was made: its profile takes the list, once a session.
pub fn sync(view: &ICoreWebView2) {
    if !with(|s| s.active) {
        return;
    }
    let Some((name, profile)) = profile_of(view) else { return };
    if with(|s| s.synced.insert(name)) {
        apply(profile);
    }
}

/// The profile a page is in, unless it is InPrivate.
fn profile_of(view: &ICoreWebView2) -> Option<(String, ICoreWebView2Profile7)> {
    unsafe {
        let profile = view.cast::<ICoreWebView2_13>().ok()?.Profile().ok()?;
        let mut private = BOOL::default();
        profile.IsInPrivateModeEnabled(&mut private).ok()?;
        if private.as_bool() {
            return None;
        }
        let mut name = PWSTR::null();
        profile.ProfileName(&mut name).ok()?;
        let name = take(name);
        Some((name, profile.cast().ok()?))
    }
}

fn take(text: PWSTR) -> String {
    let out = unsafe { text.to_string().unwrap_or_default() };
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    out
}

/// The list changed, or the switch: every profile with a page open now
/// follows at once; the rest when a page is next made in them.
fn refresh() {
    if !with(|s| {
        s.synced.clear();
        s.active
    }) {
        return;
    }
    for view in page::views() {
        sync(&view);
    }
}

/// One profile brought in line: what isn't in the list goes, what is comes
/// in, and each is on or off as the list (and the switch) says.
fn apply(profile: ICoreWebView2Profile7) {
    let target = profile.clone();
    let handler = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |result, found| {
        result?;
        let wanted = list();
        let master = turned_on();
        let mut have = HashSet::new();
        if let Some(found) = found {
            let mut count = 0;
            unsafe { found.Count(&mut count)? };
            for i in 0..count {
                let extension = unsafe { found.GetValueAtIndex(i)? };
                let mut id = PWSTR::null();
                unsafe { extension.Id(&mut id)? };
                let id = take(id);
                match wanted.iter().find(|e| e.id == id) {
                    None => unsafe {
                        let _ = extension.Remove(&BrowserExtensionRemoveCompletedHandler::create(Box::new(|_| Ok(()))));
                    },
                    Some(e) => {
                        have.insert(id);
                        set_enabled(&extension, e.on && master);
                    }
                }
            }
        }
        for e in wanted.into_iter().filter(|e| !have.contains(&e.id)) {
            add(&target, e, master);
        }
        Ok(())
    }));
    unsafe {
        let _ = profile.GetBrowserExtensions(&handler);
    }
}

fn set_enabled(extension: &ICoreWebView2BrowserExtension, on: bool) {
    let mut now = BOOL::default();
    unsafe {
        if extension.IsEnabled(&mut now).is_ok() && now.as_bool() != on {
            let _ = extension.Enable(on, &BrowserExtensionEnableCompletedHandler::create(Box::new(|_| Ok(()))));
        }
    }
}

/// Into a profile from its copy, keeping the id WebView2 gives it.
fn add(profile: &ICoreWebView2Profile7, e: Extension, master: bool) {
    let dir = e.dir.clone();
    let handler = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |result, extension| {
        if let Err(error) = result {
            crate::crash_log(&format!("extension {dir} not added: {error}"));
            return Ok(());
        }
        let Some(extension) = extension else { return Ok(()) };
        let mut id = PWSTR::null();
        unsafe { extension.Id(&mut id)? };
        let id = take(id);
        let mut list = list();
        if let Some(entry) = list.iter_mut().find(|x| x.dir == dir).filter(|x| x.id != id) {
            entry.id = id;
            let _ = store(list);
            crate::repaint_bar();
        }
        set_enabled(&extension, e.on && master);
        Ok(())
    }));
    let folder = HSTRING::from(root().join(&e.dir).as_os_str());
    unsafe {
        if let Err(error) = profile.AddBrowserExtension(&folder, &handler) {
            crate::crash_log(&format!("extension {} not added: {error}", e.dir));
        }
    }
}

/// What search://settings asks.
pub fn message(op: &str, asked: &Value) -> Value {
    let dir = asked.get("dir").and_then(Value::as_str).unwrap_or_default();
    let error = match op {
        "extensions.get" => None,
        "extensions.add" => pick_folder().and_then(|folder| added(&folder).err()),
        "extensions.set" => {
            let on = asked.get("on").and_then(Value::as_bool).unwrap_or(true);
            change(|list| list.iter_mut().filter(|e| e.dir == dir).for_each(|e| e.on = on))
        }
        "extensions.remove" => {
            let listed = list().iter().any(|e| e.dir == dir);
            let error = change(|list| list.retain(|e| e.dir != dir));
            // Only a folder the list had. In use until WebView2 lets it go:
            // whatever stays is swept at the next launch.
            if error.is_none() && listed {
                let _ = std::fs::remove_dir_all(root().join(dir));
            }
            error
        }
        _ => return json!({ "error": "unknown op" }),
    };
    let mut value = state();
    if let Some(error) = error {
        value["error"] = json!(error);
    }
    value
}

/// The list as Settings draws it.
fn state() -> Value {
    let list = list();
    let active = with(|s| s.active);
    let rows: Vec<Value> = list.iter().map(|e| json!({ "dir": e.dir, "name": e.name, "popup": e.popup.is_some(), "on": e.on })).collect();
    json!({ "list": rows, "active": active, "restart": turned_on() && !list.is_empty() && !active })
}

fn change(edit: impl FnOnce(&mut Vec<Extension>)) -> Option<String> {
    let mut list = list();
    edit(&mut list);
    if let Err(error) = store(list) {
        return Some(error.to_string());
    }
    refresh();
    crate::repaint_bar();
    None
}

/// An unpacked extension's folder, copied in and put on the list.
fn added(folder: &std::path::Path) -> std::result::Result<(), String> {
    let manifest = searchwin_extensions::read_manifest(folder)?;
    let root = root();
    let mut list = list();
    let dir = searchwin_extensions::free_dir(&root, &list);
    if let Err(error) = searchwin_extensions::copy_folder(folder, &root.join(&dir)) {
        let _ = std::fs::remove_dir_all(root.join(&dir));
        return Err(error.to_string());
    }
    list.push(Extension { dir, id: String::new(), name: manifest.name, popup: manifest.popup, icon: manifest.icon, on: true });
    store(list).map_err(|e| e.to_string())?;
    refresh();
    crate::repaint_bar();
    Ok(())
}

/// The folder to add from. A test run names it instead (SEARCHWIN_BENCH_FILE).
fn pick_folder() -> Option<PathBuf> {
    if std::env::var("SEARCHWIN_BENCH").as_deref() == Ok("1") {
        return std::env::var_os("SEARCHWIN_BENCH_FILE").map(Into::into);
    }
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS).ok()?;
        // Cancelled is an error here, and no folder.
        dialog.Show(crate::window()).ok()?;
        let name = dialog.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path.map(Into::into)
    }
}

/// Extensions with a popup to open, in the list's order: (dir, name). Empty
/// when the engine runs without them or the page in front is private.
pub fn popups() -> Vec<(String, String)> {
    if !with(|s| s.active) || !turned_on() || crate::front_tab().is_some_and(|(id, _)| crate::is_private(id)) {
        return Vec::new();
    }
    list().into_iter().filter(|e| e.on && e.popup.is_some() && !e.id.is_empty()).map(|e| (e.dir, e.name)).collect()
}

/// Whether the bar has the button: an extension with a popup to open, and
/// a page in front that isn't private. Asked from inside the window's state:
/// it reads only what is kept here.
pub fn button(front_private: bool) -> bool {
    !front_private && with(|s| s.has_popups)
}

/// The bar's button clicked, at `anchor` on screen: the one popup there is
/// opens, or a menu of them to choose from.
pub fn clicked(anchor: RECT) {
    // The click that took the focus from the popup closed it: that is all it does.
    if with(|s| s.closed_at.is_some_and(|t| t.elapsed().as_millis() < 300)) {
        return;
    }
    let popups = popups();
    let chosen = match popups.len() {
        0 => return,
        1 => 0,
        _ => {
            let Some(window) = crate::window() else { return };
            unsafe {
                let Ok(menu) = CreatePopupMenu() else { return };
                for (i, (_, name)) in popups.iter().enumerate() {
                    let _ = AppendMenuW(menu, MF_STRING, i + 1, &HSTRING::from(name.as_str()));
                }
                let picked = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTALIGN | TPM_TOPALIGN, anchor.right, anchor.bottom, None, window, None);
                let _ = DestroyMenu(menu);
                match picked.0 {
                    0 => return,
                    n => n as usize - 1,
                }
            }
        }
    };
    open_popup(&popups[chosen].0, anchor);
}

/// The Settings switch turned: extensions follow at once, on or off.
pub fn switched() {
    refresh();
    recount();
    crate::repaint_bar();
}

/// The popup of extension `dir`, under `anchor` (on screen), over the tab in front.
pub fn open_popup(dir: &str, anchor: RECT) {
    close_popup();
    let Some(e) = list().into_iter().find(|e| e.dir == dir) else { return };
    let (Some(popup), false) = (e.popup, e.id.is_empty()) else { return };
    let Some((from, _)) = crate::front_tab() else { return };
    let Some(main) = crate::window() else { return };
    register();
    let instance = unsafe { GetModuleHandleW(None).ok().map(|h| h.into()) };
    let side = crate::scale(POPUP_LEAST, crate::dpi());
    // Owned by the window: above it, gone with it. Hidden until it has its size.
    let Ok(window) = (unsafe {
        CreateWindowExW(WS_EX_TOOLWINDOW, w!("SearchWinExtPopup"), &HSTRING::from(e.name.as_str()), WS_POPUP | WS_CLIPCHILDREN, anchor.left, anchor.bottom, side, side, Some(main), None, instance, None)
    }) else {
        return;
    };
    unsafe {
        let round = DWMWCP_ROUND;
        let dark = BOOL::from(crate::is_dark());
        let _ = DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
    }
    let url = format!("chrome-extension://{}/{}", e.id, popup);
    let Some(page) = page::peek(from, &url, window, RECT { left: 0, top: 0, right: side, bottom: side }) else {
        unsafe {
            let _ = DestroyWindow(window);
        }
        return;
    };
    with(|s| s.popup = Some(Popup { window, page, from, anchor, shown: false }));
}

pub fn close_popup() {
    let Some(popup) = with(|s| {
        s.closed_at = Some(std::time::Instant::now());
        s.popup.take()
    }) else {
        return;
    };
    page::discard(popup.page);
    unsafe {
        let _ = DestroyWindow(popup.window);
    }
}

/// What the popup's page says. True when it was the popup's, and nobody else's to hear.
pub fn page_event(id: TabId, event: &page::Event) -> bool {
    let Some((window, from)) = with(|s| s.popup.as_ref().filter(|p| p.page == id).map(|p| (p.window, p.from))) else { return false };
    match event {
        page::Event::Loaded(_) => {
            measure();
            unsafe {
                SetTimer(Some(window), REMEASURE, REMEASURE_MS, None);
            }
        }
        // Its links open as tabs, after the one it is over.
        page::Event::NewWindow { url, .. } => {
            close_later(window);
            crate::open_link(url, false, from, false);
        }
        _ => {}
    }
    true
}

/// window.close() in a page, or Esc: the popup's closes it.
pub fn close_requested(id: TabId) -> bool {
    let Some(window) = with(|s| s.popup.as_ref().filter(|p| p.page == id).map(|p| p.window)) else { return false };
    close_later(window);
    true
}

/// Closed once the page's own event is over: a page is never closed from inside one.
fn close_later(window: HWND) {
    unsafe {
        let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn measure() {
    let Some(page) = with(|s| s.popup.as_ref().map(|p| p.page)) else { return };
    page::run(page, MEASURE, |result| {
        let size: Option<(i32, i32)> = result.ok().and_then(|text| serde_json::from_str(&text).ok());
        if let Some((w, h)) = size {
            fit(w, h);
        }
    });
}

/// The popup at the page's size, under its button and on its screen, then shown.
fn fit(w: i32, h: i32) {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect};
    let Some((window, page, anchor, shown)) = with(|s| s.popup.as_ref().map(|p| (p.window, p.page, p.anchor, p.shown))) else { return };
    let dpi = crate::dpi();
    let w = crate::scale(w.clamp(POPUP_LEAST, POPUP_MOST.0), dpi);
    let h = crate::scale(h.clamp(POPUP_LEAST, POPUP_MOST.1), dpi);
    let mut room = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST), &mut room);
    }
    let work = room.rcWork;
    // Its right edge under the button's, as Chrome hangs it, kept on the screen.
    let left = (anchor.right - w).clamp(work.left, (work.right - w).max(work.left));
    let top = anchor.bottom.min(work.bottom - h).max(work.top);
    unsafe {
        let _ = SetWindowPos(window, None, left, top, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    page::place_elsewhere(page, RECT { left: 0, top: 0, right: w, bottom: h });
    page::moved_elsewhere(page);
    if !shown {
        with(|s| s.popup.as_mut().map(|p| p.shown = true));
        unsafe {
            let _ = ShowWindow(window, SW_SHOW);
            let _ = SetForegroundWindow(window);
        }
        page::focus_page(page);
    }
}

fn register() {
    if with(|s| std::mem::replace(&mut s.registered, true)) {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(popup_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("SearchWinExtPopup"),
            ..Default::default()
        });
    }
}

extern "system" fn popup_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Anywhere else clicked: it goes, as Chrome's does.
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            if with(|s| s.popup.as_ref().is_some_and(|p| p.window == window && p.shown)) {
                close_popup();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if with(|s| s.popup.as_ref().is_some_and(|p| p.window == window)) {
                close_popup();
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == REMEASURE => {
            unsafe {
                let _ = KillTimer(Some(window), REMEASURE);
            }
            measure();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// The extensions as a script sees them.
pub fn bench() -> Value {
    let popup = with(|s| s.popup.as_ref().map(|p| (p.page, p.window, p.shown)));
    let mut value = state();
    value["popups"] = json!(popups().into_iter().map(|(dir, _)| dir).collect::<Vec<_>>());
    if let Some((page, window, shown)) = popup {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(window, &mut r);
        }
        value["popup"] = json!({ "page": page, "shown": shown, "now": page::address_and_title(page).map(|(u, _)| u), "rect": [r.left, r.top, r.right - r.left, r.bottom - r.top] });
    }
    value
}

/// Runs a script in the open popup's page (bench only).
pub fn eval_popup(js: &str, done: impl FnOnce(std::result::Result<String, String>) + 'static) {
    match with(|s| s.popup.as_ref().map(|p| p.page)) {
        Some(page) => page::run(page, js, done),
        None => done(Err("no popup".into())),
    }
}
