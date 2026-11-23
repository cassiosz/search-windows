//! Motion: short eased transitions, one frame per screen refresh.
//!
//! A helper thread waits on DwmFlush, the compositor's own beat, and posts
//! WM_FRAME; each frame is worked out on the window's thread from the time
//! gone by, so a late frame never slows a transition down. Nothing runs
//! while nothing moves: the thread ends with the last track. With Windows'
//! animations off (SPI_GETCLIENTAREAANIMATION), everything lands at once.
//!
//! GDI draws the column fast enough to keep up (one 250 px strip, already
//! double-buffered), and the pop-ups are windows of their own that DWM
//! moves, so no DirectComposition device is made: launch and memory stay
//! as they were.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmFlush, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GdiFlush, InvalidateRect, UpdateWindow};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::BOOL;

/// Posted by the beat thread: time for the next frame.
pub const WM_FRAME: u32 = WM_APP + 10;

// How long each kind of change takes, in ms: routine state changes stay
// under 250, the overlays a little longer; an exit is faster than its entrance.
pub const PILL_MS: u32 = 180;
pub const ROW_IN_MS: u32 = 200;
pub const ROW_OUT_MS: u32 = 160;
pub const GROUND_MS: u32 = 240;
pub const SPLIT_MS: u32 = 240;
pub const DROP_MS: u32 = 150;
pub const SHEET_MS: u32 = 220;

/// What is moving. One track per key: a new start takes over from the old.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// The raised pill of the tab in front, gliding to its new row.
    Pill,
    /// The column's ground, fading to another space's colour.
    Ground,
    /// A new tab's row, opening.
    Row(u64),
    /// The room a closed tab's row left, after this tab (TOP: above the first).
    Gap(u64),
    /// The pages of a split, to their new places.
    Split,
    /// A pop-up window, to its place.
    Window(isize),
    /// A layered window's opacity.
    Alpha(isize),
}

/// Above the first row, for Key::Gap.
pub const TOP: u64 = u64::MAX;

enum Apply {
    /// Drawn by the column's paint, which asks `progress`.
    Column,
    /// Laid out by main's split_frame, which asks `progress`.
    Split,
    Place(HWND, RECT, RECT),
    Alpha(HWND, u8, u8),
}

struct Track {
    key: Key,
    start: Instant,
    ms: f32,
    apply: Apply,
}

thread_local! {
    static TRACKS: RefCell<Vec<Track>> = const { RefCell::new(Vec::new()) };
}

static BEATING: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU32 = AtomicU32::new(0);
/// A frame is posted and not drawn yet: the beat never floods the queue.
static OWED: AtomicBool = AtomicBool::new(false);
static TARGET: AtomicIsize = AtomicIsize::new(0);
/// Frames drawn since launch, for the bench.
static FRAMES: AtomicU64 = AtomicU64::new(0);

/// Windows' "Animation effects" setting, unless SEARCHWIN_MOTION (a test
/// hook, "on" or "off") says otherwise: the reduced path is tried without
/// touching your settings.
pub fn enabled() -> bool {
    match std::env::var("SEARCHWIN_MOTION").as_deref() {
        Ok("off") => false,
        Ok("on") => true,
        _ => {
            let mut on = BOOL(1);
            let asked = unsafe {
                SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
            };
            asked.is_err() || on.as_bool()
        }
    }
}

/// The window frames are posted to; set once, at launch.
pub fn set_target(window: HWND) {
    TARGET.store(window.0 as isize, Ordering::Relaxed);
}

/// Natural deceleration, close to cubic-bezier(0.16, 1, 0.3, 1): a quick
/// start that settles softly, never past its end.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(4)
}

pub fn lerp(a: i32, b: i32, p: f32) -> i32 {
    a + ((b - a) as f32 * p).round() as i32
}

pub fn lerp_rect(a: RECT, b: RECT, p: f32) -> RECT {
    RECT { left: lerp(a.left, b.left, p), top: lerp(a.top, b.top, p), right: lerp(a.right, b.right, p), bottom: lerp(a.bottom, b.bottom, p) }
}

/// Starts `key` from 0; false with animations off, when the caller simply
/// shows the end.
fn start(key: Key, ms: u32, apply: Apply) -> bool {
    if !enabled() {
        return false;
    }
    TRACKS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|x| x.key != key);
        t.push(Track { key, start: Instant::now(), ms: ms as f32, apply });
    });
    beat();
    true
}

/// A change the column draws (or the split lays out) itself, asking `progress`.
pub fn begin(key: Key, ms: u32) -> bool {
    start(key, ms, if key == Key::Split { Apply::Split } else { Apply::Column })
}

/// How far `key` has eased, 0 to 1; None once it has landed (or never moved).
pub fn progress(key: Key) -> Option<f32> {
    TRACKS.with(|t| {
        let t = t.borrow();
        // Asked for every row on every paint: nothing more when nothing moves.
        let x = t.iter().find(|x| x.key == key)?;
        Some(ease(Instant::now().duration_since(x.start).as_secs_f32() * 1000.0 / x.ms))
    })
}

/// Lands `key` where it is; nothing more is drawn for it.
pub fn stop(key: Key) {
    TRACKS.with(|t| t.borrow_mut().retain(|x| x.key != key));
}

/// A pop-up already put at `to`, brought there from `from` (None: it stays,
/// and any glide it was in stops, as when its owner moves it at once).
pub fn place(window: HWND, from: Option<RECT>, to: RECT, ms: u32) {
    let key = Key::Window(window.0 as isize);
    stop(key);
    let Some(from) = from.filter(|f| *f != to) else { return };
    if start(key, ms, Apply::Place(window, from, to)) {
        set_rect(window, from);
    }
}

/// A pop-up shown at `to` (on top of its owner's others with `top`), from
/// `from` when it glides: it appears where it starts and already drawn,
/// never as an empty box for a frame.
pub fn show(window: HWND, top: bool, from: Option<RECT>, to: RECT, ms: u32) {
    let z = if top { SWP_NOACTIVATE } else { SWP_NOACTIVATE | SWP_NOZORDER };
    let after = top.then_some(HWND_TOP);
    unsafe {
        // Cloaked while it shows and paints: the compositor never sees it blank.
        let appearing = !IsWindowVisible(window).as_bool();
        let cloak = |on: bool| {
            let on = BOOL::from(on);
            let _ = DwmSetWindowAttribute(window, DWMWA_CLOAK, &on as *const _ as _, 4);
        };
        if appearing {
            cloak(true);
        }
        let _ = SetWindowPos(window, after, to.left, to.top, to.right - to.left, to.bottom - to.top, z);
        place(window, from, to, ms);
        let _ = SetWindowPos(window, after, 0, 0, 0, 0, z | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = InvalidateRect(Some(window), None, false);
        let _ = UpdateWindow(window);
        if appearing {
            // GDI batches drawing per thread: out to the window before it is seen.
            let _ = GdiFlush();
            cloak(false);
        }
    }
}

/// A layered window at alpha `to`, brought there from `from`.
pub fn fade(window: HWND, from: u8, to: u8, ms: u32) {
    if start(Key::Alpha(window.0 as isize), ms, Apply::Alpha(window, from, to)) {
        set_alpha(window, from);
    }
}

/// Where a pop-up shown at `to` opens from: its top part, so it unfolds downward.
pub fn unfolding(to: RECT) -> RECT {
    RECT { bottom: to.top + (to.bottom - to.top) / 2, ..to }
}

/// Where a sheet shown at `to` rises from: a little lower, the same size.
pub fn rising(to: RECT, by: i32) -> RECT {
    RECT { top: to.top + by, bottom: to.bottom + by, ..to }
}

/// A window's place on screen, as `place` takes it.
pub fn rect_of(window: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(window, &mut r);
    }
    r
}

fn set_rect(window: HWND, r: RECT) {
    unsafe {
        let _ = SetWindowPos(window, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER);
    }
}

fn set_alpha(window: HWND, alpha: u8) {
    unsafe {
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// The beat: a thread that wakes on each composition and asks for a frame.
fn beat() {
    let target = TARGET.load(Ordering::Relaxed);
    if target == 0 || BEATING.swap(true, Ordering::AcqRel) {
        return;
    }
    // Each thread beats only while it is the latest: one stopping as another
    // starts (a key held in the field) never leaves two running.
    let me = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    // A frame posted to a window that never drew it is not owed any more.
    OWED.store(false, Ordering::Release);
    let spawned = std::thread::Builder::new().name("motion".into()).stack_size(64 * 1024).spawn(move || {
        while BEATING.load(Ordering::Acquire) && GENERATION.load(Ordering::Acquire) == me {
            // shortcut: DwmFlush fails only with composition off (never since Windows 8); then a plain 60 Hz wait.
            if unsafe { DwmFlush() }.is_err() {
                std::thread::sleep(Duration::from_millis(16));
            }
            if !OWED.swap(true, Ordering::AcqRel)
                && unsafe { PostMessageW(Some(HWND(target as _)), WM_FRAME, WPARAM(0), LPARAM(0)) }.is_err()
            {
                OWED.store(false, Ordering::Release);
            }
        }
    });
    if spawned.is_err() {
        // No thread, no beat: one frame, posted (a caller may be painting), lands everything.
        BEATING.store(false, Ordering::Release);
        TRACKS.with(|t| t.borrow_mut().iter_mut().for_each(|x| x.ms = 0.0));
        unsafe {
            let _ = PostMessageW(Some(HWND(target as _)), WM_FRAME, WPARAM(0), LPARAM(0));
        }
    }
}

/// WM_FRAME: every track one step on; the finished ones land and go.
pub fn frame() {
    OWED.store(false, Ordering::Release);
    FRAMES.fetch_add(1, Ordering::Relaxed);
    let now = Instant::now();
    let mut column = false;
    let mut split = false;
    let mut places = Vec::new();
    let mut alphas = Vec::new();
    let empty = TRACKS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|x| {
            let raw = if x.ms <= 0.0 { 1.0 } else { now.duration_since(x.start).as_secs_f32() * 1000.0 / x.ms };
            let p = ease(raw);
            match x.apply {
                Apply::Column => column = true,
                Apply::Split => split = true,
                Apply::Place(w, a, b) => places.push((w, lerp_rect(a, b, p))),
                Apply::Alpha(w, a, b) => alphas.push((w, lerp(a as i32, b as i32, p) as u8)),
            }
            // A pop-up closed mid-way: its track goes with it.
            let alive = match x.apply {
                Apply::Place(w, ..) | Apply::Alpha(w, ..) => unsafe { IsWindow(Some(w)).as_bool() },
                _ => true,
            };
            raw < 1.0 && alive
        });
        t.is_empty()
    });
    if empty {
        BEATING.store(false, Ordering::Release);
    }
    // Outside the borrow: these calls can come back into a window procedure.
    for (w, r) in places {
        set_rect(w, r);
    }
    for (w, a) in alphas {
        set_alpha(w, a);
    }
    if split {
        crate::split_frame();
    }
    if column {
        crate::repaint_column();
    }
}

/// For the bench: on, how many tracks are moving, frames drawn so far.
pub fn bench_state() -> serde_json::Value {
    serde_json::json!({
        "on": enabled(),
        "moving": TRACKS.with(|t| t.borrow().len()),
        "frames": FRAMES.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_starts_at_zero_ends_at_one_and_never_overshoots() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(2.0), 1.0);
        let mut last = 0.0;
        for i in 1..=100 {
            let v = ease(i as f32 / 100.0);
            assert!(v >= last && v <= 1.0);
            last = v;
        }
        // Decelerates: most of the way is covered early.
        assert!(ease(0.5) > 0.9);
    }

    #[test]
    fn lerp_lands_exactly() {
        let a = RECT { left: 0, top: 10, right: 100, bottom: 50 };
        let b = RECT { left: 20, top: 0, right: 80, bottom: 90 };
        assert_eq!(lerp_rect(a, b, 0.0), a);
        assert_eq!(lerp_rect(a, b, 1.0), b);
        assert_eq!(unfolding(b).bottom, 45);
        assert_eq!(rising(b, 8).top, 8);
    }
}
