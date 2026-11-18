//! Downloads, in the window: WebView2's downloads go straight into your
//! Downloads folder under a free name, with no dialog of its own, and
//! search://downloads (Ctrl+J) is where they are watched, paused, opened.
//! The list and its file are crates/downloads.
//!
//! The page never names a file: it names a row, and the path is the one this
//! side chose when the download started. Nothing is read until a download
//! starts or the page opens.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

use searchwin_downloads::{Downloads, Intent, State};
use serde_json::{Value, json};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{BytesReceivedChangedEventHandler, StateChangedEventHandler};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SW_SHOWNORMAL, SetTimer};
use windows_core::{BOOL, HSTRING, PWSTR, Result, w};

use crate::page::TabId;

/// Progress is pushed to an open downloads page at most this often: about four times a second.
pub const PUSH_TIMER: usize = 3;
const PUSH_EVERY_MS: u32 = 250;

thread_local! {
    static MODEL: RefCell<Option<Downloads>> = const { RefCell::new(None) };
    /// What is under way, by row: what pause, resume and cancel act on.
    static OPERATIONS: RefCell<HashMap<u64, ICoreWebView2DownloadOperation>> = RefCell::new(HashMap::new());
    static OWED: Cell<bool> = const { Cell::new(false) };
    /// A push is already due: changes until then go with it.
    static PUSHING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> PathBuf {
    crate::data_folder().join("downloads.json")
}

fn with<R>(f: impl FnOnce(&mut Downloads) -> R) -> R {
    MODEL.with(|cell| f(cell.borrow_mut().get_or_insert_with(|| Downloads::load(&file()))))
}

/// Where downloads go: your Downloads folder, wherever Windows has it moved.
/// SEARCHWIN_DOWNLOADS is a test hook: a run under test puts its files there instead.
fn folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("SEARCHWIN_DOWNLOADS").filter(|f| !f.is_empty()) {
        return PathBuf::from(folder);
    }
    unsafe {
        if let Ok(path) = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None) {
            let folder = path.to_string().unwrap_or_default();
            CoTaskMemFree(Some(path.0 as _));
            if !folder.is_empty() {
                return PathBuf::from(folder);
            }
        }
    }
    crate::data_folder().join("Downloads")
}

fn take(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let owned = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    owned
}

/// DownloadStarting: into the Downloads folder, under a name nothing there
/// (or nothing still coming in) has, and without WebView2's own download
/// bubble: our page is the place for them. A private tab's is shown while it
/// lasts and never written to downloads.json.
pub fn starting(tab: TabId, args: &ICoreWebView2DownloadStartingEventArgs) -> Result<()> {
    unsafe {
        let operation = args.DownloadOperation()?;
        let mut suggested = PWSTR::null();
        args.ResultFilePath(&mut suggested)?;
        let suggested = take(suggested);
        let name = std::path::Path::new(&suggested).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut uri = PWSTR::null();
        operation.Uri(&mut uri)?;
        let uri = take(uri);
        let mut total = 0i64;
        operation.TotalBytesToReceive(&mut total)?;

        let folder = folder();
        let _ = std::fs::create_dir_all(&folder);
        let coming: Vec<String> = with(|d| d.entries().iter().filter(|e| e.state.active()).map(|e| e.path.clone()).collect());
        let free = searchwin_downloads::free_name(&name, |n| {
            let path = folder.join(n);
            path.exists() || coming.iter().any(|c| std::path::Path::new(c) == path)
        });
        let path = folder.join(free);
        args.SetResultFilePath(&HSTRING::from(path.as_os_str()))?;
        args.SetHandled(true)?;

        let listed = !crate::is_private(tab);
        let path_text = path.to_string_lossy().into_owned();
        let id = with(|d| d.begin(&uri, &path_text, total.max(0) as u64, listed, searchwin_history::now_ms()));
        OPERATIONS.with(|o| o.borrow_mut().insert(id, operation.clone()));

        let mut token = 0i64;
        operation.add_BytesReceivedChanged(
            &BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(operation) = sender {
                    let (mut received, mut total) = (0i64, 0i64);
                    operation.BytesReceived(&mut received)?;
                    operation.TotalBytesToReceive(&mut total)?;
                    if with(|d| d.progress(id, received.max(0) as u64, total.max(0) as u64)) {
                        push_soon();
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        operation.add_StateChanged(
            &StateChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(operation) = sender {
                    state_changed(id, &operation)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    push_soon();
    Ok(())
}

/// Paused, on again, done, cancelled or failed, as the operation says.
fn state_changed(id: u64, operation: &ICoreWebView2DownloadOperation) -> Result<()> {
    let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
    let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
    unsafe {
        operation.State(&mut state)?;
        operation.InterruptReason(&mut reason)?;
    }
    let mut ended = true;
    if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
        let mut path = PWSTR::null();
        unsafe { operation.ResultFilePath(&mut path)? };
        let path = take(path);
        let (mut received, mut total) = (0i64, 0i64);
        unsafe {
            operation.BytesReceived(&mut received)?;
            operation.TotalBytesToReceive(&mut total)?;
        }
        with(|d| {
            if !path.is_empty() {
                d.set_path(id, &path);
            }
            d.progress(id, received.max(0) as u64, total.max(0) as u64);
            d.finish(id);
        });
        owe();
    } else if state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
        with(|d| d.set_paused(id, false));
        ended = false;
    } else if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_PAUSED {
        with(|d| d.set_paused(id, true));
        ended = false;
    } else if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED {
        with(|d| d.cancel(id));
    } else {
        with(|d| d.fail(id, &format!("interrupted ({})", reason.0)));
    }
    if ended {
        OPERATIONS.with(|o| o.borrow_mut().remove(&id));
    }
    push_soon();
    Ok(())
}

fn owe() {
    OWED.with(|o| o.set(true));
    crate::save_soon();
}

/// Called from the window's save: the finished ones, if anything changed.
pub fn save() {
    if !OWED.with(|o| o.replace(false)) {
        return;
    }
    let written = MODEL.with(|cell| cell.borrow().as_ref().map(|d| d.save(&file())));
    if let Some(Err(error)) = written {
        crate::crash_log(&format!("downloads not saved: {error}"));
    }
}

/// Changes go to an open downloads page together, a quarter of a second
/// after the first: a fast download fires its progress far more often than
/// anyone can read a bar.
fn push_soon() {
    if PUSHING.with(|p| p.replace(true)) {
        return;
    }
    if let Some(window) = crate::window() {
        unsafe { SetTimer(Some(window), PUSH_TIMER, PUSH_EVERY_MS, None) };
    }
}

/// On PUSH_TIMER: every open downloads page gets the list as it is now.
pub fn push() {
    PUSHING.with(|p| p.set(false));
    if let Some(window) = crate::window() {
        unsafe {
            let _ = KillTimer(Some(window), PUSH_TIMER);
        }
    }
    let mut message = list();
    message["event"] = json!("downloads");
    crate::page::post_to_internal("downloads", &message.to_string());
}

/// How many are under way, for `bench state`.
pub fn active() -> usize {
    MODEL.with(|cell| cell.borrow().as_ref().map_or(0, |d| d.active()))
}

fn list() -> Value {
    let entries: Vec<Value> = with(|d| {
        d.entries()
            .iter()
            .map(|e| {
                json!({
                    "id": e.id, "name": e.name, "from": e.from, "url": e.url, "total": e.total,
                    "received": e.received, "state": e.state, "error": e.error,
                })
            })
            .collect()
    });
    json!({ "entries": entries })
}

/// What search://downloads asks, already known to come from one of our pages.
pub fn message(op: &str, asked: &Value) -> Value {
    // "id" is the question's own number (common.js ask), so a row is "row".
    let row = asked.get("row").and_then(Value::as_u64).unwrap_or(0);
    match op {
        "downloads.list" => list(),
        "downloads.pause" => act(row, Intent::Pause),
        "downloads.resume" => act(row, Intent::Resume),
        "downloads.cancel" => act(row, Intent::Cancel),
        "downloads.open" => json!({ "ok": open(row, false) }),
        "downloads.show" => json!({ "ok": open(row, true) }),
        "downloads.clear" => {
            let gone = with(|d| d.clear_finished());
            if gone > 0 {
                owe();
            }
            json!({ "cleared": gone })
        }
        _ => json!({ "error": "unknown op" }),
    }
}

/// A button on the page, if the row allows it now; the operation's own
/// state event then tells the list what happened.
fn act(row: u64, intent: Intent) -> Value {
    let operation = with(|d| d.allows(row, intent)).then(|| OPERATIONS.with(|o| o.borrow().get(&row).cloned())).flatten();
    let Some(operation) = operation else { return json!({ "ok": false }) };
    let done = unsafe {
        match intent {
            Intent::Pause => operation.Pause(),
            Intent::Resume => {
                let mut can = BOOL::default();
                operation.CanResume(&mut can).and_then(|_| if can.as_bool() { operation.Resume() } else { Ok(()) })
            }
            Intent::Cancel => operation.Cancel(),
        }
    };
    json!({ "ok": done.is_ok() })
}

/// The file opened with whatever Windows opens it with, or shown selected in
/// its folder. Only a finished one, and only at the path chosen when it started.
fn open(row: u64, in_folder: bool) -> bool {
    let Some((path, state)) = with(|d| d.get(row).map(|e| (e.path.clone(), e.state))) else { return false };
    if (state != State::Done && !in_folder) || !std::path::Path::new(&path).exists() {
        return false;
    }
    let result = unsafe {
        if in_folder {
            // A Windows path can't hold a quote, so this stays one argument.
            let select = HSTRING::from(format!("/select,\"{path}\""));
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), &select, None, SW_SHOWNORMAL)
        } else {
            ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_str()), None, None, SW_SHOWNORMAL)
        }
    };
    // ShellExecute says it worked with a number over 32.
    result.0 as isize > 32
}
