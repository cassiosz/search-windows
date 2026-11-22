//! The floating video (upstream Float.swift): a video that keeps playing
//! after you have gone somewhere else, in a small window that stays above
//! everything, other apps included. Ctrl+Shift+P lifts it out and puts it back.
//!
//! WebView2 has no picture in picture of its own to hand a video to, so the
//! page itself is moved, as upstream does: everything but the video is made
//! invisible (assets/js/float-on.js), the video fills the viewport, and the
//! tab's whole page is given another parent window, a small borderless one
//! that stays on top. The video never stops, because it is the same page it
//! always was; it has only changed windows. Back in its tab, the page is as
//! it was (float-off.js).
//!
//! Nothing here exists until the first Ctrl+Shift+P: no window, no class, no
//! script in any page. With "Video follows you to another app" on, leaving
//! the app asks the page in front whether a video plays, and only then.

use std::cell::RefCell;

use serde_json::{Value, json};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, w};

use crate::page::{self, TabId};

const ON: &str = include_str!("../assets/js/float-on.js");
const OFF: &str = include_str!("../assets/js/float-off.js");

// In device-independent pixels.
/// The black frame around the picture: where the window is taken to size it.
const EDGE: i32 = 6;
/// Along an edge this near a corner, the corner is what is taken.
const CORNER: i32 = 18;
/// The width it opens at the first time, and how far in from the screen's corner.
const WIDTH: i32 = 400;
const MARGIN: i32 = 16;
const SMALLEST: i32 = 200;
/// The pill of buttons over the top of the picture, while the pointer is on it.
const KNOB: i32 = 30;
const PILL_PAD: i32 = 3;
const PILL_TOP: i32 = 10;
const KNOB_GLYPH: i32 = 13;

/// Looks for the pointer while the window is up: the controls show only then.
const HOVER_TIMER: usize = 1;
const HOVER_MS: u32 = 150;

const GRIP_GLYPH: &str = "\u{E76F}";
const BACK_GLYPH: &str = "\u{E73F}";
const CLOSE_GLYPH: &str = "\u{E8BB}";

/// The controls' buttons, left to right: a handle to drag the window by,
/// back to the page, and put away.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    Grip,
    Back,
    Close,
}
const KNOBS: [Knob; 3] = [Knob::Grip, Knob::Back, Knob::Close];

#[derive(Default)]
struct Float {
    window: HWND,
    controls: HWND,
    tab: Option<TabId>,
    /// The picture's width over its height.
    aspect: f64,
    /// Where it was last, at the size it was: it comes back there this session.
    kept: Option<RECT>,
    /// Lifted because you went to another app: it goes home on its own when you come back.
    away: bool,
    /// The page is being asked whether a video plays.
    asking: bool,
    /// What the last Ctrl+Shift+P came to, for the bench.
    note: &'static str,
    hot: Option<Knob>,
    glyphs: HFONT,
    registered: bool,
}

thread_local! {
    static FLOAT: RefCell<Float> = RefCell::new(Float::default());
}

/// Never hold the borrow across a Win32 or WebView2 call: both come back in.
fn with<R>(f: impl FnOnce(&mut Float) -> R) -> R {
    FLOAT.with(|float| f(&mut float.borrow_mut()))
}

/// The tab whose page is out in the window, if one is.
pub fn tab() -> Option<TabId> {
    with(|f| f.tab)
}

/// Posted to the main window: the keys arrive inside a page's own event, and
/// its page is not moved under it (as WM_CLOSE_TAB).
pub const WM_FLOAT: u32 = WM_APP + 5;

/// Ctrl+Shift+P: out, or back, once the message loop gets to it.
pub fn toggle() {
    if let Some(main) = crate::window() {
        unsafe {
            let _ = PostMessageW(Some(main), WM_FLOAT, WPARAM(0), LPARAM(0));
        }
    }
}

/// On WM_FLOAT.
pub fn toggled() {
    if tab().is_some() {
        land();
    } else {
        lift(false);
    }
}

/// Esc pressed in page `from`: in the floating window's page, the video goes
/// back. Esc anywhere else is the page's.
pub fn escape(from: TabId) -> bool {
    if tab() != Some(from) {
        return false;
    }
    toggle();
    true
}

/// Another app in front: the video playing in front comes along, with
/// Settings › Video follows you to another app on (upstream's floatsAway).
pub fn app_left() {
    if !crate::internal::prefs(|p| p.float_away) || tab().is_some() {
        return;
    }
    // Still in view on another screen (a TV, a second display) while you work
    // on the one under the pointer: nothing to get out of the way of.
    if let Some(main) = crate::window() {
        let mut point = POINT::default();
        let elsewhere = unsafe {
            GetCursorPos(&mut point).is_ok()
                && !IsIconic(main).as_bool()
                && MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) != MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST)
        };
        if elsewhere {
            return;
        }
    }
    lift(true);
}

/// Back in the window, and still on the tab it came from: into the tab again.
/// Only a video that left on its own comes back on its own.
pub fn app_back() {
    let home = with(|f| f.away && f.tab.is_some() && f.tab == crate::front_tab().map(|(id, _)| id));
    if home {
        land();
    }
}

/// Asks the page in front for its video; with one playing, the page goes out.
/// `away`: lifted by leaving the app, not by hand.
fn lift(away: bool) {
    if with(|f| f.tab.is_some() || std::mem::replace(&mut f.asking, true)) {
        return;
    }
    let ready = crate::front_tab().and_then(|(id, url)| {
        let web = url.is_some_and(|u| ["http:", "https:", "file:"].iter().any(|s| u.get(..s.len()).is_some_and(|h| h.eq_ignore_ascii_case(s))));
        (web && page::has_page(id)).then_some(id)
    });
    let Some(id) = ready else {
        with(|f| {
            f.asking = false;
            f.note = "not a web page";
        });
        return;
    };
    page::run(id, ON, move |result| {
        let found = result.ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let found = found.filter(|v| v["floating"] == json!(true));
        with(|f| {
            f.asking = false;
            f.note = if found.is_some() { "floating" } else { "no video playing" };
        });
        let Some(found) = found else { return };
        let (w, h) = (found["w"].as_f64().unwrap_or(0.0), found["h"].as_f64().unwrap_or(0.0));
        let aspect = if w > 0.0 && h > 0.0 { (w / h).clamp(0.25, 4.0) } else { 16.0 / 9.0 };
        // Closed, slept or gone while it was asked: nothing to lift.
        if page::has_page(id) {
            open(id, aspect, away);
        } else {
            page::run(id, OFF, |_| {});
        }
    });
}

fn register() {
    if with(|f| std::mem::replace(&mut f.registered, true)) {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            // Black around and behind the picture, never a flash of anything else.
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: w!("SearchWinFloat"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(controls_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            lpszClassName: w!("SearchWinFloatControls"),
            ..Default::default()
        });
    }
}

/// The work area of the screen the main window is on.
fn work_area(near: HWND) -> RECT {
    unsafe {
        let monitor = MonitorFromWindow(near, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcWork
    }
}

/// Where the window opens: where it was last this session, at that width,
/// against the same corner, if a screen still shows it; otherwise the bottom
/// right of the main window's screen.
fn place(main: HWND, aspect: f64, dpi: i32) -> RECT {
    let edge = crate::scale(EDGE, dpi);
    let height_for = |width: i32| ((width - 2 * edge) as f64 / aspect).round() as i32 + 2 * edge;
    let kept = with(|f| f.kept).filter(|r| unsafe { !MonitorFromRect(r, MONITOR_DEFAULTTONULL).is_invalid() });
    if let Some(r) = kept {
        let area = unsafe {
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut info);
            info.rcWork
        };
        let width = r.right - r.left;
        let height = height_for(width);
        let low = (r.top + r.bottom) / 2 > (area.top + area.bottom) / 2;
        let top = if low { r.bottom - height } else { r.top };
        return RECT { left: r.left, top, right: r.right, bottom: top + height };
    }
    let area = work_area(main);
    let margin = crate::scale(MARGIN, dpi);
    let width = crate::scale(WIDTH, dpi).min((area.right - area.left) / 2);
    let height = height_for(width);
    RECT { left: area.right - margin - width, top: area.bottom - margin - height, right: area.right - margin, bottom: area.bottom - margin }
}

/// The picture's place in the window: all of it but the frame.
fn inner(window: HWND) -> RECT {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut client);
    }
    let edge = crate::scale(EDGE, dpi_of(window));
    RECT { left: edge, top: edge, right: (client.right - edge).max(edge), bottom: (client.bottom - edge).max(edge) }
}

fn dpi_of(window: HWND) -> i32 {
    (unsafe { GetDpiForWindow(window) } as i32).max(96)
}

fn open(id: TabId, aspect: f64, away: bool) {
    let Some(main) = crate::window() else { return };
    register();
    let dpi = dpi_of(main);
    let at = place(main, aspect, dpi);
    let window = unsafe {
        CreateWindowExW(
            // Above every ordinary window, this app's and everyone else's; never
            // in the taskbar; and a click on it leaves the app you are in in front.
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("SearchWinFloat"),
            w!("Video"),
            WS_POPUP | WS_CLIPCHILDREN,
            at.left,
            at.top,
            at.right - at.left,
            at.bottom - at.top,
            None,
            None,
            GetModuleHandleW(None).ok().map(|h| h.into()),
            None,
        )
    };
    let Ok(window) = window else { return };
    let controls = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("SearchWinFloatControls"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(window),
            None,
            GetModuleHandleW(None).ok().map(|h| h.into()),
            None,
        )
    }
    .unwrap_or_default();
    unsafe {
        let round = DWMWCP_ROUND;
        let dark = BOOL::from(true);
        for w in [window, controls] {
            let _ = DwmSetWindowAttribute(w, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
            let _ = DwmSetWindowAttribute(w, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        }
    }
    let glyphs = unsafe { crate::make_glyph_font(dpi, KNOB_GLYPH) };
    let old = with(|f| {
        f.window = window;
        f.controls = controls;
        f.tab = Some(id);
        f.aspect = aspect;
        f.away = away;
        f.hot = None;
        std::mem::replace(&mut f.glyphs, glyphs)
    });
    unsafe {
        if !old.is_invalid() {
            let _ = DeleteObject(old.into());
        }
    }
    // The page is in before the window shows: it opens on the video, not on black.
    if !page::lift(id, window, inner(window)) {
        close_windows();
        return;
    }
    unsafe {
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        SetTimer(Some(window), HOVER_TIMER, HOVER_MS, None);
    }
    crate::repaint_card();
}

/// Back into its tab, and the window closes.
pub fn land() {
    let Some(id) = with(|f| f.tab) else { return };
    page::bring_home(id);
    close_windows();
    page::run(id, OFF, |_| {});
    crate::repaint_card();
}

/// The control at the top of the window: the video back in its tab, and that
/// tab in front, in the window, which comes back up if it was minimized.
fn back() {
    let Some(id) = tab() else { return };
    land();
    crate::select_tab(id);
    if let Some(main) = crate::window() {
        unsafe {
            if IsIconic(main).as_bool() {
                let _ = ShowWindow(main, SW_RESTORE);
            }
            let _ = SetWindowPos(main, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        }
    }
}

/// Tab `id`'s page went (the tab closed or slept): the window goes with it.
pub fn page_gone(id: TabId) {
    if tab() == Some(id) {
        close_windows();
        crate::repaint_card();
    }
}

/// The main window is closing: nothing of ours stays on screen after it.
pub fn close_all() {
    close_windows();
}

/// Its place kept for next time, then gone.
fn close_windows() {
    let (window, controls) = with(|f| {
        f.tab = None;
        f.away = false;
        f.hot = None;
        (std::mem::take(&mut f.window), std::mem::take(&mut f.controls))
    });
    if window.is_invalid() {
        return;
    }
    let mut kept = RECT::default();
    unsafe {
        if GetWindowRect(window, &mut kept).is_ok() {
            with(|f| f.kept = Some(kept));
        }
        let _ = KillTimer(Some(window), HOVER_TIMER);
        if !controls.is_invalid() {
            let _ = DestroyWindow(controls);
        }
        let _ = DestroyWindow(window);
    }
}

/// The window, as a script sees it: whether it is up, whose page it holds,
/// where on screen, and what the last Ctrl+Shift+P came to.
pub fn bench() -> Value {
    let (window, controls, tab, away, note) = with(|f| (f.window, f.controls, f.tab, f.away, f.note));
    let mut value = json!({ "open": tab.is_some(), "tab": tab, "away": away, "note": note });
    if !window.is_invalid() {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(window, &mut r);
            let shown = |w: HWND| !w.is_invalid() && IsWindowVisible(w).as_bool();
            value["rect"] = json!([r.left, r.top, r.right - r.left, r.bottom - r.top]);
            value["visible"] = json!(shown(window));
            value["controls_shown"] = json!(shown(controls));
            let ex = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
            value["topmost"] = json!(ex & WS_EX_TOPMOST.0 != 0);
        }
    }
    value
}

/// Which way the window is being sized from its frame, if the point is on it.
fn edge_hit(window: HWND, x: i32, y: i32) -> u32 {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(window, &mut r);
    }
    let dpi = dpi_of(window);
    let (edge, corner) = (crate::scale(EDGE, dpi), crate::scale(CORNER, dpi));
    let (x, y) = (x - r.left, y - r.top);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let (left, right, top, bottom) = (x < edge, x >= w - edge, y < edge, y >= h - edge);
    let (near_left, near_right, near_top, near_bottom) = (x < corner, x >= w - corner, y < corner, y >= h - corner);
    if (top && near_left) || (left && near_top) {
        HTTOPLEFT
    } else if (top && near_right) || (right && near_top) {
        HTTOPRIGHT
    } else if (bottom && near_left) || (left && near_bottom) {
        HTBOTTOMLEFT
    } else if (bottom && near_right) || (right && near_bottom) {
        HTBOTTOMRIGHT
    } else if left {
        HTLEFT
    } else if right {
        HTRIGHT
    } else if top {
        HTTOP
    } else if bottom {
        HTBOTTOM
    } else {
        HTCAPTION
    }
}

/// Sized by an edge or a corner, the picture keeps its shape: the side being
/// pulled leads, and the other follows.
fn keep_shape(rect: &mut RECT, side: u32, aspect: f64, edge: i32) {
    let width = (rect.right - rect.left - 2 * edge).max(1) as f64;
    let height = (rect.bottom - rect.top - 2 * edge).max(1) as f64;
    match side {
        WMSZ_TOP | WMSZ_BOTTOM => rect.right = rect.left + (height * aspect).round() as i32 + 2 * edge,
        WMSZ_TOPLEFT | WMSZ_TOPRIGHT => rect.top = rect.bottom - (width / aspect).round() as i32 - 2 * edge,
        _ => rect.bottom = rect.top + (width / aspect).round() as i32 + 2 * edge,
    }
}

/// The controls, centred along the top of the picture.
fn place_controls() {
    let (window, controls) = with(|f| (f.window, f.controls));
    if window.is_invalid() || controls.is_invalid() {
        return;
    }
    let dpi = dpi_of(window);
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(window, &mut r);
        let width = KNOBS.len() as i32 * crate::scale(KNOB, dpi) + 2 * crate::scale(PILL_PAD, dpi);
        let height = crate::scale(KNOB, dpi) + 2 * crate::scale(PILL_PAD, dpi);
        let left = r.left + (r.right - r.left - width) / 2;
        let top = r.top + crate::scale(PILL_TOP, dpi);
        let _ = SetWindowPos(controls, None, left, top, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Shown while the pointer is over the window, as upstream: the whole point
/// of the window is the picture.
fn hover() {
    let (window, controls) = with(|f| (f.window, f.controls));
    if window.is_invalid() || controls.is_invalid() {
        return;
    }
    unsafe {
        let mut point = POINT::default();
        let mut r = RECT::default();
        let _ = GetCursorPos(&mut point);
        let _ = GetWindowRect(window, &mut r);
        let over = point.x >= r.left && point.x < r.right && point.y >= r.top && point.y < r.bottom;
        let shown = IsWindowVisible(controls).as_bool();
        if over && !shown {
            place_controls();
            let _ = ShowWindow(controls, SW_SHOWNOACTIVATE);
        } else if !over && shown {
            let _ = ShowWindow(controls, SW_HIDE);
        }
    }
}

extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            // The page covers the rest: only the frame is ever the window's own.
            WM_NCHITTEST => {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
                LRESULT(edge_hit(window, x, y) as isize)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SIZING => {
                let aspect = with(|f| f.aspect);
                if aspect > 0.0 {
                    keep_shape(&mut *(lparam.0 as *mut RECT), wparam.0 as u32, aspect, crate::scale(EDGE, dpi_of(window)));
                }
                LRESULT(1)
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let dpi = dpi_of(window);
                info.ptMinTrackSize = POINT { x: crate::scale(SMALLEST, dpi), y: crate::scale(SMALLEST * 9 / 16, dpi) };
                LRESULT(0)
            }
            WM_SIZE => {
                if let Some(id) = tab() {
                    page::place_elsewhere(id, inner(window));
                }
                place_controls();
                LRESULT(0)
            }
            WM_MOVE => {
                if let Some(id) = tab() {
                    page::moved_elsewhere(id);
                }
                place_controls();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == HOVER_TIMER => {
                hover();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(window, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

fn knob_at(controls: HWND, x: i32) -> Option<Knob> {
    let dpi = dpi_of(controls);
    let i = (x - crate::scale(PILL_PAD, dpi)).div_euclid(crate::scale(KNOB, dpi));
    usize::try_from(i).ok().and_then(|i| KNOBS.get(i).copied())
}

fn paint_controls(controls: HWND) {
    let (hot, glyphs) = with(|f| (f.hot, f.glyphs));
    let dpi = dpi_of(controls);
    unsafe {
        let mut area = RECT::default();
        let _ = GetClientRect(controls, &mut area);
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(controls, &mut paint);
        crate::buffered(hdc, area, |canvas, area| {
            // Dark over any picture, light or dark: white glyphs hold against it.
            let ground = (32, 33, 36);
            crate::fill(canvas, area, ground);
            let (pad, knob) = (crate::scale(PILL_PAD, dpi), crate::scale(KNOB, dpi));
            for (i, which) in KNOBS.iter().enumerate() {
                let left = pad + i as i32 * knob;
                let spot = RECT { left, top: pad, right: left + knob, bottom: pad + knob };
                if hot == Some(*which) && *which != Knob::Grip {
                    crate::rounded(canvas, spot, (64, 66, 71), (64, 66, 71), crate::scale(6, dpi));
                }
                let (glyph, ink) = match which {
                    Knob::Grip => (GRIP_GLYPH, (150, 152, 158)),
                    Knob::Back => (BACK_GLYPH, (240, 240, 242)),
                    Knob::Close => (CLOSE_GLYPH, (240, 240, 242)),
                };
                crate::text(canvas, glyph, spot, glyphs, ink, true);
            }
        });
        let _ = EndPaint(controls, &paint);
    }
}

extern "system" fn controls_proc(controls: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_controls(controls);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let knob = knob_at(controls, x);
                if with(|f| std::mem::replace(&mut f.hot, knob) != knob) {
                    let _ = InvalidateRect(Some(controls), None, false);
                }
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: controls, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
                LRESULT(0)
            }
            crate::WM_MOUSELEAVE => {
                if with(|f| f.hot.take().is_some()) {
                    let _ = InvalidateRect(Some(controls), None, false);
                }
                LRESULT(0)
            }
            // The handle, or anywhere off a button: the window goes where it is dragged.
            WM_LBUTTONDOWN if !matches!(knob_at(controls, x), Some(Knob::Back | Knob::Close)) => {
                let window = with(|f| f.window);
                let _ = ReleaseCapture();
                SendMessageW(window, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                match knob_at(controls, x) {
                    Some(Knob::Back) => back(),
                    Some(Knob::Close) => land(),
                    _ => {}
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(controls, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_keeps_the_pictures_shape() {
        let aspect = 16.0 / 9.0;
        // Pulled by the right side: the bottom follows.
        let mut r = RECT { left: 0, top: 0, right: 332, bottom: 100 };
        keep_shape(&mut r, WMSZ_RIGHT, aspect, 6);
        assert_eq!(r.bottom, 180 + 12);
        // By the bottom: the right side follows.
        let mut r = RECT { left: 0, top: 0, right: 100, bottom: 192 };
        keep_shape(&mut r, WMSZ_BOTTOM, aspect, 6);
        assert_eq!(r.right, 320 + 12);
        // By the top left corner: the top moves, the bottom stays.
        let mut r = RECT { left: 0, top: 50, right: 332, bottom: 400 };
        keep_shape(&mut r, WMSZ_TOPLEFT, aspect, 6);
        assert_eq!((r.top, r.bottom), (400 - 192, 400));
    }
}
