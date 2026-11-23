//! A peek at a link (upstream Peek.swift), Arc's way: a modified click opens
//! its page in a panel over the one you are reading, which stays where it
//! was underneath, dimmed, and doesn't react to the pointer. Esc, a click
//! beside the panel or its cross puts it away; its other button keeps it, as
//! a tab beside this one, loaded as it is.
//!
//! Settings turns peeking on (off until then) and chooses the click:
//! Shift-click, or Alt-click.
//!
//! The panel's page is a page of its own, made in the same store as the tab
//! it came from, and not a tab: keeping it is giving it to a new tab, with
//! nothing loaded twice (page::rename).
//!
//! The click is noticed in the page, by a document-start script given only
//! while Peek is on, which posts through the channel any web page can post
//! on. So what it says is never taken on trust: it is heard only from a
//! document that was given the script (page.rs), only from a tab in the row,
//! only while Peek is on, and only for an http or https address. A page
//! forging it can at worst open a peek of a web page, which it could as well
//! have opened as a pop-up.

use std::cell::RefCell;

use serde_json::{Value, json};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, w};

use crate::page::{self, TabId};

/// What a document was given, as page.rs keeps it, beside veil's: the click listener.
pub const PEEK_IN: u8 = 8;

/// Posted to the main window: a peek opens, closes or is kept from the
/// message loop, never inside a page's own event (as WM_CLOSE_TAB).
pub const WM_PEEK: u32 = WM_APP + 6;
const OPEN: usize = 0;
const CLOSE: usize = 1;
const KEEP: usize = 2;

/// The longest address a page may hand over.
const URL_MAX: usize = 4096;

// In device-independent pixels.
/// The panel's strip along the top: where it came from, and its two buttons.
const HEAD: i32 = 40;
const KNOB: i32 = 30;
const KNOB_GAP: i32 = 4;
const HEAD_PAD: i32 = 8;
/// The page under the panel, darkened this much (upstream's 0.22).
const DIM_ALPHA: u8 = 56;

const KEEP_GLYPH: &str = "\u{E8A7}";
const CLOSE_GLYPH: &str = "\u{E8BB}";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    Keep,
    Close,
}

#[derive(Default)]
struct Peek {
    dim: HWND,
    panel: HWND,
    /// The panel's page.
    page: Option<TabId>,
    /// The tab it was opened from.
    from: TabId,
    url: String,
    /// Asked for by a page, opened when the message loop gets to it.
    asked: Option<(TabId, String)>,
    hot: Option<Knob>,
    /// Why the last page's ask was turned down, for the bench.
    refused: &'static str,
    registered: bool,
}

thread_local! {
    static PEEK: RefCell<Peek> = RefCell::new(Peek::default());
}

/// Never hold the borrow across a Win32 or WebView2 call: both come back in.
fn with<R>(f: impl FnOnce(&mut Peek) -> R) -> R {
    PEEK.with(|peek| f(&mut peek.borrow_mut()))
}

/// The listener every web page starts with while Peek is on; empty, and so
/// nothing at all, while it is off. Only the top document of a website, only
/// the user's own click (isTrusted), only the click chosen, only a link to a
/// web page: anything else is the page's as always.
pub fn start_script() -> String {
    let (on, with) = crate::internal::prefs(|p| (p.peek, p.peek_with));
    if !on {
        return String::new();
    }
    let (held, not) = match with {
        searchwin_prefs::PeekWith::Shift => ("shiftKey", "altKey"),
        searchwin_prefs::PeekWith::Alt => ("altKey", "shiftKey"),
    };
    format!(
        "(()=>{{if(window!==window.top||!/^https?:$/.test(location.protocol)||!window.chrome||!chrome.webview)return;\
         addEventListener('click',e=>{{if(!e.isTrusted||e.button!==0||!e.{held}||e.{not}||e.ctrlKey||e.metaKey)return;\
         for(const n of e.composedPath()){{if(n.tagName!=='A'||typeof n.href!=='string'||!n.href)continue;\
         if(!/^https?:/i.test(n.href))return;e.preventDefault();e.stopImmediatePropagation();\
         chrome.webview.postMessage({{op:'peek.open',url:n.href}});return}}}},true)}})()"
    )
}

/// A message from a web page in tab `id`, which page.rs has checked holds the
/// listener. False if it isn't Peek's, to be heard by whoever it is for.
pub fn message(id: TabId, json: &str) -> bool {
    let Ok(asked) = serde_json::from_str::<Value>(json) else { return false };
    if asked.get("op").and_then(Value::as_str) != Some("peek.open") {
        return false;
    }
    let url = asked.get("url").and_then(Value::as_str).unwrap_or_default();
    let refused = if !crate::internal::prefs(|p| p.peek) {
        Some("peek is off")
    } else if !crate::has_tab(id) {
        Some("not from a tab")
    } else if !peekable(url) {
        Some("not a web address")
    } else {
        None
    };
    with(|p| p.refused = refused.unwrap_or(""));
    if refused.is_none() {
        with(|p| p.asked = Some((id, url.to_string())));
        post(OPEN, 0);
    }
    true
}

/// What may open in a peek: a web page, http or https with a host, never
/// about:, data:, file:, javascript: or anything else a page can name.
fn peekable(url: &str) -> bool {
    url.len() <= URL_MAX
        && url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty()))
}

fn post(what: usize, page: TabId) {
    if let Some(main) = crate::window() {
        unsafe {
            let _ = PostMessageW(Some(main), WM_PEEK, WPARAM(what), LPARAM(page as isize));
        }
    }
}

/// On WM_PEEK.
pub fn act(what: WPARAM, page: LPARAM) {
    match what.0 {
        OPEN => {
            if let Some((from, url)) = with(|p| p.asked.take()) {
                open(from, &url);
            }
        }
        // Only the peek it was meant for: one opened since stays.
        CLOSE if with(|p| p.page) == Some(page.0 as TabId) => close_now(),
        KEEP if with(|p| p.page) == Some(page.0 as TabId) => keep_now(),
        _ => {}
    }
}

pub fn is_open() -> bool {
    with(|p| p.page.is_some())
}

/// Puts it away (Esc, a click beside it, its cross, another tab).
pub fn close() {
    if let Some(page) = with(|p| p.page) {
        post(CLOSE, page);
    }
}

/// Esc with a peek up: it goes. False with none.
pub fn escape() -> bool {
    let open = is_open();
    close();
    open
}

/// Ctrl+Enter with a peek up (upstream's ⌘↩): kept as a tab. False with none.
pub fn keep() -> bool {
    match with(|p| p.page) {
        Some(page) => {
            post(KEEP, page);
            true
        }
        None => false,
    }
}

fn register() {
    if with(|p| std::mem::replace(&mut p.registered, true)) {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(dim_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: w!("SearchWinPeekDim"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(panel_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            lpszClassName: w!("SearchWinPeek"),
            ..Default::default()
        });
    }
}

/// Where everything goes, on screen: the dimming over the page, the panel
/// in the middle of it (upstream's 82% by 86%), and the page in the panel,
/// under its strip.
fn spots() -> Option<(RECT, RECT, RECT)> {
    let card = crate::card_on_screen()?;
    let dpi = crate::dpi();
    let (w, h) = (card.right - card.left, card.bottom - card.top);
    let (pw, ph) = (w * 82 / 100, h * 86 / 100);
    let left = card.left + (w - pw) / 2;
    let top = card.top + (h - ph) / 2;
    let panel = RECT { left, top, right: left + pw, bottom: top + ph };
    let page = RECT { left: 0, top: crate::scale(HEAD, dpi), right: pw, bottom: ph };
    Some((card, panel, page))
}

fn open(from: TabId, url: &str) {
    // One at a time: a peek from a peek's opener replaces it.
    if with(|p| p.page.is_some()) {
        close_now();
    }
    let Some(main) = crate::window() else { return };
    let Some((card, at, inside)) = spots() else { return };
    register();
    let instance = unsafe { GetModuleHandleW(None).ok().map(|h| h.into()) };
    // Owned by the window: above its pages, gone with it, hidden with it when minimized.
    let dim = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            w!("SearchWinPeekDim"),
            w!(""),
            WS_POPUP,
            card.left,
            card.top,
            card.right - card.left,
            card.bottom - card.top,
            Some(main),
            None,
            instance,
            None,
        )
    };
    let Ok(dim) = dim else { return };
    let panel = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("SearchWinPeek"),
            w!("Peek"),
            WS_POPUP | WS_CLIPCHILDREN,
            at.left,
            at.top,
            at.right - at.left,
            at.bottom - at.top,
            Some(dim),
            None,
            instance,
            None,
        )
    };
    let Ok(panel) = panel else {
        unsafe {
            let _ = DestroyWindow(dim);
        }
        return;
    };
    let dark = crate::is_dark();
    unsafe {
        let _ = SetLayeredWindowAttributes(dim, COLORREF(0), DIM_ALPHA, LWA_ALPHA);
        let round = DWMWCP_ROUND;
        let dark = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(panel, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(panel, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
    }
    with(|p| {
        p.dim = dim;
        p.panel = panel;
        p.from = from;
        p.url = url.to_string();
        p.hot = None;
    });
    let Some(page) = page::peek(from, url, panel, inside) else {
        destroy(dim);
        return;
    };
    with(|p| p.page = Some(page));
    // The page underneath dims as the panel rises into place, a sheet over
    // it: both start where they rise from before they show.
    crate::motion::fade(dim, 0, DIM_ALPHA, crate::motion::SHEET_MS);
    let rise = crate::scale(16, crate::dpi());
    crate::motion::place(panel, Some(crate::motion::rising(at, rise)), at, crate::motion::SHEET_MS);
    unsafe {
        let _ = ShowWindow(dim, SW_SHOWNOACTIVATE);
        let _ = ShowWindow(panel, SW_SHOW);
    }
}

fn destroy(dim: HWND) {
    with(|p| {
        p.page = None;
        p.dim = HWND::default();
        p.panel = HWND::default();
        p.hot = None;
    });
    // The panel is the dimming's: it goes with it.
    unsafe {
        let _ = DestroyWindow(dim);
    }
}

fn close_now() {
    let (page, dim) = with(|p| (p.page, p.dim));
    if let Some(page) = page {
        page::discard(page);
    }
    if !dim.is_invalid() {
        destroy(dim);
    }
}

/// The peek, kept: a tab after the one it came from, in front, its page as it is.
fn keep_now() {
    let Some((page, from, url, dim)) = with(|p| Some((p.page?, p.from, p.url.clone(), p.dim))) else { return };
    let (url, title) = page::address_and_title(page).filter(|(u, _)| !u.is_empty()).unwrap_or((url, String::new()));
    let Some(tab) = crate::open_kept(from, &url, &title) else { return close_now() };
    page::rename(page, tab);
    // Into the window before the panel goes, or the page would go with it.
    page::bring_home(tab);
    destroy(dim);
    crate::show_selected();
}

/// The window moved or changed size: the peek stays over its page.
pub fn place() {
    let (dim, panel, page) = with(|p| (p.dim, p.panel, p.page));
    let Some(page) = page else { return };
    let Some((card, at, inside)) = spots() else { return };
    unsafe {
        let _ = SetWindowPos(dim, None, card.left, card.top, card.right - card.left, card.bottom - card.top, SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = SetWindowPos(panel, None, at.left, at.top, at.right - at.left, at.bottom - at.top, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    // The window moved: the panel follows at once, not from where it was rising.
    crate::motion::place(panel, None, at, 0);
    page::place_elsewhere(page, inside);
    page::moved_elsewhere(page);
}

/// The light or dark changed.
pub fn repaint() {
    let panel = with(|p| p.panel);
    if !panel.is_invalid() {
        let dark = BOOL::from(crate::is_dark());
        unsafe {
            let _ = DwmSetWindowAttribute(panel, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
            let _ = InvalidateRect(Some(panel), None, false);
        }
    }
}

/// The peek as a script sees it.
pub fn bench() -> Value {
    let (page, from, url, dim, panel, refused) = with(|p| (p.page, p.from, p.url.clone(), p.dim, p.panel, p.refused));
    let mut value = json!({ "open": page.is_some(), "refused": refused });
    if let Some(page) = page {
        let rect = |w: HWND| {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(w, &mut r);
            }
            json!([r.left, r.top, r.right - r.left, r.bottom - r.top])
        };
        value["from"] = json!(from);
        value["url"] = json!(url);
        value["has_page"] = json!(page::has_page(page));
        value["now"] = json!(page::address_and_title(page).map(|(u, _)| u));
        value["dim"] = rect(dim);
        value["panel"] = rect(panel);
        value["dim_shown"] = json!(unsafe { IsWindowVisible(dim).as_bool() });
    }
    value
}

/// The strip's buttons, right to left from its end: close, then keep.
fn knob_rect(panel: HWND, which: Knob) -> RECT {
    let dpi = crate::dpi();
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(panel, &mut client);
    }
    let (knob, gap, pad, head) = (crate::scale(KNOB, dpi), crate::scale(KNOB_GAP, dpi), crate::scale(HEAD_PAD, dpi), crate::scale(HEAD, dpi));
    let top = (head - knob) / 2;
    let right = client.right - pad - if which == Knob::Keep { knob + gap } else { 0 };
    RECT { left: right - knob, top, right, bottom: top + knob }
}

fn knob_at(panel: HWND, x: i32, y: i32) -> Option<Knob> {
    [Knob::Keep, Knob::Close].into_iter().find(|k| crate::inside(knob_rect(panel, *k), x, y))
}

fn paint(panel: HWND) {
    let Some(look) = crate::look() else { return };
    let (hot, url) = with(|p| (p.hot, p.url.clone()));
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or(url);
    let dpi = crate::dpi();
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(panel, &mut client);
        let strip = RECT { bottom: crate::scale(HEAD, dpi), ..client };
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(panel, &mut paint);
        crate::buffered(hdc, strip, |canvas, area| {
            crate::fill(canvas, area, look.palette.card);
            crate::fill(canvas, RECT { top: area.bottom - 1, ..area }, look.palette.border);
            let keep = knob_rect(panel, Knob::Keep);
            let words = RECT { left: area.left + crate::scale(14, dpi), right: keep.left - crate::scale(8, dpi), ..area };
            crate::text(canvas, &host, words, look.crumb_font, look.palette.dim, false);
            for which in [Knob::Keep, Knob::Close] {
                let spot = knob_rect(panel, which);
                if hot == Some(which) {
                    crate::rounded(canvas, spot, look.palette.bar_hover, look.palette.bar_hover, crate::scale(6, dpi));
                }
                let glyph = if which == Knob::Keep { KEEP_GLYPH } else { CLOSE_GLYPH };
                crate::text(canvas, glyph, spot, look.glyphs, look.palette.text, true);
            }
        });
        // Under the page, seen only before it has drawn.
        let below = RECT { top: strip.bottom, ..client };
        if below.bottom > below.top {
            crate::fill(hdc, below, look.palette.card);
        }
        let _ = EndPaint(panel, &paint);
    }
}

extern "system" fn dim_proc(dim: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            // A click beside the panel puts it away, and the page under it never hears it.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                close();
                LRESULT(0)
            }
            _ => DefWindowProcW(dim, message, wparam, lparam),
        }
    }
}

extern "system" fn panel_proc(panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            WM_ERASEBKGND => LRESULT(1),
            // Rising into place: the page's pop-ups (a <select>) open where it is now.
            WM_MOVE => {
                if let Some(page) = with(|p| p.page) {
                    page::moved_elsewhere(page);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                paint(panel);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let knob = knob_at(panel, x, y);
                if with(|p| std::mem::replace(&mut p.hot, knob) != knob) {
                    let _ = InvalidateRect(Some(panel), None, false);
                }
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: panel, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
                LRESULT(0)
            }
            crate::WM_MOUSELEAVE => {
                if with(|p| p.hot.take().is_some()) {
                    let _ = InvalidateRect(Some(panel), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                match knob_at(panel, x, y) {
                    Some(Knob::Keep) => {
                        keep();
                    }
                    Some(Knob::Close) => close(),
                    None => {}
                }
                LRESULT(0)
            }
            // Esc and Ctrl+Enter typed while the strip has the keys reach the
            // window's shortcuts from the message loop, as every key does.
            _ => DefWindowProcW(panel, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_are_peeked_at() {
        assert!(peekable("https://example.com/a?b#c"));
        assert!(peekable("http://localhost:8771/x"));
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///C:/Windows/win.ini",
            "data:text/html,<b>x</b>",
            "about:blank",
            "search://settings",
            "blob:https://example.com/1",
            "https://",
        ] {
            assert!(!peekable(bad), "{bad}");
        }
        assert!(!peekable(&format!("https://example.com/{}", "a".repeat(URL_MAX))));
    }

    #[test]
    fn a_message_that_isnt_peeks_is_left_for_others() {
        assert!(!message(1, r##"{"op":"curtain.pick","selector":"#x"}"##));
        assert!(!message(1, "not json"));
    }
}
