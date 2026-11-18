//! The app's side of `bench`: what each scripted request does in this window.
//! Only alive with SEARCHWIN_BENCH=1; otherwise `start` creates nothing.
//! Everything here goes through the same functions the keyboard and the field
//! use, so a test exercises the real paths, never synthesized OS input.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use searchwin_bench::{Reply, Request, Response, Server};
use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_APP};

/// Posted by the pipe thread when requests are waiting.
pub const WM_BENCH: u32 = WM_APP + 1;
/// Ticks while a `wait` is pending, to give up on time.
pub const WAIT_TIMER: usize = 0xBE;

struct Waiting {
    reply: Reply,
    deadline: Instant,
}

#[derive(Default)]
struct Drive {
    server: Option<Server>,
    waiting: Vec<Waiting>,
}

thread_local! {
    static DRIVE: RefCell<Drive> = RefCell::new(Drive::default());
}

pub fn start(window: HWND) {
    let raw = window.0 as isize; // HWND isn't Send; the number is, and PostMessage is thread-safe
    let server = searchwin_bench::start(move || unsafe {
        let _ = PostMessageW(Some(HWND(raw as _)), WM_BENCH, WPARAM(0), LPARAM(0));
    });
    DRIVE.with(|d| d.borrow_mut().server = server);
}

/// On WM_BENCH: answer what came in. The server is taken out while it drains,
/// because answering runs window code that may come back here.
pub fn drain(window: HWND) {
    let Some(server) = DRIVE.with(|d| d.borrow_mut().server.take()) else { return };
    server.drain(|request, reply| handle(window, request, reply));
    DRIVE.with(|d| d.borrow_mut().server = Some(server));
}

fn handle(window: HWND, request: Request, reply: Reply) {
    match request {
        Request::State => reply.complete(Response::ok(crate::bench_state())),
        Request::Open { url } => {
            crate::go(&url);
            reply.complete(Response::ok(Value::Null));
        }
        Request::Type { text } => {
            crate::type_and_go(&text);
            reply.complete(Response::ok(Value::Null));
        }
        Request::Key { keys } => match parse_keys(&keys) {
            Some((key, ctrl, alt, shift)) => {
                let handled = crate::shortcut_with(key, ctrl, alt, shift) || crate::field_key(key, ctrl, alt, shift);
                reply.complete(Response::ok(json!({ "handled": handled })));
            }
            None => reply.complete(Response::err(format!("unknown keys: {keys}"))),
        },
        Request::Eval { js } => crate::page::eval(&js, move |result| {
            reply.complete(match result {
                Ok(text) => Response::ok(serde_json::from_str(&text).unwrap_or(Value::String(text))),
                Err(error) => Response::err(error),
            })
        }),
        Request::Wait { until, timeout_ms } => {
            if until != "loaded" {
                reply.complete(Response::err(format!("can only wait until loaded, not {until}")));
            } else if !crate::loading() {
                reply.complete(Response::ok(Value::Null));
            } else {
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                DRIVE.with(|d| d.borrow_mut().waiting.push(Waiting { reply, deadline }));
                unsafe { SetTimer(Some(window), WAIT_TIMER, 100, None) };
            }
        }
        Request::Quit => {
            reply.complete(Response::ok(Value::Null));
            crate::quit();
        }
    }
}

/// A page finished loading: everyone waiting for that is answered.
pub fn loaded(window: HWND) {
    let waiting = DRIVE.with(|d| std::mem::take(&mut d.borrow_mut().waiting));
    for wait in waiting {
        wait.reply.complete(Response::ok(Value::Null));
    }
    unsafe {
        let _ = KillTimer(Some(window), WAIT_TIMER);
    }
}

/// On the wait timer: those past their deadline are told so.
pub fn tick(window: HWND) {
    let now = Instant::now();
    let (late, empty) = DRIVE.with(|d| {
        let mut drive = d.borrow_mut();
        let (late, keep): (Vec<_>, Vec<_>) = drive.waiting.drain(..).partition(|w| w.deadline <= now);
        drive.waiting = keep;
        (late, drive.waiting.is_empty())
    });
    for wait in late {
        wait.reply.complete(Response::err("timed out waiting for the page to load"));
    }
    if empty {
        unsafe {
            let _ = KillTimer(Some(window), WAIT_TIMER);
        }
    }
}

/// "ctrl+shift+t", "alt+left", "f5" → the key and its modifiers.
fn parse_keys(keys: &str) -> Option<(u32, bool, bool, bool)> {
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    let mut key = None;
    for part in keys.to_ascii_lowercase().split('+') {
        match part.trim() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            name => key = Some(virtual_key(name)?),
        }
    }
    Some((key?, ctrl, alt, shift))
}

fn virtual_key(name: &str) -> Option<u32> {
    let named = match name {
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "tab" => VK_TAB,
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "f5" => VK_F5,
        "backspace" => VK_BACK,
        "space" => VK_SPACE,
        "." | "period" => VK_OEM_PERIOD,
        "comma" => VK_OEM_COMMA,
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            return (chars.next().is_none() && c.is_ascii_alphanumeric()).then(|| c.to_ascii_uppercase() as u32);
        }
    };
    Some(named.0 as u32)
}
