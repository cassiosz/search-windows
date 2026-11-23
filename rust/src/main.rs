//! Search for Windows (Rust spike): one window, one field, the tabs, and the page.
//! Plain Win32 for the frame, WebView2 for the page, nothing else loaded.
//!
//! One surface, no title bar: the tabs run down the left the whole height of
//! the window, and the page has a bar above it (back, forward, reload, where
//! you are, and the window's own buttons), as upstream's roadmap draws it.

#![windows_subsystem = "windows"]

mod address;
mod bookmarks;
mod downloads;
mod engine;
mod favicon;
mod float;
mod forms;
mod import;
mod internal;
mod omnibox;
mod drive;
mod page;
mod passwords;
mod peek;
mod shield;
mod spaces;
mod split;
mod veil;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use searchwin_history::{History, Suggestion, now_ms};
use searchwin_tabs::{Shape, SleepPace, Tab, Tabs};
use windows::Win32::Foundation::*;
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{BOOL, HSTRING, PCWSTR, w};

// Layout, in device-independent pixels.
/// The tabs' side of the window, top to bottom.
const COLUMN: i32 = 250;
/// The bar above the page; the column's top strip is as tall, so the two line up.
const BAR: i32 = 44;
/// Rows sit this far in from the column's edges.
const SIDE_PAD: i32 = 10;
const GAP: i32 = 8;
const RADIUS: i32 = 8;
const FONT: i32 = 15;
const SMALL_FONT: i32 = 11;
/// The page's place in the bar, a little smaller than the tabs: it is a label, not a list.
const CRUMB_FONT: i32 = 14;
/// Glyphs from Segoe Fluent Icons (Segoe MDL2 Assets before Windows 11).
const GLYPH: i32 = 16;
const CAPTION_GLYPH: i32 = 10;
const LINE: i32 = 20;
/// The address field, when it is out: a box in the bar.
const FIELD_H: i32 = 32;
const FIELD_PAD: i32 = 10;
/// Wider than this, an address is hard to read across.
const FIELD_MOST: i32 = 680;
/// Back, forward, reload: square buttons.
const NAV: i32 = 36;
/// Minimize, maximize, close: as wide as Windows' own.
const CAPTION_W: i32 = 46;
/// Pins: big tiles, three to a line.
const PIN_H: i32 = 52;
const PIN_GAP: i32 = 8;
const PINS_PER_LINE: i32 = 3;
const PIN_ICON: i32 = 20;
const ROW: i32 = 44;
const ICON: i32 = 16;
/// A suggestion is two lines: its title, and the address under it.
const SUGGESTION_ROW: i32 = 48;
/// As many as the drop-down shows without a scroll bar.
const SUGGESTIONS: usize = 8;
const DROP_PAD: i32 = 6;

// Segoe Fluent Icons / MDL2 code points.
const BACK_GLYPH: &str = "\u{E72B}";
const FORWARD_GLYPH: &str = "\u{E72A}";
const RELOAD_GLYPH: &str = "\u{E72C}";
const ADD_GLYPH: &str = "\u{E710}";
const MINIMIZE_GLYPH: &str = "\u{E921}";
const MAXIMIZE_GLYPH: &str = "\u{E922}";
const RESTORE_GLYPH: &str = "\u{E923}";
const CLOSE_GLYPH: &str = "\u{E8BB}";

// Timers. The bench's own is drive::WAIT_TIMER.
const SLEEP_TIMER: usize = 1;
const SAVE_TIMER: usize = 2;
/// Tabs sleep after half an hour or more: a look once a minute is plenty.
const SLEEP_CHECK_MS: u32 = 60_000;
/// Changes come in bursts (an address, then a title): one write once they settle.
const SAVE_AFTER_MS: u32 = 1_500;
/// A tab closes from the message loop, never inside its own page's event.
const WM_CLOSE_TAB: u32 = WM_APP + 2;

// Edit control constants not worth a feature flag.
const ES_AUTOHSCROLL: u32 = 0x0080;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;
const EN_CHANGE: u32 = 0x0300;
/// The mouse left the window, as asked for with TrackMouseEvent.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy)]
struct Palette {
    /// The tabs' column: a calm tint, so the page is what stands out.
    background: Rgb,
    /// The page and the bar above it.
    card: Rgb,
    /// The address field's box, in the bar.
    field: Rgb,
    /// The tab in front: a raised pill.
    pill: Rgb,
    /// Pins: tiles a step above the column.
    tile: Rgb,
    /// Hairlines: pills, tiles, the column's edge, the field.
    border: Rgb,
    /// The pill's soft shadow, one pixel under it.
    shadow: Rgb,
    text: Rgb,
    /// Under the mouse, in the column.
    hover: Rgb,
    /// Under the mouse, in the bar.
    bar_hover: Rgb,
    /// Tabs not in front.
    dim: Rgb,
    /// Asleep tabs, and the new-tab row: there, but quiet.
    faint: Rgb,
    /// The page with the keys, when two or more are side by side: the pages' own accent.
    accent: Rgb,
}

/// `a` moved `percent` of the way to `b`.
fn mix(a: Rgb, b: Rgb, percent: u32) -> Rgb {
    let one = |x: u8, y: u8| ((x as u32 * (100 - percent) + y as u32 * percent) / 100) as u8;
    (one(a.0, b.0), one(a.1, b.1), one(a.2, b.2))
}

impl Palette {
    fn new(dark: bool) -> Self {
        // Light: a cool grey with a breath of blue beside a white page.
        // Dark: near-black beside a page a step lighter.
        let (background, card, pill, text) = if dark {
            ((30, 31, 34), (40, 42, 46), (56, 58, 64), (236, 237, 240))
        } else {
            ((221, 227, 234), (255, 255, 255), (255, 255, 255), (24, 26, 30))
        };
        let toward_ink = |percent| mix(background, text, percent);
        Palette {
            background,
            card,
            field: if dark { (52, 54, 59) } else { (240, 243, 247) },
            pill,
            tile: mix(background, pill, if dark { 45 } else { 50 }),
            border: if dark { toward_ink(16) } else { toward_ink(12) },
            shadow: if dark { (20, 21, 23) } else { toward_ink(9) },
            text,
            hover: if dark { mix(background, pill, 50) } else { toward_ink(6) },
            bar_hover: mix(card, text, 8),
            // Both stay above 4.5:1 on the column (see the test below).
            dim: mix(text, background, 20),
            faint: mix(text, background, 35),
            accent: if dark { (76, 194, 255) } else { (0, 103, 192) },
        }
    }
}

/// The system's light or dark, unless SEARCHWIN_THEME (a test hook, "dark" or
/// "light") says otherwise: a capture of either never touches your settings.
fn theme_dark() -> bool {
    match std::env::var("SEARCHWIN_THEME").as_deref() {
        Ok("dark") => true,
        Ok("light") => false,
        _ => system_dark(),
    }
}

fn color((r, g, b): Rgb) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

/// What a spot in the column is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Pin(u64),
    Tab(u64),
    NewTab,
    /// A space's chip at the column's foot, by its place among them.
    Space(usize),
    /// A row of the drop-down under the field.
    Suggestion(usize),
    // The bar's.
    Back,
    Forward,
    Reload,
    /// Where you are: a click turns it into the field.
    Crumb,
    Minimize,
    Maximize,
    Close,
}

struct State {
    window: HWND,
    field: HWND,
    field_proc: WNDPROC,
    /// The suggestions, under the field: a window of its own, as the page is
    /// one too and would cover anything drawn in ours.
    drop: HWND,
    font: HFONT,
    small_font: HFONT,
    crumb_font: HFONT,
    glyphs: HFONT,
    caption_glyphs: HFONT,
    field_brush: HBRUSH,
    dpi: i32,
    dark: bool,
    palette: Palette,
    timed: bool,
    /// Page ids are tab ids.
    tabs: Tabs,
    /// Read the first time it is needed: a launch never waits for it.
    history: Option<History>,
    /// Shown instead of the tabs while something is being typed.
    suggestions: Vec<Suggestion>,
    highlight: Option<usize>,
    hover: Option<Item>,
    /// The mouse is on the hovered row's ×.
    hover_close: bool,
    tracking_mouse: bool,
    /// Over the maximize button, which Windows sees as its own (for Snap Layouts).
    tracking_frame: bool,
    /// Minimize and close act when the button is let go over them, as Windows' own do.
    pressed: Option<Item>,
    /// Where the page's place in the bar ends, as last drawn: it is clicked there.
    crumb_right: i32,
    /// The drop-down's row under the mouse.
    drop_hover: Option<usize>,
    /// How far the tab rows are scrolled, in pixels.
    scroll: i32,
    /// The window is setting the field's text: that isn't typing.
    quiet: bool,
    session_owed: bool,
    history_owed: bool,
    /// Private tabs, and the InPrivate profile each keeps its cookies in. A
    /// link opened from one shares its profile; a new private tab gets its own.
    private: HashMap<u64, String>,
    privates_made: u64,
    /// Ctrl+K: the rows under the field are the open tabs, `switch_ids` says which.
    switching: bool,
    switch_ids: Vec<u64>,
    /// Profiles and spaces (spaces.json); with no file, one space, switched off.
    spaces: searchwin_spaces::Spaces,
    /// The rows of the spaces not on screen, read the first time each is wanted.
    rows: searchwin_spaces::Rows,
    /// The space `tabs` is the row of. Until the window follows a switch, it
    /// can differ from the model's current space for a moment.
    row_of: String,
    /// What deleted spaces and profiles left, cleared away once the window has moved off them.
    removals: Vec<searchwin_spaces::Removal>,
    /// Links sent to the space being switched to, opened when it is on screen.
    sent: Vec<String>,
    /// Split View: pages of `tabs` side by side. Empty with the setting off.
    splits: searchwin_split::Splits,
    /// A divider being dragged.
    drag: Option<searchwin_split::Drag>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Never hold the borrow across a Win32 call: messages re-enter the window procedure.
fn state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// The tabs, with their clock set: every change may stamp a time.
fn tabs<R>(f: impl FnOnce(&mut Tabs) -> R) -> Option<R> {
    state(|s| {
        s.tabs.set_now(now_ms());
        f(&mut s.tabs)
    })
}

fn history(s: &mut State) -> &mut History {
    // The profile on screen's own: Personal's is where it always was.
    s.history.get_or_insert_with(|| History::load(&spaces::folder().join("history.json")))
}

fn scale(value: i32, dpi: i32) -> i32 {
    value * dpi / 96
}

/// Everything the app keeps, the page engine's profile included. SEARCHWIN_DATA
/// puts it somewhere else, so a test run never writes into the one you use.
pub fn data_folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("SEARCHWIN_DATA").filter(|f| !f.is_empty()) {
        return PathBuf::from(folder);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("SearchWin-rs")
}

/// As upstream: a crash is written down on this PC and goes nowhere else.
pub fn crash_log(text: &str) {
    let folder = data_folder();
    let _ = std::fs::create_dir_all(&folder);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("crash.log")) {
        let _ = writeln!(file, "{text}\n");
    }
}

fn main() -> windows_core::Result<()> {
    std::panic::set_hook(Box::new(|info| crash_log(&info.to_string())));
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        // Before the page engine starts: a store a deletion left is removed while nothing holds it.
        let spaces = spaces::load();
        let class = WNDCLASSW {
            // Double clicks: a split's divider evens the pages out.
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            lpszClassName: w!("SearchWin"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let drop_class = WNDCLASSW {
            lpfnWndProc: Some(drop_proc),
            hInstance: instance.into(),
            hCursor: arrow,
            lpszClassName: w!("SearchWinDrop"),
            ..Default::default()
        };
        RegisterClassW(&drop_class);

        let dpi = GetDpiForSystem() as i32;
        let window = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("SearchWin"),
            w!("Search"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            scale(1280, dpi),
            scale(840, dpi),
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // The page engine is the slow part of a launch: start it before anything is drawn.
        page::warm(window);
        // The frame is asked for again now the window exists: WM_NCCALCSIZE takes the caption away.
        let _ = SetWindowPos(window, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);

        // Hidden until it is wanted: the bar shows where you are instead.
        let field = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!(""),
            WINDOW_STYLE(WS_CHILD.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL),
            0,
            0,
            0,
            0,
            Some(window),
            Some(HMENU(1 as _)),
            Some(instance.into()),
            None,
        )?;
        let previous = SetWindowLongPtrW(field, GWLP_WNDPROC, field_proc as *const () as isize);
        // Owned by the window, so it stays above it and goes with it; it never
        // takes the keyboard from the field.
        let drop = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("SearchWinDrop"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(window),
            None,
            Some(instance.into()),
            None,
        )?;
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(drop, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
        let dpi = GetDpiForWindow(window) as i32;
        let dark = theme_dark();
        let palette = Palette::new(dark);
        // What was open last time comes back asleep, but for the tab in front.
        // The space on screen's row: session.json, unless spaces are on and another was left in front.
        let row_of = spaces.current_id().to_string();
        let restored = searchwin_tabs::load(&searchwin_spaces::session_path(&data_folder(), &row_of)).into_tabs(now_ms());
        let splits = if !restored.splits.is_empty() && split::on() { split::from_row(&restored) } else { searchwin_split::Splits::new() };
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                window,
                field,
                field_proc: std::mem::transmute::<isize, WNDPROC>(previous),
                drop,
                font: make_font(dpi, FONT),
                small_font: make_font(dpi, SMALL_FONT),
                crumb_font: make_font(dpi, CRUMB_FONT),
                glyphs: make_glyph_font(dpi, GLYPH),
                caption_glyphs: make_glyph_font(dpi, CAPTION_GLYPH),
                field_brush: CreateSolidBrush(color(palette.field)),
                dpi,
                dark,
                palette,
                timed: false,
                tabs: restored,
                history: None,
                suggestions: Vec::new(),
                highlight: None,
                hover: None,
                hover_close: false,
                tracking_mouse: false,
                tracking_frame: false,
                pressed: None,
                crumb_right: 0,
                drop_hover: None,
                scroll: 0,
                quiet: false,
                session_owed: false,
                history_owed: false,
                private: HashMap::new(),
                privates_made: 0,
                switching: false,
                switch_ids: Vec::new(),
                spaces,
                rows: searchwin_spaces::Rows::new(),
                row_of,
                removals: Vec::new(),
                sent: Vec::new(),
                splits,
                drag: None,
            })
        });
        apply_font();
        let placeholder = HSTRING::from(placeholder());
        SendMessageW(field, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(placeholder.as_ptr() as isize)));
        apply_theme();
        drive::start(window);

        let _ = ShowWindow(window, SW_SHOW);
        layout();

        // A link handed over on the command line (as the default browser will) opens at once,
        // in front of what came back; otherwise the window starts where it was left.
        // A link from another app goes to the space a rule names, when that is another one.
        let routed = std::env::args().nth(1).and_then(|first| resolve(&first)).and_then(|url| Some((spaces::sends_to(url.as_str())?, url)));
        match (std::env::args().nth(1), routed) {
            (_, Some((space, url))) => {
                show_selected();
                spaces::send(url.as_str(), &space);
            }
            (Some(first), None) => open_typed(&first),
            (None, None) => show_selected(),
        }
        SetTimer(Some(window), SLEEP_TIMER, SLEEP_CHECK_MS, None);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if (message.message == WM_KEYDOWN || message.message == WM_SYSKEYDOWN) && shortcut(message.wParam.0 as u32) {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn portuguese() -> bool {
    const LANG_PORTUGUESE: u16 = 0x16;
    unsafe { GetUserDefaultUILanguage() & 0x3ff == LANG_PORTUGUESE }
}

fn placeholder() -> &'static str {
    if portuguese() { "Pesquise ou digite um endereço" } else { "Search or type an address" }
}

fn floating_label() -> &'static str {
    if portuguese() { "Tocando na janela flutuante — Ctrl+Shift+P traz de volta" } else { "Playing in the floating window — Ctrl+Shift+P brings it back" }
}

fn new_tab_label() -> &'static str {
    if portuguese() { "Nova aba" } else { "New tab" }
}

fn system_dark() -> bool {
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 0
}

unsafe fn make_font(dpi: i32, size: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -scale(size, dpi),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Segoe UI Variable Text"),
        )
    }
}

/// The icon font Windows 11 draws its own buttons with; Windows 10 has the
/// older one with the same code points. Asked for by name, a missing font
/// silently becomes another, so the face actually given is checked.
unsafe fn make_glyph_font(dpi: i32, size: i32) -> HFONT {
    let make = |face: PCWSTR| unsafe {
        CreateFontW(
            -scale(size, dpi),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            face,
        )
    };
    let fluent = make(w!("Segoe Fluent Icons"));
    let given = unsafe {
        let hdc = GetDC(None);
        let old = SelectObject(hdc, fluent.into());
        let mut face = [0u16; 32];
        let length = GetTextFaceW(hdc, Some(&mut face));
        SelectObject(hdc, old);
        ReleaseDC(None, hdc);
        String::from_utf16_lossy(&face[..(length.max(1) - 1) as usize])
    };
    if given == "Segoe Fluent Icons" {
        return fluent;
    }
    unsafe {
        let _ = DeleteObject(fluent.into());
    }
    make(w!("Segoe MDL2 Assets"))
}

fn apply_font() {
    if let Some((field, font)) = state(|s| (s.field, s.font)) {
        unsafe { SendMessageW(field, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1))) };
    }
}

/// Menus in the app's light or dark. Windows has no public switch for a Win32
/// app's menus; uxtheme's SetPreferredAppMode (ordinal 135) and FlushMenuThemes
/// (136), there since Windows 10 1903, are what Explorer and Notepad++ use. If a
/// later Windows drops them, the lookup fails and menus simply stay light.
fn menus_follow(dark: bool) {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        let ordinal = |n: usize| GetProcAddress(uxtheme, windows_core::PCSTR(n as *const u8));
        if let Some(set_mode) = ordinal(135) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(if dark { 2 } else { 3 }); // ForceDark : ForceLight
        }
        if let Some(flush) = ordinal(136) {
            let flush: extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}

fn apply_theme() {
    let Some((window, field, drop, dark, palette)) = state(|s| (s.window, s.field, s.drop, s.dark, s.palette)) else { return };
    menus_follow(dark);
    unsafe {
        let dark_mode = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_mode as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(drop, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_mode as *const _ as _, 4);
        let _ = InvalidateRect(Some(window), None, true);
        let _ = InvalidateRect(Some(field), None, true);
        let _ = InvalidateRect(Some(drop), None, true);
    }
    page::set_background(palette.card);
}

/// Where the bar's buttons are, and the field when it is out.
struct Bar {
    back: RECT,
    forward: RECT,
    reload: RECT,
    /// The field's box; the page's place is written where the field's text would start.
    field: RECT,
    minimize: RECT,
    maximize: RECT,
    close: RECT,
}

fn bar_spots(client: RECT, dpi: i32) -> Bar {
    let height = scale(BAR, dpi);
    let nav = scale(NAV, dpi);
    let top = (height - nav) / 2;
    let button = |left: i32| RECT { left, top, right: left + nav, bottom: top + nav };
    let back = button(scale(COLUMN + GAP, dpi));
    let forward = button(back.right + scale(2, dpi));
    let reload = button(forward.right + scale(2, dpi));
    let caption = scale(CAPTION_W, dpi);
    let close = RECT { left: client.right - caption, top: 0, right: client.right, bottom: height };
    let maximize = RECT { left: close.left - caption, right: close.left, ..close };
    let minimize = RECT { left: maximize.left - caption, right: maximize.left, ..close };
    let field_top = (height - scale(FIELD_H, dpi)) / 2;
    let left = reload.right + scale(GAP, dpi);
    let right = (minimize.left - scale(2 * GAP, dpi)).min(left + scale(FIELD_MOST, dpi)).max(left);
    let field = RECT { left, top: field_top, right, bottom: field_top + scale(FIELD_H, dpi) };
    Bar { back, forward, reload, field, minimize, maximize, close }
}

fn bar_rect(client: RECT, dpi: i32) -> RECT {
    RECT { left: scale(COLUMN, dpi), top: 0, right: client.right.max(scale(COLUMN, dpi)), bottom: scale(BAR, dpi) }
}

/// The page fills the rest, to the window's edges.
fn card_rect(client: RECT, dpi: i32) -> RECT {
    RECT {
        left: scale(COLUMN, dpi),
        top: scale(BAR, dpi),
        right: client.right.max(scale(COLUMN, dpi)),
        bottom: client.bottom.max(scale(BAR, dpi)),
    }
}

fn column_rect(client: RECT, dpi: i32) -> RECT {
    RECT { left: 0, top: 0, right: scale(COLUMN, dpi), bottom: client.bottom }
}

fn client_rect() -> RECT {
    let mut client = RECT::default();
    if let Some(window) = state(|s| s.window) {
        unsafe {
            let _ = GetClientRect(window, &mut client);
        }
    }
    client
}

fn layout() {
    let Some((window, field, dpi)) = state(|s| (s.window, s.field, s.dpi)) else { return };
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut client);
        let frame = bar_spots(client, dpi).field;
        let line = scale(LINE, dpi);
        let pad = scale(FIELD_PAD, dpi);
        let _ = MoveWindow(
            field,
            frame.left + pad,
            frame.top + (frame.bottom - frame.top - line) / 2,
            frame.right - frame.left - 2 * pad,
            line,
            true,
        );
    }
    place_pages();
    place_drop();
}

/// The page in front in the card, or the pages of its split side by side.
fn place_pages() {
    let client = client_rect();
    let Some((card, panes)) = state(|s| {
        let card = card_rect(client, s.dpi);
        (card, if s.splits.is_empty() { Vec::new() } else { split::panes(s, card) })
    }) else {
        return;
    };
    page::arrange(card, panes);
}

/// How thick Windows makes the sizing frame at this DPI: what a maximized
/// window hangs off the screen by, and how deep the top edge's grip is.
fn frame_thickness(dpi: i32) -> i32 {
    unsafe {
        GetSystemMetricsForDpi(SM_CYFRAME, dpi as u32) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi as u32)
    }
}

fn zoomed(window: HWND) -> bool {
    unsafe { IsZoomed(window).as_bool() }
}
/// One spot in the column, placed.
struct Row {
    item: Item,
    rect: RECT,
    label: String,
    /// A suggestion's address, under its title.
    detail: String,
    letter: String,
    /// Whose icon it shows.
    host: String,
    selected: bool,
    asleep: bool,
    /// A private tab: its row says so, quietly.
    private: bool,
    /// A split's row: the next page's site and letter, its mark beside the first's.
    also: Option<(String, String)>,
}

/// The column below the field: what is where, as painted and as clicked.
struct Column {
    rows: Vec<Row>,
    /// Where tab rows may show; they scroll inside it.
    list: RECT,
    /// The most the tab rows can scroll.
    most: i32,
    /// With spaces on, a chip for each space at the column's foot.
    chips: Vec<(RECT, spaces::Chip)>,
}

/// A space's chip: a small square, as upstream's space dot.
const CHIP: i32 = 26;
const CHIP_GAP: i32 = 4;

fn private_tab_label() -> &'static str {
    if portuguese() { "Aba privada" } else { "Private tab" }
}

/// What a tab's row says: its name, its title, or its address while it has none.
fn label(tab: &Tab, private: bool) -> String {
    if let Some(name) = &tab.name {
        return name.clone();
    }
    if !tab.title.is_empty() {
        return tab.title.clone();
    }
    match tab.url.as_deref().and_then(|u| url::Url::parse(u).ok()) {
        Some(url) => address::pretty(&url),
        None if private => private_tab_label().to_string(),
        None => new_tab_label().to_string(),
    }
}

/// Ctrl+K's rows: the open tabs other than the one in front, the most
/// recently used first, whose title or address has every word typed, in any
/// order, whatever their case or accents (upstream's openPages).
fn open_tabs(s: &State, typed: &str) -> Vec<(u64, Suggestion)> {
    let fold = |text: &str| -> String { text.to_lowercase().chars().map(plain_letter).collect() };
    let typed = fold(typed);
    let words: Vec<&str> = typed.split_whitespace().collect();
    let front = s.tabs.selected_id();
    // A word with a letter wrong still finds its tab (History's Fuzzy), after
    // every tab that has the words as typed.
    let mut found: Vec<(u8, u64, u64, Suggestion)> = s
        .tabs
        .iter()
        .filter(|t| t.id != front && !t.is_blank())
        .filter_map(|t| {
            let title = label(t, s.private.contains_key(&t.id));
            let address = t.url.as_deref().and_then(|u| url::Url::parse(u).ok()).map(|u| address::pretty(&u)).unwrap_or_default();
            let text = fold(&format!("{title} {address}"));
            searchwin_history::fuzzy::matches(&words, &text).map(|exact| {
                let row = Suggestion { key: address, title, url: String::new(), kind: searchwin_history::Kind::Visited };
                (exact, t.id, t.last_active_ms, row)
            })
        })
        .collect();
    found.sort_unstable_by_key(|(exact, id, touched, _)| (std::cmp::Reverse(*exact), std::cmp::Reverse(*touched), *id));
    found.into_iter().take(SUGGESTIONS).map(|(_, id, _, row)| (id, row)).collect()
}

/// A letter without its accent, as typed by someone who didn't bother: "e" finds "é".
fn plain_letter(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        _ => c,
    }
}

/// The small letter a tab row shows for its site.
fn letter(url: Option<&str>) -> String {
    let Some(url) = url.and_then(|u| url::Url::parse(u).ok()) else { return String::new() };
    let host = address::without_www(url.host_str().unwrap_or(""));
    host.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// A page's site, as its icon is kept: the host without www.
fn host_of(url: Option<&str>) -> String {
    url.and_then(|u| url::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(|h| address::without_www(h).to_lowercase()))
        .unwrap_or_default()
}

fn column(s: &State, bottom: i32) -> Column {
    let dpi = s.dpi;
    let left = scale(SIDE_PAD, dpi);
    let right = scale(COLUMN - SIDE_PAD, dpi);
    // The strip above is left empty: the window is dragged by it.
    let mut top = scale(BAR, dpi);
    let mut rows = Vec::new();
    let mut list = RECT { left: 0, top, right: scale(COLUMN, dpi), bottom: bottom - scale(SIDE_PAD, dpi) };

    // The spaces, one chip each, in a line at the foot; the tabs end above them.
    let mut chips = Vec::new();
    let found = spaces::chips(&s.spaces);
    if !found.is_empty() {
        let side = scale(CHIP, dpi);
        let step = side + scale(CHIP_GAP, dpi);
        let y = bottom - scale(SIDE_PAD, dpi) - side;
        for (i, chip) in found.into_iter().enumerate() {
            let x = left + i as i32 * step;
            if x + side > right {
                break;
            }
            chips.push((RECT { left: x, top: y, right: x + side, bottom: y + side }, chip));
        }
        list.bottom = y - scale(GAP, dpi);
    }

    // The tab in front, or the split it is a page of: that row stands for it.
    let selected = s.tabs.selected_id();
    let selected = s.splits.split_of(selected).map_or(selected, |split| split.first());
    // Pins: big tiles that never move, so each stays under your hand.
    let pins = s.tabs.pinned_count();
    let gap = scale(PIN_GAP, dpi);
    let per_line = PINS_PER_LINE as usize;
    let width = (right - left - (PINS_PER_LINE - 1) * gap) / PINS_PER_LINE;
    let height = scale(PIN_H, dpi);
    for (i, tab) in s.tabs.iter().take(pins).enumerate() {
        let x = left + (i % per_line) as i32 * (width + gap);
        let y = top + (i / per_line) as i32 * (height + gap);
        rows.push(Row {
            item: Item::Pin(tab.id),
            rect: RECT { left: x, top: y, right: x + width, bottom: y + height },
            label: label(tab, s.private.contains_key(&tab.id)),
            detail: String::new(),
            letter: tab.pin_letter.clone().unwrap_or_else(|| letter(tab.url.as_deref())),
            host: host_of(tab.url.as_deref()),
            selected: tab.id == selected,
            asleep: tab.asleep,
            private: s.private.contains_key(&tab.id),
            also: None,
        });
    }
    if pins > 0 {
        top += pins.div_ceil(per_line) as i32 * (height + gap);
    }
    list.top = top;

    let height = scale(ROW, dpi);
    // A split's later pages have no row of their own (Browser.standsInRow).
    let standing = |t: &&Tab| s.splits.stands_in_row(t.id);
    let content = (s.tabs.iter().skip(pins).filter(standing).count() + 1) as i32 * height;
    let most = (content - (list.bottom - list.top)).max(0);
    let mut y = top - s.scroll.clamp(0, most);
    for tab in s.tabs.iter().skip(pins).filter(standing) {
        let mut label = label(tab, s.private.contains_key(&tab.id));
        let mut also = None;
        // A split is one row: both marks, every title.
        if let Some(split) = s.splits.split_of(tab.id) {
            let others: Vec<&Tab> = split.tabs.iter().skip(1).filter_map(|t| s.tabs.get(*t)).collect();
            for other in &others {
                label = format!("{label}  |  {}", self::label(other, s.private.contains_key(&other.id)));
            }
            also = others.first().map(|o| (host_of(o.url.as_deref()), letter(o.url.as_deref())));
        }
        rows.push(Row {
            item: Item::Tab(tab.id),
            rect: RECT { left, top: y, right, bottom: y + height },
            label,
            detail: String::new(),
            letter: letter(tab.url.as_deref()),
            host: host_of(tab.url.as_deref()),
            selected: tab.id == selected,
            asleep: tab.asleep,
            private: s.private.contains_key(&tab.id),
            also,
        });
        y += height;
    }
    rows.push(Row {
        item: Item::NewTab,
        rect: RECT { left, top: y, right, bottom: y + height },
        label: new_tab_label().to_string(),
        detail: String::new(),
        letter: String::new(),
        host: String::new(),
        selected: false,
        asleep: false,
        private: false,
        also: None,
    });
    Column { rows, list, most, chips }
}

/// The drop-down's rows, in its own window's coordinates.
fn drop_rows(s: &State, width: i32) -> Vec<Row> {
    let dpi = s.dpi;
    let pad = scale(DROP_PAD, dpi);
    let height = scale(SUGGESTION_ROW, dpi);
    s.suggestions
        .iter()
        .enumerate()
        .map(|(i, suggestion)| {
            let y = pad + i as i32 * height;
            // Ctrl+K's rows have no address of their own to read: their tab's is in `key`.
            let shown = if suggestion.url.is_empty() { format!("https://{}", suggestion.key) } else { suggestion.url.clone() };
            Row {
                item: Item::Suggestion(i),
                rect: RECT { left: pad, top: y, right: width - pad, bottom: y + height },
                label: if suggestion.title.is_empty() { suggestion.key.clone() } else { suggestion.title.clone() },
                detail: suggestion.key.clone(),
                letter: letter(Some(&shown)),
                host: host_of(Some(&shown)),
                selected: s.highlight == Some(i) || (s.highlight.is_none() && s.drop_hover == Some(i)),
                asleep: false,
                private: false,
                also: None,
            }
        })
        .collect()
}

/// The × at the right end of a tab row.
fn close_rect(row: RECT) -> RECT {
    let side = row.bottom - row.top;
    RECT { left: row.right - side, ..row }
}

fn inside(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// What in the bar is at x, y (client coordinates), if anything.
fn bar_hit(s: &State, client: RECT, x: i32, y: i32) -> Option<Item> {
    let dpi = s.dpi;
    if y >= scale(BAR, dpi) || x < scale(COLUMN, dpi) {
        return None;
    }
    let bar = bar_spots(client, dpi);
    let spots = [
        (bar.back, Item::Back),
        (bar.forward, Item::Forward),
        (bar.reload, Item::Reload),
        (bar.minimize, Item::Minimize),
        (bar.maximize, Item::Maximize),
        (bar.close, Item::Close),
    ];
    if let Some((_, item)) = spots.iter().find(|(r, _)| inside(*r, x, y)) {
        return Some(*item);
    }
    // A short place is still easy to hit; the rest of the bar drags the window.
    let right = s.crumb_right.max(bar.field.left + scale(160, dpi)).min(bar.field.right);
    inside(RECT { right, ..bar.field }, x, y).then_some(Item::Crumb)
}

/// What is under the mouse, and whether it is on that row's ×.
fn hit(x: i32, y: i32) -> Option<(Item, bool)> {
    let client = client_rect();
    state(|s| {
        if let Some(item) = bar_hit(s, client, x, y) {
            return Some((item, false));
        }
        let column = column(s, client.bottom);
        if let Some(i) = column.chips.iter().position(|(r, _)| inside(*r, x, y)) {
            return Some((Item::Space(i), false));
        }
        let row = column.rows.iter().find(|r| {
            inside(r.rect, x, y) && (!matches!(r.item, Item::Tab(_) | Item::NewTab) || inside(column.list, x, y))
        })?;
        let on_close = matches!(row.item, Item::Tab(_)) && inside(close_rect(row.rect), x, y);
        Some((row.item, on_close))
    })
    .flatten()
}

unsafe fn rounded(hdc: HDC, rect: RECT, fill: Rgb, edge: Rgb, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(color(fill));
        let pen = CreatePen(PS_SOLID, 1, color(edge));
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

/// A raised pill: a one-pixel shadow under it, then the pill with its hairline.
unsafe fn raised(hdc: HDC, rect: RECT, palette: &Palette, radius: i32) {
    unsafe {
        let below = RECT { top: rect.top + 1, bottom: rect.bottom + 1, ..rect };
        rounded(hdc, below, palette.shadow, palette.shadow, radius);
        rounded(hdc, rect, palette.pill, palette.border, radius);
    }
}

unsafe fn fill(hdc: HDC, rect: RECT, colour: Rgb) {
    unsafe {
        let brush = CreateSolidBrush(color(colour));
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

unsafe fn text(hdc: HDC, words: &str, mut rect: RECT, font: HFONT, ink: Rgb, center: bool) {
    if words.is_empty() {
        return;
    }
    let mut wide: Vec<u16> = words.encode_utf16().collect();
    let align = if center { DT_CENTER } else { DT_LEFT };
    unsafe {
        SelectObject(hdc, font.into());
        SetTextColor(hdc, color(ink));
        DrawTextW(hdc, &mut wide, &mut rect, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | align);
    }
}

/// How wide `words` come out in `font`.
unsafe fn measure(hdc: HDC, words: &str, font: HFONT) -> i32 {
    if words.is_empty() {
        return 0;
    }
    let mut wide: Vec<u16> = words.encode_utf16().collect();
    let mut rect = RECT::default();
    unsafe {
        SelectObject(hdc, font.into());
        DrawTextW(hdc, &mut wide, &mut rect, DT_SINGLELINE | DT_NOPREFIX | DT_CALCRECT);
    }
    rect.right - rect.left
}

/// A site's mark in `spot`: its icon if one came, else its letter on a small tile.
/// A private tab's letter is drawn the other way round: dark on light, light on dark.
unsafe fn site_mark(hdc: HDC, row: &Row, spot: RECT, behind: Rgb, palette: &Palette, small_font: HFONT, dpi: i32) {
    unsafe {
        if row.private {
            rounded(hdc, spot, palette.dim, palette.dim, scale(4, dpi));
            text(hdc, &row.letter, spot, small_font, palette.card, true);
        } else if !favicon::draw(hdc, &row.host, spot, behind) && !row.letter.is_empty() {
            rounded(hdc, spot, palette.border, palette.border, scale(4, dpi));
            text(hdc, &row.letter, spot, small_font, palette.dim, true);
        }
    }
}

/// A square of `side` centred in `rect`.
fn centred(rect: RECT, side: i32) -> RECT {
    let left = rect.left + (rect.right - rect.left - side) / 2;
    let top = rect.top + (rect.bottom - rect.top - side) / 2;
    RECT { left, top, right: left + side, bottom: top + side }
}

/// Everything the window needs to be drawn, copied out so no borrow is held while drawing.
struct Picture {
    column: Column,
    palette: Palette,
    font: HFONT,
    small_font: HFONT,
    crumb_font: HFONT,
    glyphs: HFONT,
    caption_glyphs: HFONT,
    dpi: i32,
    hover: Option<Item>,
    hover_close: bool,
    pressed: Option<Item>,
    /// The bar's.
    can_back: bool,
    can_forward: bool,
    site: String,
    title: String,
    editing: bool,
    zoomed: bool,
    /// The column's ground: the space's colour faintly over it, with spaces on.
    ground: Rgb,
    /// The split in front, side by side: its dividers, and the page with the keys.
    dividers: Vec<RECT>,
    focused_pane: Option<RECT>,
    /// The page in front is out in the floating window: the card says so.
    floated: bool,
}

/// Draws off screen and copies in at once: hovering never flickers.
unsafe fn buffered(hdc: HDC, area: RECT, draw: impl FnOnce(HDC, RECT)) {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    if width <= 0 || height <= 0 {
        return;
    }
    unsafe {
        let canvas = CreateCompatibleDC(Some(hdc));
        let bitmap = CreateCompatibleBitmap(hdc, width, height);
        let old_bitmap = SelectObject(canvas, bitmap.into());
        SetBkMode(canvas, TRANSPARENT);
        // Drawn as if the canvas were the window, from the area's own corner.
        let _ = SetViewportOrgEx(canvas, -area.left, -area.top, None);
        let old_font = SelectObject(canvas, GetStockObject(DEFAULT_GUI_FONT));
        draw(canvas, area);
        let _ = SetViewportOrgEx(canvas, 0, 0, None);
        let _ = BitBlt(hdc, area.left, area.top, width, height, Some(canvas), 0, 0, SRCCOPY);
        SelectObject(canvas, old_font);
        SelectObject(canvas, old_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(canvas);
    }
}

unsafe fn paint_column(canvas: HDC, area: RECT, p: &Picture) {
    let dpi = p.dpi;
    let palette = p.palette;
    // The space's wash is the column's ground: everything drawn "on the column" sits on it.
    let palette = Palette { background: p.ground, ..palette };
    unsafe {
        fill(canvas, area, palette.background);
        // The column's edge: one hairline between it and the page.
        fill(canvas, RECT { left: area.right - 1, ..area }, palette.border);

        let radius = scale(RADIUS, dpi);
        let gap = scale(12, dpi);
        let icon = scale(ICON, dpi);
        let mut clipped = false;
        for row in &p.column.rows {
            let r = row.rect;
            let hovered = p.hover == Some(row.item);
            match row.item {
                Item::Pin(_) => {
                    if row.selected {
                        raised(canvas, r, &palette, radius);
                    } else {
                        let tile = if hovered { mix(palette.tile, palette.pill, 50) } else { palette.tile };
                        rounded(canvas, r, tile, palette.border, radius);
                    }
                    let spot = centred(r, scale(PIN_ICON, dpi));
                    if row.private || !favicon::draw(canvas, &row.host, spot, if row.selected { palette.pill } else { palette.tile }) {
                        let ink = if row.selected { palette.text } else if row.asleep { palette.faint } else { palette.dim };
                        let ink = if row.private { palette.text } else { ink };
                        text(canvas, &row.letter, r, p.font, ink, true);
                    }
                }
                Item::Tab(_) | Item::NewTab => {
                    // Tab rows scroll under the pins, never over them.
                    if !clipped {
                        let list = p.column.list;
                        SaveDC(canvas);
                        IntersectClipRect(canvas, list.left, list.top, list.right, list.bottom);
                        clipped = true;
                    }
                    let inset = scale(2, dpi);
                    let pill = RECT { top: r.top + inset, bottom: r.bottom - inset, ..r };
                    if row.selected {
                        raised(canvas, pill, &palette, radius);
                    } else if hovered {
                        rounded(canvas, pill, palette.hover, palette.hover, radius);
                    }
                    let icon_box = RECT {
                        left: r.left + gap,
                        top: r.top + (r.bottom - r.top - icon) / 2,
                        right: r.left + gap + icon,
                        bottom: r.top + (r.bottom - r.top - icon) / 2 + icon,
                    };
                    let mut words = RECT { left: icon_box.right + scale(10, dpi), ..r };
                    if row.item == Item::NewTab {
                        let ink = if hovered { palette.dim } else { palette.faint };
                        text(canvas, ADD_GLYPH, icon_box, p.caption_glyphs, ink, true);
                        text(canvas, &row.label, words, p.font, ink, false);
                        continue;
                    }
                    let behind = if row.selected { palette.pill } else if hovered { palette.hover } else { palette.background };
                    site_mark(canvas, row, icon_box, behind, &palette, p.small_font, dpi);
                    // A split: the next page's mark beside the first's.
                    if let Some((host, letter)) = &row.also {
                        let next = RECT { left: icon_box.right + scale(4, dpi), right: icon_box.right + scale(4, dpi) + icon, ..icon_box };
                        let other = Row { host: host.clone(), letter: letter.clone(), private: false, also: None, label: String::new(), detail: String::new(), ..*row };
                        site_mark(canvas, &other, next, behind, &palette, p.small_font, dpi);
                        words.left = next.right + scale(10, dpi);
                    }
                    if hovered {
                        let close = close_rect(pill);
                        words.right = close.left;
                        if p.hover_close {
                            let spot = centred(close, scale(24, dpi));
                            rounded(canvas, spot, palette.border, palette.border, scale(4, dpi));
                        }
                        text(canvas, "\u{00D7}", close, p.font, palette.dim, true);
                    } else {
                        words.right -= gap;
                    }
                    let ink = if row.selected { palette.text } else if row.asleep { palette.faint } else { palette.dim };
                    text(canvas, &row.label, words, p.font, ink, false);
                }
                _ => {}
            }
        }
        if clipped {
            let _ = RestoreDC(canvas, -1);
        }
        // The spaces: the one on screen raised, each in its colour if it has one.
        for (i, (r, chip)) in p.column.chips.iter().enumerate() {
            let hovered = p.hover == Some(Item::Space(i));
            let radius = scale(8, dpi);
            if chip.current {
                let wash = chip.tint.map_or(palette.pill, |t| mix(palette.pill, t, 14));
                rounded(canvas, *r, wash, palette.border, radius);
            } else if hovered {
                let wash = chip.tint.map_or(palette.hover, |t| mix(palette.background, t, 24));
                rounded(canvas, *r, wash, wash, radius);
            }
            let ink = match chip.tint {
                Some(t) => mix(t, palette.text, 25),
                None if chip.current || hovered => palette.text,
                None => palette.dim,
            };
            text(canvas, &chip.letter, *r, p.small_font, ink, true);
        }
    }
}

/// The bar: back, forward, reload; where you are (or the field's box); the
/// window's buttons. Gives back where the page's place ends, to be clicked.
unsafe fn paint_bar(canvas: HDC, area: RECT, client: RECT, p: &Picture) -> i32 {
    let dpi = p.dpi;
    let palette = p.palette;
    let bar = bar_spots(client, dpi);
    let mut crumb_right = 0;
    unsafe {
        fill(canvas, area, palette.card);
        // A hairline under it, so a white page and a white bar don't run together.
        fill(canvas, RECT { top: area.bottom - 1, ..area }, mix(palette.card, palette.border, 60));

        let radius = scale(6, dpi);
        for (rect, item, glyph, enabled) in [
            (bar.back, Item::Back, BACK_GLYPH, p.can_back),
            (bar.forward, Item::Forward, FORWARD_GLYPH, p.can_forward),
            (bar.reload, Item::Reload, RELOAD_GLYPH, true),
        ] {
            if enabled && p.hover == Some(item) {
                rounded(canvas, rect, palette.bar_hover, palette.bar_hover, radius);
            }
            let ink = if enabled { palette.dim } else { mix(palette.faint, palette.card, 45) };
            text(canvas, glyph, rect, p.glyphs, ink, true);
        }

        let words = RECT { left: bar.field.left + scale(FIELD_PAD, dpi), right: bar.field.right, ..bar.field };
        if p.editing {
            rounded(canvas, bar.field, palette.field, palette.border, scale(RADIUS, dpi));
        } else if p.site.is_empty() {
            if p.hover == Some(Item::Crumb) {
                rounded(canvas, bar.field, palette.bar_hover, palette.bar_hover, scale(RADIUS, dpi));
            }
            text(canvas, placeholder(), words, p.crumb_font, palette.faint, false);
            crumb_right = words.left + measure(canvas, placeholder(), p.crumb_font);
        } else {
            // "site / title": the site a little stronger, the title quieter after it.
            let site_w = measure(canvas, &p.site, p.crumb_font).min(words.right - words.left);
            let rest = if p.title.is_empty() { String::new() } else { format!("  /  {}", p.title) };
            let rest_w = measure(canvas, &rest, p.crumb_font);
            crumb_right = (words.left + site_w + rest_w).min(words.right);
            if p.hover == Some(Item::Crumb) {
                let spot = RECT { right: (crumb_right + scale(FIELD_PAD, dpi)).min(bar.field.right), ..bar.field };
                rounded(canvas, spot, palette.bar_hover, palette.bar_hover, scale(RADIUS, dpi));
            }
            text(canvas, &p.site, RECT { right: words.left + site_w, ..words }, p.crumb_font, mix(palette.text, palette.dim, 35), false);
            text(canvas, &rest, RECT { left: words.left + site_w, ..words }, p.crumb_font, palette.dim, false);
        }

        let maximize = if p.zoomed { RESTORE_GLYPH } else { MAXIMIZE_GLYPH };
        for (rect, item, glyph) in
            [(bar.minimize, Item::Minimize, MINIMIZE_GLYPH), (bar.maximize, Item::Maximize, maximize), (bar.close, Item::Close, CLOSE_GLYPH)]
        {
            let hovered = p.hover == Some(item);
            let pressed = hovered && p.pressed == Some(item);
            let mut ink = palette.text;
            if item == Item::Close && hovered {
                // Windows' own close red, and white on it.
                fill(canvas, rect, if pressed { (148, 26, 18) } else { (196, 43, 28) });
                ink = (255, 255, 255);
            } else if hovered {
                fill(canvas, rect, if pressed { mix(palette.card, palette.text, 14) } else { palette.bar_hover });
            }
            text(canvas, glyph, rect, p.caption_glyphs, ink, true);
        }
    }
    crumb_right
}

/// The page's place in the bar: its site, and its title.
fn crumb(tab: &Tab) -> (String, String) {
    let Some(url) = tab.url.as_deref().and_then(|u| url::Url::parse(u).ok()) else { return Default::default() };
    if url.scheme() == "about" {
        return Default::default();
    }
    let site = match url.host_str() {
        Some(host) if url.scheme() != omnibox::INTERNAL_SCHEME => address::without_www(host).to_string(),
        _ => address::pretty(&url),
    };
    let title = if tab.title == site { String::new() } else { tab.title.clone() };
    (site, title)
}

fn paint(window: HWND) {
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(window, &mut client);
    }
    // Asked of the page before the state is borrowed: it is a call into WebView2.
    let (can_back, can_forward) = page::can_go();
    let zoomed = zoomed(window);
    let focus = unsafe { GetFocus() };
    let floating = float::tab();
    let Some(picture) = state(|s| {
        let (site, title) = crumb(s.tabs.selected());
        let ground = spaces::wash(&s.spaces, s.dark).map_or(s.palette.background, |(tint, amount)| mix(s.palette.background, tint, amount));
        let (mut dividers, mut focused_pane) = (Vec::new(), None);
        if let Some(layout) = split::layout_of(s, card_rect(client, s.dpi)).filter(|l| l.paired) {
            dividers = layout.dividers.iter().map(|d| split::win(*d)).collect();
            let front = s.tabs.selected_id();
            focused_pane = layout.panes.iter().find(|(id, _)| *id == front).map(|(_, r)| split::win(*r));
        }
        Picture {
            ground,
            dividers,
            focused_pane,
            floated: floating.is_some() && floating == Some(s.tabs.selected_id()),
            column: column(s, client.bottom),
            palette: s.palette,
            font: s.font,
            small_font: s.small_font,
            crumb_font: s.crumb_font,
            glyphs: s.glyphs,
            caption_glyphs: s.caption_glyphs,
            dpi: s.dpi,
            hover: s.hover,
            hover_close: s.hover_close,
            pressed: s.pressed,
            can_back,
            can_forward,
            site,
            title,
            editing: focus == s.field,
            zoomed,
        }
    }) else {
        return;
    };
    let dpi = picture.dpi;
    let mut crumb_right = None;
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(window, &mut paint);
        let dirty = paint.rcPaint;
        let overlaps = |r: RECT| dirty.left < r.right && dirty.right > r.left && dirty.top < r.bottom && dirty.bottom > r.top;
        let column = column_rect(client, dpi);
        if overlaps(column) {
            buffered(hdc, column, |canvas, area| paint_column(canvas, area, &picture));
        }
        let bar = bar_rect(client, dpi);
        if overlaps(bar) {
            buffered(hdc, bar, |canvas, area| crumb_right = Some(paint_bar(canvas, area, client, &picture)));
        }
        // Under the page, seen only before it has drawn.
        let card = card_rect(client, dpi);
        if overlaps(card) {
            let palette = picture.palette;
            fill(hdc, card, palette.card);
            if picture.floated {
                SetBkMode(hdc, TRANSPARENT);
                text(hdc, floating_label(), card, picture.crumb_font, palette.faint, true);
            }
            // Pages side by side: a hairline in each gap, and the page with the
            // keys outlined in the accent, in the hair its place is drawn inside.
            for d in &picture.dividers {
                let middle = d.left + (d.right - d.left) / 2;
                fill(hdc, RECT { left: middle, right: middle + 1, ..*d }, palette.border);
            }
            if let Some(r) = picture.focused_pane {
                let line = scale(2, dpi);
                for edge in [
                    RECT { bottom: r.top + line, ..r },
                    RECT { top: r.bottom - line, ..r },
                    RECT { right: r.left + line, ..r },
                    RECT { left: r.right - line, ..r },
                ] {
                    fill(hdc, edge, palette.accent);
                }
            }
        }
        let _ = EndPaint(window, &paint);
    }
    if let Some(right) = crumb_right {
        state(|s| s.crumb_right = right);
    }
}

/// Only the column is drawn again; the bar and the page are left alone.
fn repaint_column() {
    let client = client_rect();
    if let Some((window, dpi)) = state(|s| (s.window, s.dpi)) {
        let area = column_rect(client, dpi);
        unsafe {
            let _ = InvalidateRect(Some(window), Some(&area), false);
        }
    }
}

fn repaint_bar() {
    let client = client_rect();
    if let Some((window, dpi)) = state(|s| (s.window, s.dpi)) {
        let area = bar_rect(client, dpi);
        unsafe {
            let _ = InvalidateRect(Some(window), Some(&area), false);
        }
    }
}

/// Under the pages: a split's dividers and its outline.
fn repaint_card() {
    let client = client_rect();
    if let Some((window, dpi)) = state(|s| (s.window, s.dpi)) {
        let area = card_rect(client, dpi);
        unsafe {
            let _ = InvalidateRect(Some(window), Some(&area), false);
        }
    }
}

fn repaint_all() {
    if let Some(window) = state(|s| s.window) {
        unsafe {
            let _ = InvalidateRect(Some(window), None, false);
        }
    }
}

/// Another space came on screen (spaces::settle): its tab in front, its pages
/// placed, and everything drawn again in its colour.
fn arrived() {
    show_selected();
    layout();
    repaint_all();
}

fn mouse_moved(window: HWND, x: i32, y: i32) {
    if state(|s| s.drag.is_some()) == Some(true) {
        split::drag_to(x);
        return;
    }
    let found = hit(x, y);
    let (changed, track) = state(|s| {
        let item = found.map(|(item, _)| item);
        let close = found.is_some_and(|(_, close)| close);
        let changed = s.hover != item || s.hover_close != close;
        s.hover = item;
        s.hover_close = close;
        (changed, !std::mem::replace(&mut s.tracking_mouse, true))
    })
    .unwrap_or_default();
    if track {
        track_leave(window, TME_LEAVE);
    }
    if changed {
        repaint_column();
        repaint_bar();
    }
}

fn track_leave(window: HWND, flags: TRACKMOUSEEVENT_FLAGS) {
    let mut leave = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: flags,
        hwndTrack: window,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut leave);
    }
}

/// The mouse is over the maximize button: Windows reports it as its own
/// (that is what brings up Snap Layouts), so the hover is kept here.
fn frame_moved(window: HWND, over_maximize: bool) {
    let (changed, track) = state(|s| {
        let item = over_maximize.then_some(Item::Maximize);
        let changed = s.hover != item;
        s.hover = item;
        s.hover_close = false;
        (changed, over_maximize && !std::mem::replace(&mut s.tracking_frame, true))
    })
    .unwrap_or_default();
    if track {
        track_leave(window, TME_LEAVE | TME_NONCLIENT);
    }
    if changed {
        repaint_bar();
    }
}

fn clicked(window: HWND, x: i32, y: i32) {
    // A split's divider, taken hold of.
    if let Some(index) = split::divider_at(x, y)
        && split::begin_drag(index)
    {
        unsafe {
            SetCapture(window);
        }
        return;
    }
    match hit(x, y) {
        Some((Item::Tab(id), true)) => request_close(id),
        // Alt+click: beside the page in front (Split View).
        Some((Item::Tab(id), _)) if held(VK_MENU) && split::on() => {
            split::join(id);
        }
        Some((Item::Tab(id), _)) => {
            // A split's row: the page in it that last had the keys.
            let to = state(|s| s.splits.split_of(id).map_or(id, |split| split.focused.unwrap_or(id))).unwrap_or(id);
            select_tab(to);
        }
        Some((Item::Pin(id), _)) => select_tab(id),
        Some((Item::Space(i), _)) => {
            let bottom = client_rect().bottom;
            if let Some(id) = state(|s| column(s, bottom).chips.get(i).map(|(_, c)| c.id.clone())).flatten() {
                spaces::go_to(&id);
            }
        }
        Some((Item::NewTab, _)) => new_tab(),
        Some((Item::Back, _)) => page::back(),
        Some((Item::Forward, _)) => page::forward(),
        Some((Item::Reload, _)) => page::reload(),
        Some((Item::Crumb, _)) => focus_field(),
        Some((item @ (Item::Minimize | Item::Close), _)) => {
            state(|s| s.pressed = Some(item));
            repaint_bar();
        }
        // A page of a split with no page of its own (an empty tab): a click on its place takes it.
        None => {
            let client = client_rect();
            let card = state(|s| card_rect(client, s.dpi));
            let pane = card
                .and_then(|card| state(|s| split::panes(s, card)))
                .and_then(|panes| panes.into_iter().find(|(_, r)| inside(*r, x, y)).map(|(id, _)| id));
            if let Some(id) = pane {
                select_tab(id);
            }
        }
        _ => {}
    }
}

/// Minimize and close, let go over the button they were pressed on.
fn released(window: HWND, x: i32, y: i32) {
    if state(|s| s.drag.is_some()) == Some(true) {
        unsafe {
            let _ = ReleaseCapture();
        }
        split::end_drag();
        return;
    }
    let Some(pressed) = state(|s| s.pressed.take()).flatten() else { return };
    repaint_bar();
    if hit(x, y).map(|(item, _)| item) != Some(pressed) {
        return;
    }
    unsafe {
        match pressed {
            Item::Minimize => {
                let _ = ShowWindow(window, SW_MINIMIZE);
            }
            Item::Close => {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            _ => {}
        }
    }
}

fn toggle_maximized(window: HWND) {
    unsafe {
        let _ = ShowWindow(window, if zoomed(window) { SW_RESTORE } else { SW_MAXIMIZE });
    }
}

/// The middle button closes, as it does on a tab everywhere else.
fn middle_clicked(x: i32, y: i32) {
    if let Some((Item::Tab(id) | Item::Pin(id), _)) = hit(x, y) {
        request_close(id);
    }
}

/// A tab's own menu, as upstream's right-click on a tab: pin or unpin it, close it.
/// It is the only way to make a pin, as there (Pin Tab has no default shortcut).
fn right_clicked(window: HWND, x: i32, y: i32) {
    let Some((Item::Tab(id) | Item::Pin(id), _)) = hit(x, y) else { return };
    let Some((pinned, private, can_join, in_split)) = state(|s| {
        let front = s.tabs.selected_id();
        let pinned = |t: u64| s.tabs.get(t).is_some_and(|t| t.pinned);
        (
            pinned(id),
            s.private.contains_key(&id),
            !pinned(id) && !pinned(front) && s.splits.can_join(id, front),
            s.splits.split_of(id).is_some(),
        )
    }) else {
        return;
    };
    const PIN: usize = 1;
    const CLOSE: usize = 2;
    const JOIN: usize = 3;
    const SEPARATE: usize = 4;
    let split_on = split::on();
    let pt = portuguese();
    let pin_label = match (pinned, pt) {
        (false, true) => "Fixar aba",
        (false, false) => "Pin Tab",
        (true, true) => "Desafixar aba",
        (true, false) => "Unpin Tab",
    };
    let close_label = if pt { "Fechar aba" } else { "Close Tab" };
    let chosen = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        // A private tab leaves nothing behind, and a pin is kept in the session: it can't be one.
        let pin_flags = if private { MF_STRING | MF_GRAYED } else { MF_STRING };
        let _ = AppendMenuW(menu, pin_flags, PIN, &HSTRING::from(pin_label));
        // Split View's two, only with it on (upstream's tab menu).
        if split_on {
            let flags = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };
            let join = if pt { "Dividir com a página atual" } else { "Split with Current Page" };
            let separate = if pt { "Separar abas divididas" } else { "Separate Split Tabs" };
            let _ = AppendMenuW(menu, flags(can_join), JOIN, &HSTRING::from(join));
            let _ = AppendMenuW(menu, flags(in_split), SEPARATE, &HSTRING::from(separate));
        }
        let _ = AppendMenuW(menu, MF_STRING, CLOSE, &HSTRING::from(close_label));
        let mut point = POINT { x, y };
        let _ = ClientToScreen(window, &mut point);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, point.x, point.y, None, window, None);
        let _ = DestroyMenu(menu);
        chosen.0 as usize
    };
    match chosen {
        PIN => toggle_pin(id),
        CLOSE => request_close(id),
        JOIN => {
            split::join(id);
        }
        SEPARATE => split::separate(id),
        _ => {}
    }
}

pub fn toggle_pin(id: u64) {
    let changed = state(|s| {
        if s.private.contains_key(&id) {
            return false;
        }
        let pinned = s.tabs.get(id).is_some_and(|t| t.pinned);
        // A pin is never a page beside another.
        if !pinned {
            s.splits.remove(id);
        }
        let changed = if pinned { s.tabs.unpin(id) } else { s.tabs.pin(id) };
        if changed {
            s.session_owed = true;
        }
        changed
    });
    if changed == Some(true) {
        place_pages();
        repaint_card();
        repaint_column();
    }
}

fn wheel(window: HWND, wparam: WPARAM, lparam: LPARAM) {
    let delta = (wparam.0 >> 16) as u16 as i16 as i32;
    let mut point = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
    unsafe {
        let _ = ScreenToClient(window, &mut point);
    }
    let bottom = client_rect().bottom;
    let moved = state(|s| {
        if point.x >= scale(COLUMN, s.dpi) {
            return false;
        }
        // One notch, three rows.
        let most = column(s, bottom).most;
        let old = s.scroll.clamp(0, most);
        s.scroll = (old - delta * 3 * scale(ROW, s.dpi) / 120).clamp(0, most);
        s.scroll != old
    })
    .unwrap_or(false);
    if moved {
        mouse_moved(window, point.x, point.y);
        repaint_column();
    }
}

/// The tab in front, scrolled into sight.
fn reveal_selected() {
    let bottom = client_rect().bottom;
    state(|s| {
        let front = s.tabs.selected_id();
        let selected = Item::Tab(s.splits.split_of(front).map_or(front, |split| split.first()));
        let column = column(s, bottom);
        let Some(row) = column.rows.iter().find(|r| r.item == selected) else { return };
        let scroll = s.scroll.clamp(0, column.most);
        let list = column.list;
        if row.rect.top < list.top {
            s.scroll = scroll - (list.top - row.rect.top);
        } else if row.rect.bottom > list.bottom {
            s.scroll = (scroll + row.rect.bottom - list.bottom).min(column.most);
        }
    });
}

/// No caption: the client area runs to the window's top. Left, right and
/// bottom keep Windows' own frame, so the window still has its shadow and its
/// resize edges. Maximized, Windows hangs the frame off the screen; the top is
/// brought back in by as much, so nothing is cut off.
unsafe fn calc_size(window: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let params = lparam.0 as *mut NCCALCSIZE_PARAMS;
        let top = (*params).rgrc[0].top;
        let result = DefWindowProcW(window, WM_NCCALCSIZE, wparam, lparam);
        if result.0 != 0 {
            return result;
        }
        (*params).rgrc[0].top = top;
        if zoomed(window) {
            (*params).rgrc[0].top += frame_thickness(GetDpiForWindow(window) as i32);
        }
        LRESULT(0)
    }
}

/// What part of the window the mouse is on, for Windows: edges resize, the
/// empty bar and the column's top strip drag, the maximize button is Windows'
/// own (Snap Layouts appear over it), and the rest is ours.
unsafe fn frame_hit(window: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let frame = unsafe { DefWindowProcW(window, WM_NCHITTEST, wparam, lparam) };
    if frame.0 as u32 != HTCLIENT {
        return frame;
    }
    let mut point = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
    unsafe {
        let _ = ScreenToClient(window, &mut point);
    }
    let client = client_rect();
    let Some((dpi, item)) = state(|s| (s.dpi, bar_hit(s, client, point.x, point.y))) else { return frame };
    // The top edge's grip lies inside the client area now there is no caption.
    let grip = frame_thickness(dpi);
    if !zoomed(window) && point.y < grip {
        let corner = 2 * grip;
        return LRESULT(if point.x < corner {
            HTTOPLEFT
        } else if point.x >= client.right - corner {
            HTTOPRIGHT
        } else {
            HTTOP
        } as isize);
    }
    if point.y >= scale(BAR, dpi) {
        return frame;
    }
    LRESULT(match item {
        Some(Item::Maximize) => HTMAXBUTTON,
        Some(_) => HTCLIENT,
        None => HTCAPTION,
    } as isize)
}

extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            WM_NCCALCSIZE if wparam.0 != 0 => calc_size(window, wparam, lparam),
            WM_NCHITTEST => frame_hit(window, wparam, lparam),
            WM_NCMOUSEMOVE => {
                frame_moved(window, wparam.0 as u32 == HTMAXBUTTON);
                DefWindowProcW(window, message, wparam, lparam)
            }
            WM_NCMOUSELEAVE => {
                let had = state(|s| {
                    s.tracking_frame = false;
                    s.hover == Some(Item::Maximize) && s.hover.take().is_some()
                });
                if had == Some(true) {
                    repaint_bar();
                }
                LRESULT(0)
            }
            // Left to Windows, the maximize button would be drawn in the old style over ours.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 as u32 == HTMAXBUTTON => {
                state(|s| s.pressed = Some(Item::Maximize));
                repaint_bar();
                LRESULT(0)
            }
            WM_NCLBUTTONUP if wparam.0 as u32 == HTMAXBUTTON => {
                if state(|s| s.pressed.take()) == Some(Some(Item::Maximize)) {
                    toggle_maximized(window);
                }
                repaint_bar();
                LRESULT(0)
            }
            WM_SIZE => {
                if wparam.0 as u32 == SIZE_MINIMIZED {
                    page::set_visible(false);
                } else {
                    page::set_visible(true);
                    layout();
                    peek::place();
                    forms::place();
                }
                LRESULT(0)
            }
            WM_MOVE | WM_MOVING => {
                page::window_moved();
                place_drop();
                peek::place();
                forms::place();
                DefWindowProcW(window, message, wparam, lparam)
            }
            // Another app in front, and back: the video that follows you
            // (float::app_left), when Settings asks for it.
            WM_ACTIVATEAPP => {
                if wparam.0 == 0 {
                    float::app_left();
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            WM_ACTIVATE => {
                if (wparam.0 & 0xffff) as u32 != WA_INACTIVE {
                    float::app_back();
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            float::WM_FLOAT => {
                float::toggled();
                LRESULT(0)
            }
            peek::WM_PEEK => {
                peek::act(wparam, lparam);
                LRESULT(0)
            }
            passwords::WM_PASSWORDS => {
                passwords::act();
                LRESULT(0)
            }
            import::WM_IMPORT => {
                import::act();
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(window);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                mouse_moved(window, x, y);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                let had = state(|s| {
                    s.tracking_mouse = false;
                    s.hover_close = false;
                    s.pressed = None;
                    // Gone onto the maximize button: that hover is the frame's to keep.
                    s.hover != Some(Item::Maximize) && s.hover.take().is_some()
                });
                if had == Some(true) {
                    repaint_column();
                    repaint_bar();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                clicked(window, x, y);
                LRESULT(0)
            }
            // A double click on a split's divider evens the pages out; anywhere
            // else it is a second click like the first.
            WM_LBUTTONDBLCLK => {
                match split::divider_at(x, y) {
                    Some(_) => split::even_out(),
                    None => clicked(window, x, y),
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                split::end_drag();
                LRESULT(0)
            }
            // Over a split's divider, the pointer says it can be dragged.
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = ScreenToClient(window, &mut point);
                let dragging = state(|s| s.drag.is_some()) == Some(true);
                if (dragging || split::divider_at(point.x, point.y).is_some())
                    && let Ok(cursor) = LoadCursorW(None, IDC_SIZEWE)
                {
                    SetCursor(Some(cursor));
                    return LRESULT(1);
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            // The mouse's side buttons over the column and the bar: the space
            // before or after, with spaces on (upstream's swipe).
            WM_XBUTTONUP => {
                let button = if ((wparam.0 >> 16) & 0xffff) as u16 == 1 { VK_XBUTTON1 } else { VK_XBUTTON2 };
                if shortcut_with(button.0 as u32, false, false, false) {
                    return LRESULT(1);
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            spaces::WM_SETTLE => {
                spaces::settle();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                released(window, x, y);
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                middle_clicked(x, y);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                right_clicked(window, x, y);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                wheel(window, wparam, lparam);
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                let hdc = HDC(wparam.0 as _);
                match state(|s| (s.palette, s.field_brush)) {
                    Some((palette, brush)) => {
                        SetTextColor(hdc, color(palette.text));
                        SetBkColor(hdc, color(palette.field));
                        LRESULT(brush.0 as isize)
                    }
                    None => DefWindowProcW(window, message, wparam, lparam),
                }
            }
            WM_COMMAND => {
                match ((wparam.0 >> 16) & 0xffff) as u32 {
                    EN_SETFOCUS => field_focused(),
                    EN_KILLFOCUS => {
                        hide_suggestions();
                        show_address();
                        // The field goes back into the bar's words.
                        if let Some(field) = state(|s| s.field) {
                            let _ = ShowWindow(field, SW_HIDE);
                        }
                        repaint_bar();
                    }
                    EN_CHANGE if state(|s| s.quiet) == Some(false) => suggest(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let suggested = &*(lparam.0 as *const RECT);
                let dpi = (wparam.0 & 0xffff) as i32;
                if let Some(old) = state(|s| {
                    s.dpi = dpi;
                    [
                        std::mem::replace(&mut s.font, make_font(dpi, FONT)),
                        std::mem::replace(&mut s.small_font, make_font(dpi, SMALL_FONT)),
                        std::mem::replace(&mut s.crumb_font, make_font(dpi, CRUMB_FONT)),
                        std::mem::replace(&mut s.glyphs, make_glyph_font(dpi, GLYPH)),
                        std::mem::replace(&mut s.caption_glyphs, make_glyph_font(dpi, CAPTION_GLYPH)),
                    ]
                }) {
                    apply_font();
                    for font in old {
                        let _ = DeleteObject(font.into());
                    }
                }
                favicon::forget_sizes();
                let _ = SetWindowPos(
                    window,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                layout();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let dark = theme_dark();
                let changed = state(|s| {
                    if s.dark == dark {
                        return None;
                    }
                    s.dark = dark;
                    s.palette = Palette::new(dark);
                    Some(std::mem::replace(&mut s.field_brush, CreateSolidBrush(color(s.palette.field))))
                })
                .flatten();
                if let Some(old) = changed {
                    let _ = DeleteObject(old.into());
                    apply_theme();
                    peek::repaint();
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            drive::WM_BENCH => {
                drive::drain(window);
                LRESULT(0)
            }
            WM_CLOSE_TAB => {
                close_tab(wparam.0 as u64);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == drive::WAIT_TIMER => {
                drive::tick(window);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == downloads::PUSH_TIMER => {
                downloads::push();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == SLEEP_TIMER => {
                sleep_idle();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == SAVE_TIMER => {
                let _ = KillTimer(Some(window), SAVE_TIMER);
                save();
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(window), SAVE_TIMER);
                let _ = KillTimer(Some(window), SLEEP_TIMER);
                save();
                page::close_all();
                // Not owned by the window, so it would outlive it.
                float::close_all();
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}

/// The drop-down sits under the field, as wide as it, as tall as its rows;
/// hidden when there is nothing to offer.
fn place_drop() {
    let client = client_rect();
    let Some((window, drop, dpi, count)) = state(|s| (s.window, s.drop, s.dpi, s.suggestions.len())) else { return };
    unsafe {
        if count == 0 || IsIconic(window).as_bool() {
            let _ = ShowWindow(drop, SW_HIDE);
            return;
        }
        let field = bar_spots(client, dpi).field;
        let mut corner = POINT { x: field.left, y: scale(BAR, dpi) + scale(2, dpi) };
        let _ = ClientToScreen(window, &mut corner);
        let height = count as i32 * scale(SUGGESTION_ROW, dpi) + 2 * scale(DROP_PAD, dpi);
        let _ = SetWindowPos(
            drop,
            None,
            corner.x,
            corner.y,
            field.right - field.left,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(drop), None, false);
    }
}

/// The drop-down's row at x, y in its own coordinates.
fn drop_hit(x: i32, y: i32) -> Option<usize> {
    let drop = state(|s| s.drop)?;
    let mut area = RECT::default();
    unsafe {
        let _ = GetClientRect(drop, &mut area);
    }
    state(|s| drop_rows(s, area.right).iter().position(|r| inside(r.rect, x, y))).flatten()
}

fn paint_drop(drop: HWND) {
    let mut area = RECT::default();
    unsafe {
        let _ = GetClientRect(drop, &mut area);
    }
    let Some((rows, palette, font, small_font, dpi)) =
        state(|s| (drop_rows(s, area.right), s.palette, s.font, s.small_font, s.dpi))
    else {
        return;
    };
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(drop, &mut paint);
        buffered(hdc, area, |canvas, area| {
            // Windows 11 rounds the window and gives it its shadow; the hairline is ours.
            rounded(canvas, area, palette.card, palette.border, 0);
            let radius = scale(6, dpi);
            let icon = scale(ICON, dpi);
            for row in &rows {
                let r = row.rect;
                if row.selected {
                    rounded(canvas, r, palette.bar_hover, palette.bar_hover, radius);
                }
                let spot = RECT { left: r.left + scale(10, dpi), right: r.left + scale(10, dpi) + icon, ..centred(r, icon) };
                let behind = if row.selected { palette.bar_hover } else { palette.card };
                site_mark(canvas, row, spot, behind, &palette, small_font, dpi);
                let middle = r.top + (r.bottom - r.top) / 2;
                let inner = RECT { left: spot.right + scale(12, dpi), right: r.right - scale(10, dpi), ..r };
                text(canvas, &row.label, RECT { top: r.top + scale(4, dpi), bottom: middle + scale(2, dpi), ..inner }, font, palette.text, false);
                text(canvas, &row.detail, RECT { top: middle, bottom: r.bottom - scale(4, dpi), ..inner }, small_font, palette.dim, false);
            }
        });
        let _ = EndPaint(drop, &paint);
    }
}

extern "system" fn drop_proc(drop: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            // A click here leaves the keyboard in the field.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_drop(drop);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let row = drop_hit(x, y);
                let changed = state(|s| std::mem::replace(&mut s.drop_hover, row) != row) == Some(true);
                track_leave(drop, TME_LEAVE);
                if changed {
                    let _ = InvalidateRect(Some(drop), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if state(|s| s.drop_hover.take().is_some()) == Some(true) {
                    let _ = InvalidateRect(Some(drop), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(i) = drop_hit(x, y) {
                    pick_suggestion(i);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(drop, message, wparam, lparam),
        }
    }
}

/// A row of the drop-down, chosen: after Ctrl+K its tab, otherwise its page.
fn pick_suggestion(i: usize) {
    let switch = state(|s| s.switching.then(|| s.switch_ids.get(i).copied()).flatten()).flatten();
    if let Some(id) = switch {
        select_tab(id);
    } else if let Some(url) = state(|s| s.suggestions.get(i).map(|x| x.url.clone())).flatten() {
        go(&url);
    }
}

extern "system" fn field_proc(field: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_KEYDOWN if wparam.0 as u16 == VK_RETURN.0 => {
                enter();
                return LRESULT(0);
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                escape();
                return LRESULT(0);
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_UP.0 || wparam.0 as u16 == VK_DOWN.0 => {
                step_highlight(if wparam.0 as u16 == VK_UP.0 { -1 } else { 1 });
                return LRESULT(0);
            }
            // Return and Escape are handled above; as characters they would only beep.
            WM_CHAR if matches!(wparam.0, 0x0a | 0x0d | 0x1b) => return LRESULT(0),
            // Ctrl+A, which a plain edit control doesn't know.
            WM_CHAR if wparam.0 == 0x01 => {
                SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                return LRESULT(0);
            }
            // A letter typed (never a deletion) may finish the address from history.
            WM_CHAR if wparam.0 >= 0x20 && wparam.0 != 0x7f => {
                let previous = state(|s| s.field_proc).flatten();
                let result = CallWindowProcW(previous, field, message, wparam, lparam);
                complete_inline();
                return result;
            }
            _ => {}
        }
        let previous = state(|s| s.field_proc).flatten();
        CallWindowProcW(previous, field, message, wparam, lparam)
    }
}

pub fn held(key: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

/// The window's own keys, whether the field or the page has the keyboard.
pub fn shortcut(key: u32) -> bool {
    let (ctrl, alt) = (held(VK_CONTROL), held(VK_MENU));
    // AltGr reaches Windows as Ctrl+Alt and types a character (² on ABNT2):
    // never taken for one of the Ctrl+Alt keys.
    if ctrl && alt && held(VK_RMENU) && !held(VK_LMENU) {
        return false;
    }
    shortcut_with(key, ctrl, alt, held(VK_SHIFT))
}

/// The same keys with their modifiers given, as `bench key` sends them.
pub fn shortcut_with(key: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
    let key = key as u16;
    let letter = |c: u8| key == u16::from(c);
    let command = ctrl && !alt;
    let both = ctrl && alt && !shift;
    let digit = (u16::from(b'1')..=u16::from(b'9')).contains(&key);
    if !ctrl && !alt && (key == VK_ESCAPE.0 || key == VK_RETURN.0) && bookmarks::card_key(key == VK_RETURN.0) {
        // The bookmark's card has the keys: Enter and Esc are its, never the page's.
        return true;
    }
    if both && digit {
        // Spaces (upstream's ⌃1–9); with them off, the keys are the page's.
        return spaces::select_number(usize::from(key - u16::from(b'0')));
    } else if both && letter(b'N') {
        return split::split_current();
    } else if both && letter(b'L') {
        // Upstream's ⌥⌘L: on Windows Ctrl+Alt, the keys Settings' search leaves free.
        open_page("search://passwords");
        return true;
    } else if both && (key == VK_LEFT.0 || key == VK_RIGHT.0) {
        return split::focus_step(key == VK_LEFT.0);
    } else if !ctrl && !alt && (key == VK_XBUTTON1.0 || key == VK_XBUTTON2.0) {
        // The side buttons, as WM_XBUTTONUP hands them on (and `bench key xbutton1`).
        return spaces::uses_spaces() && {
            spaces::step(if key == VK_XBUTTON1.0 { -1 } else { 1 });
            true
        };
    }
    if command && !shift && letter(b'L') {
        focus_field();
    } else if !ctrl && !alt && key == VK_ESCAPE.0 && (peek::escape() || forms::escape() || veil::escape()) {
        // Esc puts a peek or the accounts' list away, or stops the picker; otherwise it is the page's, or the field's.
    } else if command && !shift && key == VK_RETURN.0 && peek::keep() {
        // Ctrl+Enter keeps a peek as a tab (upstream's ⌘↩); otherwise it is the page's.
    } else if command && shift && letter(b'P') {
        float::toggle();
    } else if command && shift && letter(b'B') {
        bookmarks::bookmark_front();
    } else if command && shift && letter(b'O') {
        // Where every other browser on Windows keeps its bookmarks.
        open_page("search://bookmarks");
    } else if command && !shift && letter(b'J') {
        open_page("search://downloads");
    } else if command && shift && letter(b'N') {
        new_private_tab();
    } else if command && !shift && letter(b'K') {
        switch_tabs();
    } else if command && shift && letter(b'H') {
        veil::toggle_picker();
    } else if command && shift && letter(b'U') {
        veil::toggle_list();
    } else if command && shift && letter(b'R') {
        veil::toggle_reader();
    } else if (command && letter(b'R')) || key == VK_F5.0 {
        page::reload();
    } else if command && letter(b'T') {
        if shift { reopen() } else { new_tab() }
    } else if command && !shift && letter(b'W') {
        if let Some(id) = state(|s| s.tabs.selected_id()) {
            request_close(id);
        }
    } else if command && key == VK_TAB.0 {
        if tabs(|t| if shift { t.select_previous() } else { t.select_next() }).is_some() {
            show_selected();
        }
    } else if command && !shift && (u16::from(b'1')..=u16::from(b'9')).contains(&key) {
        let number = usize::from(key - u16::from(b'0'));
        if tabs(|t| t.select_number(number)).flatten().is_some() {
            show_selected();
        }
    } else if command && !shift && key == VK_OEM_COMMA.0 {
        open_page("search://settings");
    } else if command && !shift && letter(b'H') {
        // Upstream's ⌘Y; on Windows every browser has history on Ctrl+H.
        open_page("search://history");
    } else if alt && !ctrl && key == VK_LEFT.0 {
        page::back();
    } else if alt && !ctrl && key == VK_RIGHT.0 {
        page::forward();
    } else {
        return false;
    }
    true
}

/// A key `bench key` sends that isn't a shortcut goes to the field, if the field
/// has the keyboard, exactly as the keyboard would deliver it there.
pub fn field_key(key: u32, ctrl: bool, alt: bool, shift: bool) -> bool {
    let Some(field) = state(|s| s.field) else { return false };
    if ctrl || alt || unsafe { GetFocus() } != field {
        return false;
    }
    let code = key as u16;
    let typed = |c: u16| unsafe {
        SendMessageW(field, WM_CHAR, Some(WPARAM(c as usize)), Some(LPARAM(0)));
    };
    match code {
        _ if [VK_RETURN.0, VK_ESCAPE.0, VK_UP.0, VK_DOWN.0].contains(&code) => unsafe {
            SendMessageW(field, WM_KEYDOWN, Some(WPARAM(key as usize)), Some(LPARAM(0)));
        },
        _ if code == VK_BACK.0 => typed(0x08),
        _ if code == VK_SPACE.0 => typed(u16::from(b' ')),
        _ if code == VK_OEM_PERIOD.0 => typed(u16::from(b'.')),
        0x30..=0x39 => typed(code),
        0x41..=0x5A => typed(if shift { code } else { code + 0x20 }),
        _ => return false,
    }
    true
}

fn field_text() -> String {
    let Some(field) = state(|s| s.field) else { return String::new() };
    unsafe {
        let mut buffer = vec![0u16; GetWindowTextLengthW(field) as usize + 1];
        let length = GetWindowTextW(field, &mut buffer);
        String::from_utf16_lossy(&buffer[..length as usize])
    }
}

/// The window's own writing in the field: never taken for typing.
fn set_field_text(text: &str) {
    if let Some(field) = state(|s| {
        s.quiet = true;
        s.field
    }) {
        unsafe {
            let _ = SetWindowTextW(field, &HSTRING::from(text));
        }
    }
    state(|s| s.quiet = false);
}

/// What was typed, as an address: words go to the engine chosen in Settings.
fn resolve(typed: &str) -> Option<url::Url> {
    internal::prefs(|p| omnibox::resolve(typed, p.engine, &p.custom_engine))
}

/// What was typed, resolved, in the tab in front.
pub fn go(typed: &str) {
    let Some(url) = resolve(typed) else { return };
    let Some(id) = state(|s| {
        s.suggestions.clear();
        s.switching = false;
        s.highlight = None;
        let id = s.tabs.selected_id();
        if let Some(tab) = s.tabs.get_mut(id) {
            tab.url = Some(url.to_string());
            tab.loading = true;
        }
        s.session_owed = true;
        id
    }) else {
        return;
    };
    page::show(id, None);
    page::navigate(id, url.as_str());
    page::focus();
    repaint_column();
    repaint_bar();
    place_drop();
    save_soon();
}

/// A link from the command line: in a tab of its own in front, unless the one
/// in front is empty and can take it.
fn open_typed(typed: &str) {
    let Some(url) = resolve(typed) else {
        show_selected();
        return;
    };
    let blank = state(|s| s.tabs.selected().is_blank()).unwrap_or(true);
    if blank {
        go(typed);
        return;
    }
    let url = url.to_string();
    tabs(|t| {
        let before = t.selected_id();
        let id = t.open_in_front(&url, None);
        // The tab it covered never got a page: it is asleep, not just behind.
        t.mark_asleep(before);
        if let Some(tab) = t.get_mut(id) {
            tab.loading = true;
        }
    });
    show_selected();
}

/// One of our own pages. As upstream, one tab of each per window: asking
/// again goes to the one already open.
pub fn open_page(address: &str) {
    // The page reports itself back as search://settings/.
    let same = |url: &str| url.trim_end_matches('/') == address;
    let open = state(|s| s.tabs.iter().find(|t| t.url.as_deref().is_some_and(same)).map(|t| t.id)).flatten();
    if let Some(id) = open {
        select_tab(id);
    } else if state(|s| s.tabs.selected().is_blank()).unwrap_or(true) {
        go(address);
    } else {
        tabs(|t| t.open_in_front(address, None));
        show_selected();
    }
}

/// What a person does in the field, done for a script: typed letter by letter
/// (so it finishes from history as it would for them), then Return.
pub fn type_and_go(text: &str) {
    focus_field();
    let Some(field) = state(|s| s.field) else { return };
    for unit in text.encode_utf16() {
        unsafe {
            SendMessageW(field, WM_CHAR, Some(WPARAM(unit as usize)), Some(LPARAM(0)));
        }
    }
    enter();
}

/// Return: the row picked with the arrows, else what is in the field.
/// After Ctrl+K: the tab picked; words no open tab has, a tab of their own,
/// so the page Ctrl+K was pressed over stays as it was.
fn enter() {
    if let Some((switching, picked)) = state(|s| (s.switching, s.highlight.and_then(|i| s.switch_ids.get(i).copied())))
        && switching
    {
        match picked {
            Some(id) => select_tab(id),
            None => {
                let typed = field_text();
                hide_suggestions();
                if !typed.trim().is_empty() {
                    open_typed(&typed);
                }
            }
        }
        return;
    }
    let picked = state(|s| s.highlight.and_then(|i| s.suggestions.get(i)).map(|x| x.url.clone())).flatten();
    go(&picked.unwrap_or_else(field_text));
}

/// Ctrl+K: the field lists the open tabs by name, the last one used already
/// picked, so Ctrl+K then Return goes back to it. Again, it is put away.
fn switch_tabs() {
    if state(|s| s.switching) == Some(true) {
        escape();
        return;
    }
    focus_field();
    state(|s| s.switching = true);
    set_field_text("");
    suggest();
}

/// Escape: the tabs come back, and the keyboard goes back to the page.
fn escape() {
    hide_suggestions();
    let id = state(|s| s.tabs.selected_id()).unwrap_or_default();
    if page::has_page(id) {
        page::focus();
    } else if let Some(field) = state(|s| s.field) {
        unsafe { SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1))) };
    }
}

fn hide_suggestions() {
    let had = state(|s| {
        s.highlight = None;
        s.switching = false;
        s.switch_ids.clear();
        !std::mem::take(&mut s.suggestions).is_empty()
    });
    if had == Some(true) {
        place_drop();
    }
}

/// While something is typed, the drop-down under the field offers where it
/// could go; after Ctrl+K, which of the open tabs to go to.
fn suggest() {
    let typed = field_text();
    state(|s| {
        s.highlight = None;
        s.hover = None;
        if s.switching {
            let (ids, rows) = open_tabs(s, &typed).into_iter().unzip();
            s.switch_ids = ids;
            s.suggestions = rows;
            s.highlight = (!s.suggestions.is_empty()).then_some(0);
            return;
        }
        s.suggestions =
            if typed.trim().is_empty() { Vec::new() } else { history(s).suggest(&typed, SUGGESTIONS) };
    });
    // Bookmarks answer from two letters, as upstream's places do.
    if state(|s| s.switching) == Some(false) && typed.trim().chars().count() >= 2 {
        let (found, slips) = bookmarks::places(&typed, SUGGESTIONS);
        state(|s| {
            let visited = std::mem::take(&mut s.suggestions);
            s.suggestions = with_bookmarks(visited, found, slips, SUGGESTIONS);
        });
    }
    place_drop();
}

/// One page, however its address is written: no scheme, no www., no lone
/// trailing slash (upstream's History.identity), so a page that is both a
/// bookmark and somewhere you have been is one row.
fn identity(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let host = address::without_www(u.host_str().unwrap_or("")).to_lowercase();
            let query = u.query().map(|q| format!("?{q}")).unwrap_or_default();
            format!("{host}{}{query}", u.path().trim_end_matches('/'))
        }
        Err(_) => url.to_string(),
    }
}

/// The rows under the field, as upstream's Browser.suggestions orders them:
/// your bookmarks first (a place you chose to keep), then where you have
/// been, then a bookmark found only by forgiving a typo. History's best row
/// keeps a place if a bookmark doesn't already take it: it is where the
/// field's grey ending points.
fn with_bookmarks(
    visited: Vec<Suggestion>,
    found: Vec<searchwin_bookmarks::Place>,
    slips: Vec<searchwin_bookmarks::Place>,
    limit: usize,
) -> Vec<Suggestion> {
    let mark = |p: searchwin_bookmarks::Place| {
        let key = url::Url::parse(&p.url).map(|u| address::pretty(&u)).unwrap_or_else(|_| p.url.clone());
        Suggestion { key, title: p.title, url: p.url, kind: searchwin_history::Kind::Visited }
    };
    let front = visited.first().map(|v| identity(&v.url));
    let mut list: Vec<Suggestion> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |row: Suggestion, list: &mut Vec<Suggestion>| {
        if list.len() < limit && seen.insert(identity(&row.url)) {
            list.push(row);
        }
    };
    for place in found {
        let id = identity(&place.url);
        if list.len() + 1 == limit && front.as_ref().is_some_and(|f| *f != id && !list.iter().any(|r| identity(&r.url) == *f)) {
            break;
        }
        add(mark(place), &mut list);
    }
    for row in visited {
        add(row, &mut list);
    }
    for place in slips {
        add(mark(place), &mut list);
    }
    list
}

/// The rest of the best address, after the caret and selected, so the next
/// letter typed replaces it (History.complete).
fn complete_inline() {
    // After Ctrl+K the words name a tab, not an address to finish.
    let Some(field) = state(|s| (!s.switching).then_some(s.field)).flatten() else { return };
    let typed = field_text();
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(
            field,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
    }
    let length = typed.encode_utf16().count() as u32;
    // Only at the end: finishing an address in its middle would write over it.
    if start != length || end != length {
        return;
    }
    let Some(rest) = state(|s| history(s).complete(&typed)).flatten() else { return };
    set_field_text(&format!("{typed}{rest}"));
    unsafe {
        SendMessageW(field, EM_SETSEL, Some(WPARAM(length as usize)), Some(LPARAM(-1)));
    }
}

fn step_highlight(by: isize) {
    let moved = state(|s| {
        let count = s.suggestions.len() as isize;
        if count == 0 {
            return false;
        }
        s.highlight = Some(match s.highlight {
            None if by > 0 => 0,
            None => count - 1,
            Some(at) => (at as isize + by).rem_euclid(count),
        } as usize);
        true
    });
    if moved == Some(true) {
        place_drop();
    }
}

pub fn loading() -> bool {
    state(|s| s.tabs.selected().loading).unwrap_or(false)
}

pub fn quit() {
    if let Some(window) = state(|s| s.window) {
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
}

/// The tab in front, in the card: its page (woken if it slept), its address, its title.
fn show_selected() {
    let Some((window, id, url, title, wakes)) = state(|s| {
        s.suggestions.clear();
        s.switching = false;
        s.highlight = None;
        s.session_owed = true;
        let tab = s.tabs.selected();
        let (id, url, title) = (tab.id, tab.url.clone(), tab.title.clone());
        // In a split, the page in front is the one with the keys.
        s.splits.set_focused(id);
        let wakes = url.is_some() && !page::has_page(id);
        if wakes && let Some(tab) = s.tabs.get_mut(id) {
            tab.loading = true;
        }
        (s.window, id, url, title, wakes)
    }) else {
        return;
    };
    // A peek goes when another tab comes in front (upstream, with "Peek stays
    // with its tab" off); one being kept is no longer a peek by now.
    peek::close();
    // The accounts' list goes; the offer to save shows only over its own tab.
    forms::place();
    // Gone back to the tab whose video floats: the video comes home to it.
    if float::tab() == Some(id) && page::front() != Some(id) {
        float::toggle();
    }
    // The pages beside it in its split, placed first: they stay on screen as it comes in front.
    let beside: Vec<(u64, String)> = state(|s| {
        let Some(split) = s.splits.split_of(id) else { return Vec::new() };
        let others: Vec<u64> = split.tabs.iter().copied().filter(|&t| t != id).collect();
        others
            .into_iter()
            .filter_map(|t| {
                let tab = s.tabs.get_mut(t)?;
                tab.asleep = false;
                Some((t, tab.url.clone()?))
            })
            .collect()
    })
    .unwrap_or_default();
    // Only when a split is or just was on screen: with Split View off this costs nothing.
    let split = state(|s| !s.splits.is_empty()).unwrap_or(false);
    if split || page::has_panes() {
        place_pages();
        repaint_card();
    }
    for (other, url) in beside {
        page::wake(other, &url);
    }
    page::show(id, url.as_deref());
    set_title(&title);
    if url.is_none() {
        focus_field();
    } else if wakes {
        // Its page is still being made; it takes the keyboard when it is.
        unsafe {
            let _ = SetFocus(Some(window));
        }
    } else {
        page::focus();
    }
    show_address();
    reveal_selected();
    repaint_column();
    repaint_bar();
    place_drop();
    save_soon();
}

fn select_tab(id: u64) {
    if tabs(|t| t.select(id)) == Some(true) {
        show_selected();
    }
}

/// Ctrl+T. From a private tab the new one is private too: a page that keeps
/// nothing, then one that keeps everything, is how a private search ends up in the history.
fn new_tab() {
    let private = state(|s| s.private.contains_key(&s.tabs.selected_id())).unwrap_or(false);
    open_blank(private);
}

/// Ctrl+Shift+N. A tab that keeps nothing: its own cookies, its own sign-ins,
/// no history, and no place in tomorrow's session.
fn new_private_tab() {
    open_blank(true);
}

/// An empty tab in front. An empty one already there is reused (Tabs::new_tab)
/// and becomes what was asked for: it has no page yet, so nothing it held carries over.
fn open_blank(private: bool) {
    state(|s| {
        s.tabs.set_now(now_ms());
        let id = s.tabs.new_tab();
        if !private {
            s.private.remove(&id);
        } else if !s.private.contains_key(&id) {
            s.privates_made += 1;
            s.private.insert(id, format!("Private {}", s.privates_made));
        }
    });
    show_selected();
}

/// The InPrivate profile a private tab's page is made in; None for any other tab.
pub fn private_profile(id: u64) -> Option<String> {
    state(|s| s.private.get(&id).cloned()).flatten()
}

pub fn is_private(id: u64) -> bool {
    state(|s| s.private.contains_key(&id)).unwrap_or(false)
}

/// The tab in front and where it is.
pub fn front_tab() -> Option<(u64, Option<String>)> {
    state(|s| {
        let tab = s.tabs.selected();
        (tab.id, tab.url.clone())
    })
}

/// The page in front, as Ctrl+Shift+B keeps it: its address and its title.
pub fn front_page() -> Option<(String, String)> {
    state(|s| {
        let tab = s.tabs.selected();
        Some((tab.url.clone()?, tab.title.clone()))
    })
    .flatten()
}

pub fn window() -> Option<HWND> {
    state(|s| s.window)
}

/// Whether `id` is a tab of the row on screen.
pub fn has_tab(id: u64) -> bool {
    state(|s| s.tabs.get(id).is_some()).unwrap_or(false)
}

pub fn dpi() -> i32 {
    state(|s| s.dpi).unwrap_or(96)
}

pub fn is_dark() -> bool {
    state(|s| s.dark).unwrap_or(false)
}

/// The window's colours and fonts, for the windows of ours beside it (a peek's strip).
pub struct Look {
    palette: Palette,
    font: HFONT,
    small_font: HFONT,
    crumb_font: HFONT,
    glyphs: HFONT,
    caption_glyphs: HFONT,
}

pub fn look() -> Option<Look> {
    state(|s| Look {
        palette: s.palette,
        font: s.font,
        small_font: s.small_font,
        crumb_font: s.crumb_font,
        glyphs: s.glyphs,
        caption_glyphs: s.caption_glyphs,
    })
}

/// The card, where the page is, on the screen.
pub fn card_on_screen() -> Option<RECT> {
    let client = client_rect();
    let (window, dpi) = state(|s| (s.window, s.dpi))?;
    let card = card_rect(client, dpi);
    let (mut corner, mut far) = (POINT { x: card.left, y: card.top }, POINT { x: card.right, y: card.bottom });
    unsafe {
        let _ = ClientToScreen(window, &mut corner);
        let _ = ClientToScreen(window, &mut far);
    }
    Some(RECT { left: corner.x, top: corner.y, right: far.x, bottom: far.y })
}

/// A peek kept: a tab after `from`, in front, already holding the page at
/// `url` (its page comes over from the peek, so nothing is asked for). From
/// a private tab it is private, in the same profile, as a link opened from it.
pub fn open_kept(from: u64, url: &str, title: &str) -> Option<u64> {
    state(|s| {
        s.tabs.set_now(now_ms());
        let id = s.tabs.open_in_front(url, Some(from));
        let private = s.private.get(&from).cloned();
        if let Some(tab) = s.tabs.get_mut(id) {
            tab.title = title.to_string();
            tab.loading = false;
        }
        match private {
            Some(profile) => {
                s.private.insert(id, profile);
            }
            // Somewhere you have been, as any page opened in a tab.
            None => {
                if history(s).record_visit(url, title, now_ms()) {
                    s.history_owed = true;
                }
            }
        }
        s.session_owed = true;
        id
    })
}

/// A bookmark, or a whole folder of them: the first in front, the rest
/// beside it, asleep, so ten sites cost one page until you go to them.
pub fn open_in_tabs(urls: &[String]) {
    let Some((first, rest)) = urls.split_first() else { return };
    state(|s| {
        s.tabs.set_now(now_ms());
        let front = s.tabs.open_in_front(first, None);
        let mut after = front;
        for url in rest {
            after = s.tabs.open_in_background(url, Some(after));
            s.tabs.mark_asleep(after);
        }
        s.tabs.select(front);
    });
    show_selected();
}

fn reopen() {
    if tabs(|t| t.reopen_closed()).flatten().is_some() {
        show_selected();
    }
}

/// Closing waits for the message loop: Ctrl+W pressed in a page arrives inside
/// that page's own event, and its page must not be torn down under it.
fn request_close(id: u64) {
    if let Some(window) = state(|s| s.window) {
        unsafe {
            let _ = PostMessageW(Some(window), WM_CLOSE_TAB, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn close_tab(id: u64) {
    let Some(window) = state(|s| s.window) else { return };
    // The last tab, already empty: closing it is closing the window.
    if state(|s| s.tabs.closes_window(id)) == Some(true) {
        unsafe {
            let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return;
    }
    let goes_back = internal::prefs(|p| p.close_goes_back);
    let Some((was_front, moved)) = state(|s| {
        s.tabs.set_now(now_ms());
        s.tabs.options.close_goes_back = goes_back;
        // A private tab leaves nothing behind, not even for Ctrl+Shift+T: emptied
        // first, it is nothing to come back to. If it was the last tab, it stays
        // as the empty one a window always has, no longer private.
        if s.private.remove(&id).is_some()
            && let Some(tab) = s.tabs.get_mut(id)
        {
            tab.url = None;
            tab.title.clear();
        }
        let was_front = s.tabs.selected_id() == id;
        // A page of a split: the others take its room, and the keys go to the
        // one beside it (Browser.close, Split.swift).
        let partner = match s.splits.remove(id) {
            searchwin_split::Removed::Shrunk { partner } => Some(partner),
            searchwin_split::Removed::Dissolved { remaining } => Some(remaining),
            searchwin_split::Removed::None => None,
        };
        let mut moved = s.tabs.close(id).is_some();
        if was_front && let Some(partner) = partner.filter(|p| s.tabs.get(*p).is_some()) {
            s.tabs.select(partner);
            s.splits.set_focused(partner);
            moved = true;
        }
        (was_front, moved)
    }) else {
        return;
    };
    page::discard(id);
    veil::forget(id);
    if was_front || moved {
        show_selected();
    } else {
        state(|s| s.session_owed = true);
        repaint_column();
        save_soon();
    }
}

/// A link that asked for a new window: a tab beside its page, behind it when Ctrl was held.
/// From a private tab, the new one is private too, and shares its cookies:
/// a sign-in followed through a link stays signed in.
fn open_link(url: &str, background: bool, opener: u64, popup: bool) {
    // Sent to the space a rule names, when it is another one: never from a
    // private tab, and never a pop-up, which belongs with its page (SpaceRoutes.swift:11-20).
    if !popup
        && !is_private(opener)
        && let Some(space) = spaces::sends_to(url)
    {
        spaces::send(url, &space);
        return;
    }
    let Some(id) = state(|s| {
        s.tabs.set_now(now_ms());
        let id = if background { s.tabs.open_in_background(url, Some(opener)) } else { s.tabs.open_in_front(url, Some(opener)) };
        if let Some(profile) = s.private.get(&opener).cloned() {
            s.private.insert(id, profile);
        }
        if background && let Some(tab) = s.tabs.get_mut(id) {
            tab.loading = true;
        }
        id
    }) else {
        return;
    };
    if background {
        // It loads behind, so it is ready when you get to it.
        page::navigate(id, url);
        state(|s| s.session_owed = true);
        repaint_column();
        save_soon();
    } else {
        show_selected();
    }
}

/// Tabs left alone long enough give their page back. They keep their place,
/// address and title, and wake when selected.
fn sleep_idle() {
    let now = now_ms();
    let pace = internal::prefs(|p| match p.sleep {
        searchwin_prefs::Sleep::Later => SleepPace::Later,
        searchwin_prefs::Sleep::Normal => SleepPace::Normal,
        searchwin_prefs::Sleep::Sooner => SleepPace::Sooner,
    });
    // A private tab never sleeps: its page is all there is of it, and would come back empty.
    // Nor does a page on screen beside the one in front, nor the floating video's.
    let floating = float::tab();
    let due: Vec<u64> = state(|s| {
        let due = s.tabs.due_to_sleep(now, pace.wait_ms());
        let front = s.tabs.selected_id();
        due.into_iter()
            .filter(|id| !s.private.contains_key(id) && !s.splits.is_visible(*id, front) && Some(*id) != floating)
            .collect()
    })
    .unwrap_or_default();
    if due.is_empty() {
        return;
    }
    for id in due {
        page::discard(id);
        state(|s| s.tabs.mark_asleep(id));
    }
    repaint_column();
}

/// Everywhere you have been, newest first, for search://history.
pub fn history_list(matching: &str) -> Vec<searchwin_history::Trace> {
    state(|s| history(s).entries(matching)).unwrap_or_default()
}

/// Forgotten here is forgotten by the field's suggestions too: they are one list.
pub fn history_remove(url: &str) -> bool {
    let removed = state(|s| {
        let removed = history(s).remove(url);
        s.history_owed |= removed;
        removed
    }) == Some(true);
    if removed {
        save_soon();
    }
    removed
}

pub fn history_clear() {
    state(|s| {
        history(s).clear();
        s.history_owed = true;
    });
    save_soon();
}

/// Session and history are written once changes settle, not on every one.
pub fn save_soon() {
    if let Some(window) = state(|s| s.window) {
        unsafe {
            SetTimer(Some(window), SAVE_TIMER, SAVE_AFTER_MS, None);
        }
    }
}

/// The session as written: no private tab is in it. With the tab in front a
/// private one, the one kept just before it comes back in front, as upstream's
/// Browser.sessionShape does.
fn without_private(mut shape: Shape, tabs: &Tabs, private: &HashMap<u64, String>) -> Shape {
    let kept: Vec<bool> = tabs.iter().map(|t| !private.contains_key(&t.id)).collect();
    if kept.iter().all(|k| *k) || kept.len() != shape.tabs.len() {
        return shape;
    }
    if let Ok(front) = usize::try_from(shape.active)
        && front < kept.len()
    {
        let before = kept[..front].iter().filter(|k| **k).count();
        shape.active = if kept[front] { before } else { before.saturating_sub(1) } as i64;
    }
    let mut place = kept.iter();
    shape.tabs.retain(|_| *place.next().unwrap_or(&true));
    shape
}

fn save() {
    let folder = data_folder();
    let Some(session) = state(|s| {
        let owed = std::mem::take(&mut s.session_owed);
        // A space just deleted has no file to write: it is being cleared away.
        let gone = s.spaces.space(&s.row_of).is_none();
        (owed && !gone).then(|| {
            let mut shape = without_private(Shape::from_tabs(&s.tabs), &s.tabs, &s.private);
            if !s.splits.is_empty() || split::on() {
                // Splits by place among the tabs written: no private tab is.
                let order: Vec<u64> = s.tabs.iter().map(|t| t.id).filter(|id| !s.private.contains_key(id)).collect();
                shape.splits = searchwin_split::to_session(&s.splits, &order);
            }
            (searchwin_spaces::session_path(&folder, &s.row_of), shape)
        })
    }) else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    if let Some((path, shape)) = session
        && let Err(error) = searchwin_tabs::save(&path, &shape)
    {
        crash_log(&format!("session not saved: {error}"));
    }
    // History is the profile on screen's.
    let own = spaces::folder();
    let written = state(|s| {
        if !std::mem::take(&mut s.history_owed) {
            return Ok(());
        }
        s.history.as_ref().map_or(Ok(()), |h| h.save(&own.join("history.json")))
    });
    if let Some(Err(error)) = written {
        crash_log(&format!("history not saved: {error}"));
    }
    bookmarks::save();
    downloads::save();
}

/// The window as a script sees it: only what is on screen, private tabs marked.
pub fn bench_state() -> serde_json::Value {
    let Some((selected, url, title, loading, field_focused, tabs, suggestions, highlight, switching)) = state(|s| {
        let tab = s.tabs.selected();
        let tabs: Vec<serde_json::Value> = s
            .tabs
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "title": t.title,
                    "url": t.url,
                    "pinned": t.pinned,
                    "asleep": t.asleep,
                    "selected": t.id == tab.id,
                    "private": s.private.contains_key(&t.id),
                    "icon": favicon::has(&host_of(t.url.as_deref())),
                })
            })
            .collect();
        let suggestions: Vec<serde_json::Value> = s
            .suggestions
            .iter()
            .map(|x| serde_json::json!({ "title": x.title, "address": x.key, "url": x.url }))
            .collect();
        (
            tab.id,
            tab.url.clone(),
            tab.title.clone(),
            tab.loading,
            unsafe { GetFocus() } == s.field,
            tabs,
            suggestions,
            s.highlight,
            s.switching,
        )
    }) else {
        return serde_json::Value::Null;
    };
    let page_kind = url.as_deref().map(|u| if internal::is_internal(u) { "internal" } else { "web" });
    let mut value = serde_json::json!({
        "url": url,
        "page_kind": page_kind,
        "title": title,
        "loading": loading,
        "field": field_text(),
        "field_focused": field_focused,
        "has_page": page::has_page(selected),
        "blocked": shield::blocked(),
        "tabs": tabs,
    });
    value["downloading"] = serde_json::json!(downloads::active());
    value["float"] = float::bench();
    value["peek"] = peek::bench();
    value["bookmark_card"] = bookmarks::card_bench();
    value["forms"] = forms::bench();
    value["passwords"] = passwords::bench();
    value["import"] = import::bench();
    // Spaces and profiles as the window has them, and the split in front as laid out.
    let client = client_rect();
    if let Some((spaces, split)) = state(|s| {
        let card = card_rect(client, s.dpi);
        let rect = |r: RECT| serde_json::json!([r.left, r.top, r.right - r.left, r.bottom - r.top]);
        let front = s.tabs.selected_id();
        let mut split = serde_json::json!({
            "on": split::on(),
            "splits": s.splits.list.iter().map(|x| x.tabs.clone()).collect::<Vec<_>>(),
            "focused": front,
            "card": rect(card),
        });
        if let Some(layout) = split::layout_of(s, card) {
            split["paired"] = serde_json::json!(layout.paired);
            split["dividers"] = layout.dividers.iter().map(|d| rect(split::win(*d))).collect();
            split["panes"] = split::panes(s, card)
                .iter()
                .map(|(id, r)| serde_json::json!({ "id": id, "rect": rect(*r), "focused": *id == front }))
                .collect();
        }
        (spaces::bench(&s.spaces, &s.row_of), split)
    }) {
        value["spaces"] = spaces;
        value["split"] = split;
    }
    // The bar as drawn: what back and forward can do, the page's place, and
    // whether the field and the drop-down are out.
    let (back, forward) = page::can_go();
    if let Some((field, drop, (site, title))) = state(|s| (s.field, s.drop, crumb(s.tabs.selected()))) {
        let shown = |w: HWND| unsafe { IsWindowVisible(w).as_bool() };
        value["bar"] = serde_json::json!({
            "back": back,
            "forward": forward,
            "site": site,
            "title": title,
            "field_shown": shown(field),
            "drop_shown": shown(drop),
        });
    }
    if !suggestions.is_empty() {
        // Which rows are bookmarks: the window draws them alike, a script needs to tell.
        let mut suggestions = suggestions;
        for row in &mut suggestions {
            let kept = row["url"].as_str().is_some_and(|u| !u.is_empty() && bookmarks::contains(u));
            row["bookmark"] = serde_json::json!(kept);
        }
        value["suggestions"] = serde_json::Value::Array(suggestions);
        value["highlight"] = serde_json::json!(highlight);
        value["switching"] = serde_json::json!(switching);
    }
    value
}

fn focus_field() {
    if let Some(field) = state(|s| s.field) {
        // Out of the bar's words and into its place, then the keyboard.
        unsafe {
            let _ = ShowWindow(field, SW_SHOW);
            let _ = SetFocus(Some(field));
        }
        field_focused();
    }
}

fn page_url() -> Option<url::Url> {
    let url = state(|s| s.tabs.selected().url.clone()).flatten()?;
    url::Url::parse(&url).ok().filter(|url| url.scheme() != "about")
}

fn field_focused() {
    set_field_text(&page_url().map(|url| address::editable(&url)).unwrap_or_default());
    if let Some(field) = state(|s| s.field) {
        unsafe { SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1))) };
    }
    repaint_bar();
}

fn show_address() {
    let Some(field) = state(|s| s.field) else { return };
    if unsafe { GetFocus() } == field {
        return;
    }
    set_field_text(&page_url().map(|url| address::pretty(&url)).unwrap_or_default());
}

/// What a tab's page reports. News from a tab already closed is dropped.
pub fn page_event(id: page::TabId, event: page::Event) {
    let Some((front, known)) = state(|s| (s.tabs.selected_id() == id, s.tabs.get(id).is_some())) else { return };
    if !known {
        return;
    }
    match event {
        page::Event::Moved(url) => {
            if url.is_empty() {
                return;
            }
            state(|s| {
                if let Some(tab) = s.tabs.get_mut(id) {
                    tab.url = Some(url);
                }
                s.session_owed = true;
            });
            if front {
                show_address();
                repaint_bar();
            }
            repaint_column();
            save_soon();
        }
        page::Event::Started => {
            state(|s| s.tabs.get_mut(id).map(|tab| tab.loading = true));
        }
        page::Event::Title(title) => {
            state(|s| {
                let tab = s.tabs.get_mut(id)?;
                tab.title = title.clone();
                let url = tab.url.clone()?;
                s.session_owed = true;
                // A private tab is nowhere you have been.
                if !s.private.contains_key(&id) && history(s).set_title(&url, &title) {
                    s.history_owed = true;
                }
                Some(())
            });
            if front {
                set_title(&title);
                repaint_bar();
            }
            repaint_column();
            save_soon();
        }
        page::Event::Loaded(ok) => {
            // A sign-in sent from this tab is judged by the page it led to.
            forms::loaded(id);
            let visit = state(|s| {
                let tab = s.tabs.get_mut(id)?;
                tab.loading = false;
                let visit = (tab.url.clone()?, tab.title.clone());
                (!s.private.contains_key(&id)).then_some(visit)
            })
            .flatten();
            if front {
                report_launch();
                if let Some(window) = state(|s| s.window) {
                    drive::loaded(window);
                }
            }
            // Only a page that came: an error page is nowhere you have been.
            if ok && let Some((url, title)) = visit {
                state(|s| {
                    if history(s).record_visit(&url, &title, now_ms()) {
                        s.history_owed = true;
                    }
                });
                save_soon();
            }
        }
        page::Event::NewWindow { url, background, popup } => open_link(&url, background, id, popup),
        page::Event::Focused => {
            // A click in the page beside the one in front: it is in front now.
            if !front && state(|s| s.splits.is_visible(id, s.tabs.selected_id())) == Some(true) {
                select_tab(id);
            }
        }
        page::Event::History => {
            if front {
                repaint_bar();
            }
        }
        page::Event::Icon(png) => {
            let host = state(|s| host_of(s.tabs.get(id).and_then(|t| t.url.as_deref()))).unwrap_or_default();
            favicon::store(&host, png);
            repaint_column();
        }
    }
}

fn set_title(title: &str) {
    if let Some(window) = state(|s| s.window) {
        let title = if title.is_empty() { "Search" } else { title };
        unsafe {
            let _ = SetWindowTextW(window, &HSTRING::from(title));
        }
    }
}

/// With SEARCHWIN_TIMING=1, the time from process start to the first page loaded,
/// appended to launch.log beside the app's data. Nothing is ever sent anywhere.
pub fn report_launch() {
    if state(|s| std::mem::replace(&mut s.timed, true)) != Some(false) {
        return;
    }
    if std::env::var("SEARCHWIN_TIMING").as_deref() != Ok("1") {
        return;
    }
    let ticks = |t: FILETIME| (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64;
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    let elapsed_ms = unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user).is_err() {
            return;
        }
        (ticks(GetSystemTimeAsFileTime()) - ticks(created)) / 10_000
    };
    let folder = data_folder();
    let _ = std::fs::create_dir_all(&folder);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("launch.log")) {
        let _ = writeln!(file, "first page {elapsed_ms} ms");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four tabs, a, b, c, d; `private` are the places of the private ones, `front` the one in front.
    fn row(private: &[usize], front: usize) -> (Shape, Tabs, HashMap<u64, String>) {
        let mut tabs = Tabs::new();
        tabs.get_mut(tabs.selected_id()).unwrap().url = Some("https://a.test/".into());
        for name in ["b", "c", "d"] {
            tabs.open_in_front(&format!("https://{name}.test/"), None);
        }
        let ids: Vec<u64> = tabs.iter().map(|t| t.id).collect();
        tabs.select(ids[front]);
        let private = private.iter().map(|&i| (ids[i], "Private 1".to_string())).collect();
        (Shape::from_tabs(&tabs), tabs, private)
    }

    fn urls(shape: &Shape) -> Vec<&str> {
        shape.tabs.iter().map(|e| e.url.as_str()).collect()
    }

    #[test]
    fn private_tabs_are_never_written() {
        let (shape, tabs, private) = row(&[1, 3], 2);
        let written = without_private(shape, &tabs, &private);
        assert_eq!(urls(&written), ["https://a.test/", "https://c.test/"]);
        assert_eq!(written.active, 1);
    }

    #[test]
    fn a_private_tab_in_front_hands_it_to_the_one_kept_before() {
        let (shape, tabs, private) = row(&[2], 2);
        let written = without_private(shape, &tabs, &private);
        assert_eq!(urls(&written), ["https://a.test/", "https://b.test/", "https://d.test/"]);
        assert_eq!(written.active, 1);
        let (shape, tabs, private) = row(&[0], 0);
        assert_eq!(without_private(shape, &tabs, &private).active, 0);
    }

    #[test]
    fn without_private_tabs_the_session_is_untouched() {
        let (shape, tabs, private) = row(&[], 3);
        let before = shape.clone();
        assert_eq!(without_private(shape, &tabs, &private), before);
    }

    fn visit(url: &str) -> Suggestion {
        Suggestion { key: url.into(), title: String::new(), url: url.into(), kind: searchwin_history::Kind::Visited }
    }

    fn place(url: &str) -> searchwin_bookmarks::Place {
        searchwin_bookmarks::Place { title: "kept".into(), url: url.into() }
    }

    #[test]
    fn bookmarks_come_first_and_a_page_is_one_row() {
        let rows = with_bookmarks(
            vec![visit("https://a.test/"), visit("https://b.test/x")],
            vec![place("https://b.test/x/"), place("https://c.test/")],
            vec![place("https://d.test/")],
            8,
        );
        let urls: Vec<&str> = rows.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, ["https://b.test/x/", "https://c.test/", "https://a.test/", "https://d.test/"]);
    }

    #[test]
    fn history_front_door_keeps_a_place() {
        let rows = with_bookmarks(vec![visit("https://a.test/")], vec![place("https://b.test/"), place("https://c.test/")], vec![], 2);
        let urls: Vec<&str> = rows.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(urls, ["https://b.test/", "https://a.test/"]);
    }

    /// WCAG's contrast ratio between two colours.
    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let light = |(r, g, b): Rgb| {
            let one = |c: u8| {
                let c = c as f64 / 255.0;
                if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * one(r) + 0.7152 * one(g) + 0.0722 * one(b)
        };
        let (x, y) = (light(a), light(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn every_word_is_readable() {
        for dark in [false, true] {
            let p = Palette::new(dark);
            for (ink, ground) in [
                (p.text, p.background),
                (p.dim, p.background),
                (p.faint, p.background),
                (p.dim, p.hover),
                (p.text, p.pill),
                (p.dim, p.card),
                (p.faint, p.card),
                (p.text, p.field),
            ] {
                let ratio = contrast(ink, ground);
                assert!(ratio >= 4.5, "dark {dark}: {ink:?} on {ground:?} is {ratio:.2}");
            }
        }
    }

    #[test]
    fn the_bar_says_site_then_title() {
        let mut tabs = Tabs::new();
        let tab = tabs.get_mut(tabs.selected_id()).unwrap();
        assert_eq!(crumb(tab), (String::new(), String::new()));
        tab.url = Some("https://www.wikipedia.org/wiki/Web".into());
        tab.title = "Web - Wikipedia".into();
        assert_eq!(crumb(tab), ("wikipedia.org".into(), "Web - Wikipedia".into()));
        tab.url = Some("search://settings/".into());
        tab.title = "search://settings".into();
        assert_eq!(crumb(tab), ("search://settings".into(), String::new()));
    }

    #[test]
    fn accents_are_forgiven() {
        let fold = |t: &str| t.to_lowercase().chars().map(plain_letter).collect::<String>();
        assert_eq!(fold("Ação Rápida"), "acao rapida");
    }
}
