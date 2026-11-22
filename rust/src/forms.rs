//! Sign-in boxes (upstream Forms.swift, and Browser.swift's `hang`, `choose`,
//! `onCredentials`): your saved accounts under the box you click, the way
//! Safari does, and an offer to keep one that just worked. Nothing is ever
//! filled on its own.
//!
//! The page's side is assets/js/forms.js, a document-start script on every
//! page. It says where the caret's box is, when a sign-in goes out, and when
//! its boxes go away; it never receives a saved password, and never posts
//! one. Everything it says comes over the channel any page can post on, so
//! nothing is taken on trust (page.rs hears it only from a document that was
//! given the script; a frame only through the frame itself):
//!
//! - accounts are chosen by the vault for the tab's own page, as the app
//!   knows it, never for an address the message names: its exact host
//!   alone, and a frame of another origin than the page's gets nothing
//!   (vault::offers);
//! - the list is a window of ours, over the page: the page never sees it;
//!   kept inside the page's place, and a click on it counts only once it has
//!   sat still a moment, down and up on the same row;
//! - a pick fills only if the page is still the one it was offered on
//!   (Offered::may_fill), and only the document that was offered — the
//!   script checks its own address before it fills;
//! - the password just sent is taken by the app from that document (a script
//!   of ours, once, within seconds), never from a message, and held in
//!   memory until the sign-in is judged; a private tab's is never taken.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use searchwin_vault::{GivenName, Offered, SaveContext, SaveOffer, Secret, Sent, Settle, Store, now_ms};
use serde_json::{Value, json};
use url::Url;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Frame2;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::w;

use crate::page::{self, TabId};

/// What a document was given, as page.rs keeps it, beside veil's and peek's.
pub const FORMS_IN: u8 = 16;

const SCRIPT: &str = include_str!("../assets/js/forms.js");

// In device-independent pixels.
const ROW: i32 = 44;
const PAD: i32 = 6;
const LIST_W: i32 = 300;
const BAR_H: i32 = 44;
const KNOB_H: i32 = 30;
/// A click on the list or the save bar counts only this long after it
/// appeared or moved: a page can't slide one under a click meant for
/// something else.
const CLICK_AFTER: Duration = Duration::from_millis(500);

/// What a press counts for (a row, a button): nothing too soon after it showed.
fn pressed<T>(hit: Option<T>, shown_at: Option<Instant>, now: Instant) -> Option<T> {
    hit.filter(|_| shown_at.is_some_and(|at| now.saturating_duration_since(at) >= CLICK_AFTER))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Knob {
    Save,
    Never,
    Close,
}

/// The list up under a box.
#[derive(Clone)]
struct Shown {
    tab: TabId,
    /// The frame the box is in; None for the page itself.
    frame: Option<ICoreWebView2Frame2>,
    /// The document the box is in, as it was when the list went up.
    doc: Url,
    /// A first step's name box: a pick puts the name alone.
    name: bool,
    offered: Offered,
}

/// The offer to keep a sign-in that worked.
struct Ask {
    tab: TabId,
    sent: Sent,
    changed: bool,
}

#[derive(Default)]
struct Forms {
    registered: bool,
    list: HWND,
    bar: HWND,
    shown: Option<Shown>,
    hot: Option<usize>,
    /// When the list last appeared or moved, and the row the button went down on.
    shown_at: Option<Instant>,
    down: Option<usize>,
    /// The same for the save bar.
    bar_at: Option<Instant>,
    bar_down: Option<Knob>,
    bar_hot: Option<Knob>,
    /// Per tab: the name sent on a first step, and a sign-in sent, not yet judged.
    given: HashMap<TabId, GivenName>,
    sent: HashMap<TabId, Sent>,
    ask: Option<Ask>,
    /// The boxes went in place: judged again then (vault::SETTLE_AFTER_MS).
    later: Option<(usize, TabId)>,
    // For the bench.
    refused: &'static str,
    filled: usize,
    saved: usize,
}

thread_local! {
    static FORMS: RefCell<Forms> = RefCell::new(Forms::default());
}

/// Never hold the borrow across a Win32 or WebView2 call: both come back in.
fn with<R>(f: impl FnOnce(&mut Forms) -> R) -> R {
    FORMS.with(|forms| f(&mut forms.borrow_mut()))
}

pub fn start_script() -> String {
    SCRIPT.to_string()
}

/// Where tab `id`'s page is, as the page itself says now.
fn page_url(id: TabId) -> Option<Url> {
    page::address_and_title(id).and_then(|(u, _)| Url::parse(&u).ok())
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// A message from a web page in tab `id` (`frame` when a frame posted it),
/// which page.rs has checked was given forms.js. False if it isn't ours.
pub fn message(id: TabId, frame: Option<ICoreWebView2Frame2>, source: &str, json: &str) -> bool {
    let Ok(asked) = serde_json::from_str::<Value>(json) else { return false };
    let Some(op) = asked.get("op").and_then(Value::as_str).filter(|op| op.starts_with("forms.")) else { return false };
    // A peek's page signs in once it is kept as a tab.
    if !crate::has_tab(id) {
        return true;
    }
    let (Some(page), Ok(doc)) = (page_url(id), Url::parse(source)) else { return true };
    let top = frame.is_none();
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default().chars().take(256).collect::<String>();
    match op {
        "forms.focus" => focus(id, frame, doc, page, &asked),
        "forms.name" if top => {
            if let Some(given) = GivenName::new(&page, &text("user"), now_ms()) {
                with(|f| f.given.insert(id, given));
            }
        }
        "forms.sent" if top => sent(id, page, text("user")),
        "forms.settled" if top => settled(id),
        _ => {}
    }
    true
}

fn refused(why: &'static str) {
    with(|f| f.refused = why);
}

fn focus(id: TabId, frame: Option<ICoreWebView2Frame2>, doc: Url, page: Url, asked: &Value) {
    let rect = asked.get("rect").filter(|r| r.is_object());
    let number = |key: &str| rect.and_then(|r| r.get(key)).and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() < 100_000.0);
    let (Some(x), Some(y), Some(w), Some(h)) = (number("x"), number("y"), number("w"), number("h")) else {
        hide_list();
        return;
    };
    // `name` only narrows (a pick then puts the name alone); the page has no
    // say in which hosts' accounts come (vault::kept_matching).
    let name = asked.get("name").and_then(Value::as_bool).unwrap_or(false);
    let now = now_ms();
    let given = with(|f| f.given.get(&id).and_then(|g| g.for_page(&page, now)).map(str::to_string));
    let field = searchwin_vault::Field { page: &page, frame: frame.is_some().then_some(&doc), given_name: given.as_deref() };
    let offered = match searchwin_vault::offers(&crate::passwords::store(), &field) {
        Ok(Some(offered)) if !offered.accounts.is_empty() => offered,
        Ok(Some(_)) => return nothing("nothing kept here"),
        Ok(None) => return nothing("not the page's site"),
        Err(_) => return nothing("store unreadable"),
    };
    let Some((under, room)) = page::to_screen(id, x, y, w, h) else { return nothing("not on screen") };
    with(|f| {
        f.refused = "";
        f.shown = Some(Shown { tab: id, frame, doc, name, offered });
        f.hot = None;
    });
    show_list(under, room);
}

fn nothing(why: &'static str) {
    refused(why);
    hide_list();
}

fn register() {
    if with(|f| std::mem::replace(&mut f.registered, true)) {
        return;
    }
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        for (proc, name) in [(list_proc as WindowProc, w!("SearchWinOffers")), (bar_proc as WindowProc, w!("SearchWinSaveBar"))] {
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), hCursor: arrow, lpszClassName: name, ..Default::default() });
        }
    }
}

type WindowProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// Owned by the window, so it stays above its pages and goes with it; it
/// never takes the keyboard from the box.
fn make(class: windows_core::PCWSTR) -> HWND {
    register();
    let Some(main) = crate::window() else { return HWND::default() };
    let made = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(main),
            None,
            GetModuleHandleW(None).ok().map(|h| h.into()),
            None,
        )
    };
    let window = made.unwrap_or_default();
    if !window.is_invalid() {
        let round = DWMWCP_ROUND;
        unsafe {
            let _ = DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
        }
    }
    window
}

/// Under the box, kept inside `room` (the tab's page on screen): a page can't
/// put the list over the window's own controls.
fn show_list(under: RECT, room: RECT) {
    let mut list = with(|f| f.list);
    if list.is_invalid() {
        list = make(w!("SearchWinOffers"));
        with(|f| f.list = list);
    }
    let rows = with(|f| {
        // Appeared or moved: clicks wait (CLICK_AFTER_MS).
        f.shown_at = Some(Instant::now());
        f.down = None;
        f.shown.as_ref().map_or(0, |s| s.offered.accounts.len())
    }) as i32;
    let dpi = crate::dpi();
    let width = (under.right - under.left).max(crate::scale(LIST_W, dpi)).min(room.right - room.left);
    let height = (rows * crate::scale(ROW, dpi) + 2 * crate::scale(PAD, dpi)).min(room.bottom - room.top);
    let left = under.left.min(room.right - width).max(room.left);
    let top = (under.bottom + crate::scale(4, dpi)).min(room.bottom - height).max(room.top);
    unsafe {
        let _ = SetWindowPos(list, Some(HWND_TOP), left, top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(list), None, false);
    }
}

fn hide_list() {
    let list = with(|f| {
        f.shown = None;
        f.hot = None;
        f.list
    });
    if !list.is_invalid() {
        unsafe {
            let _ = ShowWindow(list, SW_HIDE);
        }
    }
}

/// Esc with the list up: it goes. False with none.
pub fn escape() -> bool {
    let up = with(|f| f.shown.is_some());
    hide_list();
    up
}

/// A row chosen: the account goes into the box it was offered for, if the
/// page is still that one.
fn pick(row: usize) {
    let Some(shown) = with(|f| f.shown.clone()) else { return };
    hide_list();
    let Some(kept) = shown.offered.accounts.get(row).cloned() else { return };
    if !page_url(shown.tab).is_some_and(|now| shown.offered.may_fill(&now)) {
        return refused("the page moved");
    }
    let host = searchwin_vault::page_host(&shown.doc).unwrap_or_default();
    let (scheme, host, user) = (quoted(shown.doc.scheme()), quoted(&host), quoted(&kept.user));
    // The document is checked here, not only by __officeForms (which a page
    // without our script could have made): location can't be faked, and ===
    // and + can't be patched the way String methods can. location.host is
    // already lowercase, with the port when it isn't the scheme's.
    let here = format!(
        "location.protocol==={scheme}+':'&&(location.host==={host}||location.host==='www.'+{host})&&!!window.__officeForms"
    );
    let js = if shown.name {
        Secret::new(format!("!!({here}&&__officeForms.name({scheme},{host},{user}))"))
    } else {
        let Ok(Some(account)) = crate::passwords::store().account(&kept.host, &kept.user) else { return refused("not kept any more") };
        let password = Secret::new(quoted(account.password.expose()));
        Secret::new(format!("!!({here}&&__officeForms.fill({scheme},{host},{user},{}))", password.expose()))
    };
    let done = |result: Result<String, String>| {
        if result.as_deref() == Ok("true") {
            with(|f| f.filled += 1);
        } else {
            refused("the box wasn't there");
        }
    };
    match &shown.frame {
        Some(frame) => page::run_in_frame(frame, js.expose(), done),
        None => page::run(shown.tab, js.expose(), done),
    }
}

/// A sign-in went out: its password is taken from that document now, held
/// until the page shows whether it worked. A private tab never keeps one.
fn sent(id: TabId, page: Url, user: String) {
    if crate::is_private(id) {
        return refused("private tab");
    }
    page::run(id, "window.__officeForms?__officeForms.take():null", move |result| {
        let Ok(text) = result else { return };
        let text = Secret::new(text);
        let Ok(Some(password)) = serde_json::from_str::<Option<String>>(text.expose()) else { return };
        let password = Secret::new(password);
        if password.is_empty() {
            return;
        }
        let now = now_ms();
        let given = with(|f| f.given.get(&id).and_then(|g| g.for_page(&page, now)).map(str::to_string));
        if let Some(sent) = Sent::new(&page, &user, password, given.as_deref(), now) {
            with(|f| f.sent.insert(id, sent));
        }
    });
}

/// The boxes went away without a new page: judged again a moment later.
fn settled(id: TabId) {
    if with(|f| f.sent.contains_key(&id)) {
        let timer = unsafe { SetTimer(None, 0, searchwin_vault::SETTLE_AFTER_MS as u32, Some(settle_later)) };
        let old = with(|f| f.later.replace((timer, id)));
        if let Some((old, _)) = old {
            unsafe {
                let _ = KillTimer(None, old);
            }
        }
    }
}

unsafe extern "system" fn settle_later(_: HWND, _: u32, timer: usize, _: u32) {
    unsafe {
        let _ = KillTimer(None, timer);
    }
    if let Some((_, id)) = with(|f| f.later.take_if(|(t, _)| *t == timer)) {
        judge(id);
    }
}

/// Tab `id`'s page finished loading: a sign-in held for it is judged.
pub fn loaded(id: TabId) {
    if with(|f| f.sent.contains_key(&id)) {
        judge(id);
    }
}

/// Whether the page still has a password box decides it (vault::settle).
fn judge(id: TabId) {
    page::run(id, "!!(window.__officeForms&&__officeForms.hasPassword())", move |result| {
        let has = result.as_deref() == Ok("true");
        let now = now_ms();
        let verdict = with(|f| f.sent.get(&id).map(|s| searchwin_vault::settle(s, now, true, has)));
        match verdict {
            Some(Settle::Drop) => {
                with(|f| f.sent.remove(&id));
            }
            Some(Settle::Worked) => {
                if let Some(sent) = with(|f| f.sent.remove(&id)) {
                    offer_save(id, sent);
                }
            }
            _ => {}
        }
    });
}

fn offer_save(id: TabId, sent: Sent) {
    let never = crate::passwords::never();
    let ctx = SaveContext { saves_passwords: true, private_tab: crate::is_private(id), extension_page: false, never: &never };
    let mut store = crate::passwords::store();
    match searchwin_vault::should_offer_save(&store, &sent, &ctx) {
        Ok(SaveOffer::Touch) => {
            let _ = searchwin_vault::touch(&mut store, &sent.host, &sent.user, sent.clear, now_ms());
        }
        Ok(SaveOffer::Offer { changed }) => {
            with(|f| f.ask = Some(Ask { tab: id, sent, changed }));
            place();
        }
        Ok(SaveOffer::Nothing) => refused("not offered"),
        Err(_) => refused("store unreadable"),
    }
}

fn answer_bar(knob: Knob) {
    let Some(ask) = with(|f| f.ask.take()) else { return };
    match knob {
        Knob::Save => {
            if searchwin_vault::keep_sent(&mut crate::passwords::store(), &ask.sent, now_ms()).is_ok() {
                with(|f| f.saved += 1);
            } else {
                refused("not saved");
            }
        }
        Knob::Never => crate::passwords::add_never(searchwin_vault::registrable(&ask.sent.host)),
        Knob::Close => {}
    }
    place();
}

/// The window moved, changed size or another tab came in front: the list
/// goes, and the offer to save shows over its own tab's page alone.
pub fn place() {
    hide_list();
    let front = crate::front_tab().map(|(id, _)| id);
    let (mut bar, wanted) = with(|f| (f.bar, f.ask.as_ref().is_some_and(|a| Some(a.tab) == front)));
    let card = crate::card_on_screen().filter(|_| wanted);
    let Some(card) = card else {
        if !bar.is_invalid() {
            unsafe {
                let _ = ShowWindow(bar, SW_HIDE);
            }
        }
        return;
    };
    if bar.is_invalid() {
        bar = make(w!("SearchWinSaveBar"));
        with(|f| f.bar = bar);
    }
    let height = crate::scale(BAR_H, crate::dpi());
    with(|f| {
        f.bar_at = Some(Instant::now());
        f.bar_down = None;
    });
    unsafe {
        let _ = SetWindowPos(bar, Some(HWND_TOP), card.left, card.top, card.right - card.left, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(bar), None, false);
    }
}

/// A tab closed or slept: what it held goes (a held password is wiped as it drops).
pub fn forget(id: TabId) {
    let (list_up, asked) = with(|f| {
        f.given.remove(&id);
        f.sent.remove(&id);
        let asked = f.ask.as_ref().is_some_and(|a| a.tab == id);
        if asked {
            f.ask = None;
        }
        (f.shown.as_ref().is_some_and(|s| s.tab == id), asked)
    });
    if list_up {
        hide_list();
    }
    if asked {
        place();
    }
}

fn label(knob: Knob) -> &'static str {
    match (knob, crate::portuguese()) {
        (Knob::Save, false) => "Save",
        (Knob::Save, true) => "Salvar",
        (Knob::Never, false) => "Never for this site",
        (Knob::Never, true) => "Nunca neste site",
        (Knob::Close, _) => "\u{E8BB}",
    }
}

/// The bar's buttons, right to left: close, never, save.
fn knob_rect(bar: HWND, knob: Knob) -> RECT {
    let dpi = crate::dpi();
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(bar, &mut client);
    }
    let s = |v| crate::scale(v, dpi);
    let top = (s(BAR_H) - s(KNOB_H)) / 2;
    let widths = [(Knob::Close, s(30)), (Knob::Never, s(150)), (Knob::Save, s(80))];
    let mut right = client.right - s(8);
    for (k, w) in widths {
        if k == knob {
            return RECT { left: right - w, top, right, bottom: top + s(KNOB_H) };
        }
        right -= w + s(6);
    }
    RECT::default()
}

fn knob_at(bar: HWND, x: i32, y: i32) -> Option<Knob> {
    [Knob::Save, Knob::Never, Knob::Close].into_iter().find(|k| crate::inside(knob_rect(bar, *k), x, y))
}

fn paint_bar(bar: HWND) {
    let Some(look) = crate::look() else { return };
    let (words, hot) = with(|f| {
        let words = f.ask.as_ref().map(|a| {
            let who = if a.sent.user.is_empty() { String::new() } else { format!(" ({})", a.sent.user) };
            match (a.changed, crate::portuguese()) {
                (false, false) => format!("Save the password for {}{who}?", a.sent.host),
                (true, false) => format!("Update the password for {}{who}?", a.sent.host),
                (false, true) => format!("Salvar a senha de {}{who}?", a.sent.host),
                (true, true) => format!("Atualizar a senha de {}{who}?", a.sent.host),
            }
        });
        (words.unwrap_or_default(), f.bar_hot)
    });
    let dpi = crate::dpi();
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(bar, &mut client);
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(bar, &mut paint);
        crate::buffered(hdc, client, |canvas, area| {
            crate::fill(canvas, area, look.palette.card);
            crate::fill(canvas, RECT { top: area.bottom - 1, ..area }, look.palette.border);
            let save = knob_rect(bar, Knob::Save);
            let room = RECT { left: area.left + crate::scale(14, dpi), right: save.left - crate::scale(8, dpi), ..area };
            crate::text(canvas, &words, room, look.crumb_font, look.palette.text, false);
            for knob in [Knob::Save, Knob::Never, Knob::Close] {
                let spot = knob_rect(bar, knob);
                let ground = if hot == Some(knob) { look.palette.bar_hover } else { look.palette.field };
                if knob != Knob::Close || hot == Some(knob) {
                    crate::rounded(canvas, spot, ground, look.palette.border, crate::scale(6, dpi));
                }
                let font = if knob == Knob::Close { look.glyphs } else { look.crumb_font };
                crate::text(canvas, label(knob), spot, font, look.palette.text, true);
            }
        });
        let _ = EndPaint(bar, &paint);
    }
}

fn row_at(y: i32) -> Option<usize> {
    let dpi = crate::dpi();
    let y = y - crate::scale(PAD, dpi);
    let rows = with(|f| f.shown.as_ref().map_or(0, |s| s.offered.accounts.len()));
    (y >= 0).then(|| (y / crate::scale(ROW, dpi)) as usize).filter(|r| *r < rows)
}

fn paint_list(list: HWND) {
    let Some(look) = crate::look() else { return };
    let (rows, hot) = with(|f| {
        let rows: Vec<(String, String)> = f
            .shown
            .as_ref()
            .map(|s| s.offered.accounts.iter().map(|k| (k.user.clone(), k.host.clone())).collect())
            .unwrap_or_default();
        (rows, f.hot)
    });
    let dpi = crate::dpi();
    let s = |v| crate::scale(v, dpi);
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(list, &mut client);
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(list, &mut paint);
        crate::buffered(hdc, client, |canvas, area| {
            crate::fill(canvas, area, look.palette.card);
            for (i, (user, host)) in rows.iter().enumerate() {
                let top = area.top + s(PAD) + i as i32 * s(ROW);
                let row = RECT { left: area.left + s(PAD), top, right: area.right - s(PAD), bottom: top + s(ROW) };
                if hot == Some(i) {
                    crate::rounded(canvas, row, look.palette.bar_hover, look.palette.bar_hover, s(6));
                }
                let inner = RECT { left: row.left + s(10), right: row.right - s(10), ..row };
                let name = if user.is_empty() { if crate::portuguese() { "(sem nome)" } else { "(no name)" } } else { user };
                crate::text(canvas, name, RECT { bottom: top + s(ROW) / 2 + s(4), ..inner }, look.crumb_font, look.palette.text, false);
                crate::text(canvas, host, RECT { top: top + s(ROW) / 2, ..inner }, look.crumb_font, look.palette.dim, false);
            }
        });
        let _ = EndPaint(list, &paint);
    }
}

fn track(window: HWND) {
    let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: window, dwHoverTime: 0 };
    unsafe {
        let _ = TrackMouseEvent(&mut track);
    }
}

extern "system" fn list_proc(list: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            // A click here never takes the keyboard from the box.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_list(list);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let row = row_at(y);
                if with(|f| std::mem::replace(&mut f.hot, row) != row) {
                    let _ = InvalidateRect(Some(list), None, false);
                }
                track(list);
                LRESULT(0)
            }
            crate::WM_MOUSELEAVE => {
                if with(|f| f.hot.take().is_some()) {
                    let _ = InvalidateRect(Some(list), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let row = row_at(y);
                with(|f| f.down = pressed(row, f.shown_at, Instant::now()));
                LRESULT(0)
            }
            // Down and up on the same row, the down late enough.
            WM_LBUTTONUP => {
                let (row, down) = (row_at(y), with(|f| f.down.take()));
                if let Some(row) = row.filter(|_| down == row) {
                    pick(row);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(list, message, wparam, lparam),
        }
    }
}

extern "system" fn bar_proc(bar: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint_bar(bar);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let knob = knob_at(bar, x, y);
                if with(|f| std::mem::replace(&mut f.bar_hot, knob) != knob) {
                    let _ = InvalidateRect(Some(bar), None, false);
                }
                track(bar);
                LRESULT(0)
            }
            crate::WM_MOUSELEAVE => {
                if with(|f| f.bar_hot.take().is_some()) {
                    let _ = InvalidateRect(Some(bar), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let knob = knob_at(bar, x, y);
                with(|f| f.bar_down = pressed(knob, f.bar_at, Instant::now()));
                LRESULT(0)
            }
            // As on the list: down and up on the same button, the down late enough.
            WM_LBUTTONUP => {
                let (knob, down) = (knob_at(bar, x, y), with(|f| f.bar_down.take()));
                if let Some(knob) = knob.filter(|_| down == knob) {
                    answer_bar(knob);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(bar, message, wparam, lparam),
        }
    }
}

/// The list and the bar as a script sees them: hosts and names, never a password.
pub fn bench() -> Value {
    with(|f| {
        let shown = |w: HWND| !w.is_invalid() && unsafe { IsWindowVisible(w).as_bool() };
        let list = f.shown.as_ref().map(|s| {
            json!({
                "tab": s.tab,
                "host": s.offered.host,
                "clear": s.offered.clear,
                "framed": s.frame.is_some(),
                "name": s.name,
                "users": s.offered.accounts.iter().map(|k| k.user.clone()).collect::<Vec<_>>(),
                "hosts": s.offered.accounts.iter().map(|k| k.host.clone()).collect::<Vec<_>>(),
            })
        });
        let bar = f.ask.as_ref().map(|a| json!({ "tab": a.tab, "host": a.sent.host, "user": a.sent.user, "changed": a.changed }));
        json!({
            "list": list,
            "list_shown": shown(f.list),
            "bar": bar,
            "bar_shown": shown(f.bar),
            "held": f.sent.values().map(|s| s.host.clone()).collect::<Vec<_>>(),
            "refused": f.refused,
            "filled": f.filled,
            "saved": f.saved,
        })
    })
}

/// `bench key "forms:pick 0"`, `forms:save`, `forms:never`, `forms:close`:
/// the same as a click on that row or button.
pub fn bench_act(what: &str) -> bool {
    match what.trim() {
        "save" => answer_bar(Knob::Save),
        "never" => answer_bar(Knob::Never),
        "close" => answer_bar(Knob::Close),
        pick_row => match pick_row.strip_prefix("pick ").and_then(|n| n.trim().parse().ok()) {
            Some(row) if with(|f| f.shown.is_some()) => pick(row),
            _ => return false,
        },
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_that_isnt_forms_is_left_for_others() {
        assert!(!message(1, None, "https://a.test/", r#"{"op":"peek.open","url":"https://b.test/"}"#));
        assert!(!message(1, None, "https://a.test/", "not json"));
        // Ours, but from no tab of the row: heard and dropped.
        assert!(message(1, None, "https://a.test/", r#"{"op":"forms.sent","user":"x"}"#));
    }

    #[test]
    fn a_click_needs_a_settled_list() {
        let t = Instant::now();
        assert_eq!(pressed(Some(1), Some(t), t + CLICK_AFTER - Duration::from_millis(1)), None);
        assert_eq!(pressed(Some(1), Some(t), t + CLICK_AFTER), Some(1));
        assert_eq!(pressed(Some(Knob::Save), Some(t), t + CLICK_AFTER), Some(Knob::Save));
        assert_eq!(pressed(None::<usize>, Some(t), t + CLICK_AFTER), None);
        // Never shown, or a clock that went back: no click.
        assert_eq!(pressed(Some(1), None, t), None);
        assert_eq!(pressed(Some(1), Some(t + CLICK_AFTER), t), None);
    }

    #[test]
    fn the_script_never_posts_a_password() {
        // Every message it posts, by op: none carries the password box's value.
        for line in SCRIPT.lines().filter(|l| l.contains("post({")) {
            assert!(!line.contains("pass.value") && !line.contains("password"), "{line}");
        }
    }
}
