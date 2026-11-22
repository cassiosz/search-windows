//! search://passwords (upstream's Passwords sheet, Ctrl+Alt+L) and the
//! vault's other doors: where accounts are kept, the sites never asked
//! about, and proving it is you before a password leaves the vault.
//!
//! The page lists accounts without their secrets. Showing, copying or
//! exporting one first asks Windows Hello, off the window's thread (it
//! blocks until answered), and each yes is good for that one request: a
//! password shown goes to the one search://passwords page that asked, once,
//! and is never written down or logged. A copied one leaves the clipboard
//! after 30 seconds unless something else was copied since.
//!
//! No Windows Hello (no PIN, face or fingerprint set up): nothing is shown,
//! copied or exported, and the page says how to set Hello up. Asking for the
//! Windows password instead (CredUI + LogonUser) was weighed and left out:
//! it is a second, password-handling path that a test run can never walk,
//! and a PC without Hello is one setting away from having it.
//!
//! Test hooks, so a run never touches what you keep: SEARCHWIN_VAULT_PREFIX
//! keeps accounts under another name in Credential Manager; with
//! SEARCHWIN_BENCH=1, SEARCHWIN_VERIFY (yes, no, none) answers for Hello and
//! SEARCHWIN_BENCH_FILE stands in for the file dialogs.

use std::cell::RefCell;
use std::sync::Mutex;

use searchwin_vault::{CredentialStore, Error, Secret, Store};
use serde_json::{Value, json};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::w;

use crate::page::{self, TabId};

/// Posted to the main window: a verification came back, or a file dialog is due.
/// Both run from the loop, never inside a page's own event.
pub const WM_PASSWORDS: u32 = WM_APP + 8;
const CLEAR_AFTER_MS: u32 = 30_000;

enum Job {
    Reveal { host: String, user: String },
    Copy { host: String, user: String },
    Export,
    Import,
}

/// A request of a page of ours, answered later: the tab, its page, and the question's id.
struct Asked {
    tab: TabId,
    from: String,
    id: Value,
    job: Job,
}

#[derive(Default)]
struct Vault {
    /// Waiting for Hello. One at a time.
    verifying: Option<Asked>,
    /// File dialogs to show from the loop.
    queued: Vec<Asked>,
    /// The clipboard's number right after a password went on it.
    clip: Option<u32>,
    /// How many the page last listed, and what the last request came to (the bench's).
    listed: usize,
    last: &'static str,
}

thread_local! {
    static VAULT: RefCell<Vault> = RefCell::new(Vault::default());
}

/// Hello's answer, from the thread that asked.
static VERIFIED: Mutex<Option<searchwin_vault::Result<bool>>> = Mutex::new(None);

/// Never hold the borrow across a Win32 or WebView2 call.
fn with<R>(f: impl FnOnce(&mut Vault) -> R) -> R {
    VAULT.with(|v| f(&mut v.borrow_mut()))
}

/// Where accounts are kept: Credential Manager under `SearchWin:`, or a
/// test run's own prefix. A prefix that won't do, or none in a bench run, is
/// a test's mistake: the store refuses everything — never what you keep.
pub fn store() -> CredentialStore {
    store_for(std::env::var("SEARCHWIN_VAULT_PREFIX").ok().as_deref(), bench_on())
}

fn store_for(prefix: Option<&str>, bench: bool) -> CredentialStore {
    match prefix {
        Some(prefix) => CredentialStore::with_prefix(prefix)
            .ok()
            .filter(|s| !s.is_real())
            .unwrap_or_else(CredentialStore::refusing),
        None if bench => CredentialStore::refusing(),
        None => CredentialStore::new(),
    }
}

fn bench_on() -> bool {
    std::env::var("SEARCHWIN_BENCH").as_deref() == Ok("1")
}

fn never_file() -> std::path::PathBuf {
    crate::data_folder().join("passwords-never.json")
}

/// Sites never asked about (Vault.swift `never`): hosts or whole sites.
pub fn never() -> Vec<String> {
    std::fs::read(never_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn set_never(list: &[String]) {
    let _ = std::fs::create_dir_all(crate::data_folder());
    if let Err(error) = std::fs::write(never_file(), json!(list).to_string()) {
        crate::crash_log(&format!("never-ask list not saved: {error}"));
    }
}

/// "Never for this site", from the offer to save.
pub fn add_never(site: &str) {
    let mut list = never();
    if !list.iter().any(|s| s == site) {
        list.push(site.to_string());
        set_never(&list);
    }
}

/// What search://passwords (and, for its CSV, search://import) asks.
/// Null is "answered later": a verification or a dialog is under way.
pub fn message(tab: TabId, from: &str, op: &str, asked: &Value) -> Value {
    let text = |key: &str| asked.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    // The import page brings a CSV in, and asks nothing else of the vault.
    if from != "passwords" && op != "passwords.import" {
        return json!({ "error": "not here" });
    }
    let later = |job: Job| Asked { tab, from: from.to_string(), id: asked.get("id").cloned().unwrap_or(Value::Null), job };
    let (host, user) = (text("host"), text("user"));
    match op {
        "passwords.list" => list(),
        "passwords.save" => edit(&host, &user, &text("new_user"), &text("note")),
        "passwords.delete" => match store().delete(&host, &user) {
            Ok(gone) => json!({ "ok": gone }),
            Err(error) => json!({ "error": error.to_string() }),
        },
        "passwords.forget_never" => {
            let site = text("site");
            let mut list = never();
            list.retain(|s| *s != site);
            set_never(&list);
            list_with(None)
        }
        "passwords.reveal" => verify_then(later(Job::Reveal { host, user })),
        "passwords.copy" => verify_then(later(Job::Copy { host, user })),
        "passwords.export" => verify_then(later(Job::Export)),
        "passwords.import" => {
            with(|v| v.queued.push(later(Job::Import)));
            post_self();
            Value::Null
        }
        _ => json!({ "error": "unknown op" }),
    }
}

fn list() -> Value {
    list_with(None)
}

fn list_with(error: Option<String>) -> Value {
    let (accounts, error) = match searchwin_vault::list(&store()) {
        Ok(all) => (all, error),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    with(|v| v.listed = accounts.len());
    let mut value = json!({ "accounts": accounts, "never": never() });
    if let Some(error) = error {
        value["error"] = json!(error);
    }
    value
}

/// A name or a note changed. The password itself is never sent by the page.
fn edit(host: &str, user: &str, new_user: &str, note: &str) -> Value {
    let mut store = store();
    let changed = (|| {
        let Some(mut account) = store.account(host, user)? else { return Ok(false) };
        if new_user != user && store.account(host, new_user)?.is_some() {
            return Err(Error::Invalid("that name is already kept for this site"));
        }
        account.user = new_user.to_string();
        account.note = note.to_string();
        store.save(&account)?;
        if new_user != user {
            store.delete(host, user)?;
        }
        Ok(true)
    })();
    match changed {
        Ok(ok) => json!({ "ok": ok }),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

fn post_self() {
    if let Some(main) = crate::window() {
        unsafe {
            let _ = PostMessageW(Some(main), WM_PASSWORDS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Hello is asked on a thread of its own: it blocks until answered.
fn verify_then(asked: Asked) -> Value {
    if with(|v| v.verifying.is_some()) {
        return json!({ "error": "busy" });
    }
    let reason = match &asked.job {
        Job::Reveal { host, .. } | Job::Copy { host, .. } => {
            if crate::portuguese() { format!("Mostrar a senha de {host}") } else { format!("Show the password for {host}") }
        }
        _ => (if crate::portuguese() { "Exportar as senhas" } else { "Export your passwords" }).to_string(),
    };
    with(|v| v.verifying = Some(asked));
    let owner = crate::window().map_or(0, |w| w.0 as isize);
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let answer = verify(owner, &reason);
        if let Ok(mut slot) = VERIFIED.lock() {
            *slot = Some(answer);
        }
        unsafe {
            let _ = PostMessageW(Some(HWND(owner as _)), WM_PASSWORDS, WPARAM(0), LPARAM(0));
        }
    });
    Value::Null
}

fn verify(owner: isize, reason: &str) -> searchwin_vault::Result<bool> {
    if bench_on() {
        match std::env::var("SEARCHWIN_VERIFY").as_deref() {
            Ok("yes") => return Ok(true),
            Ok("no") => return Ok(false),
            Ok("none") => return Err(Error::NoVerifier(0)),
            _ => {}
        }
    }
    searchwin_vault::verify_user(owner, reason)
}

/// On WM_PASSWORDS: Hello's answer, then any dialog due.
pub fn act() {
    let answer = VERIFIED.lock().ok().and_then(|mut slot| slot.take());
    if let Some(answer) = answer
        && let Some(asked) = with(|v| v.verifying.take())
    {
        match answer {
            Ok(true) => run(asked),
            Ok(false) => refuse(&asked, "refused", if crate::portuguese() { "Não confirmado." } else { "Not confirmed." }),
            Err(Error::NoVerifier(_)) => refuse(&asked, "no hello", no_hello()),
            Err(error) => refuse(&asked, "failed", &error.to_string()),
        }
    }
    for asked in with(|v| std::mem::take(&mut v.queued)) {
        run(asked);
    }
}

fn no_hello() -> &'static str {
    if crate::portuguese() {
        "O Windows Hello não está configurado neste PC, então o Search não tem como confirmar que é você. Configure um PIN, rosto ou digital em Configurações › Contas › Opções de entrada e tente de novo."
    } else {
        "Windows Hello isn't set up on this PC, so Search can't check that it's you. Set up a PIN, face or fingerprint in Settings › Accounts › Sign-in options, then try again."
    }
}

fn refuse(asked: &Asked, why: &'static str, message: &str) {
    with(|v| v.last = why);
    answer(asked, json!({ "error": message, "refused": true }));
}

fn answer(asked: &Asked, mut value: Value) {
    value["id"] = asked.id.clone();
    page::post_to(asked.tab, &asked.from, &value.to_string());
}

/// The request itself, once it may go ahead.
fn run(asked: Asked) {
    let failed = |e: &dyn std::fmt::Display| json!({ "error": e.to_string() });
    match &asked.job {
        Job::Reveal { host, user } => match store().account(host, user) {
            Ok(Some(account)) => {
                // Written straight into strings that are wiped when dropped.
                let quoted = Secret::new(serde_json::to_string(account.password.expose()).unwrap_or_default());
                let text = Secret::new(format!(r#"{{"id":{},"password":{}}}"#, asked.id, quoted.expose()));
                page::post_to(asked.tab, "passwords", text.expose());
                with(|v| v.last = "revealed");
            }
            Ok(None) => answer(&asked, json!({ "error": "gone" })),
            Err(error) => answer(&asked, failed(&error)),
        },
        Job::Copy { host, user } => match store().account(host, user) {
            Ok(Some(account)) => {
                let copied = copy(account.password.expose());
                with(|v| v.last = if copied { "copied" } else { "copy failed" });
                answer(&asked, json!({ "copied": copied }));
            }
            Ok(None) => answer(&asked, json!({ "error": "gone" })),
            Err(error) => answer(&asked, failed(&error)),
        },
        Job::Export => {
            let Some(path) = pick_file(true) else { return answer(&asked, json!({ "cancelled": true })) };
            let written = searchwin_vault::export_csv(&store()).map_err(|e| e.to_string()).and_then(|csv| {
                let rows = csv.expose().lines().count().saturating_sub(1);
                std::fs::write(&path, csv.expose()).map(|_| rows).map_err(|e| e.to_string())
            });
            match written {
                Ok(rows) => {
                    with(|v| v.last = "exported");
                    answer(&asked, json!({ "exported": rows }));
                }
                Err(error) => answer(&asked, json!({ "error": error })),
            }
        }
        Job::Import => {
            let Some(path) = pick_file(false) else { return answer(&asked, json!({ "cancelled": true })) };
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let text = Secret::new(String::from_utf8_lossy(&bytes).into_owned());
                    let mut bytes = bytes;
                    bytes.fill(0);
                    let (kept, skipped) = searchwin_vault::take_csv(&mut store(), text.expose().trim_start_matches('\u{feff}'));
                    with(|v| v.last = "imported");
                    answer(&asked, json!({ "kept": kept, "skipped": skipped }));
                }
                Err(error) => answer(&asked, failed(&error)),
            }
        }
    }
}

/// A CSV to read, or where to write one. A test run names it instead.
fn pick_file(save: bool) -> Option<std::path::PathBuf> {
    if bench_on() {
        return std::env::var_os("SEARCHWIN_BENCH_FILE").map(Into::into);
    }
    let owner = crate::window();
    let filter = [COMDLG_FILTERSPEC { pszName: w!("CSV"), pszSpec: w!("*.csv") }];
    unsafe {
        let dialog: IFileDialog = if save {
            let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            dialog.SetFileName(w!("Search Passwords.csv")).ok()?;
            dialog.into()
        } else {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            dialog.into()
        };
        dialog.SetFileTypes(&filter).ok()?;
        dialog.SetDefaultExtension(w!("csv")).ok()?;
        // Cancelled is an error here, and no file.
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path.map(Into::into)
    }
}

/// On the clipboard, kept out of its history and the cloud's, and taken off
/// again in 30 seconds if it is still what is there.
fn copy(password: &str) -> bool {
    let Some(owner) = crate::window() else { return false };
    let mut wide: Vec<u16> = password.encode_utf16().chain(Some(0)).collect();
    let done = unsafe {
        let put = |format: u32, bytes: &[u8]| -> bool {
            let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
            let at = GlobalLock(memory);
            if at.is_null() {
                let _ = GlobalFree(Some(memory));
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len());
            let _ = GlobalUnlock(memory);
            // The clipboard owns it once taken; only a refusal leaves it ours.
            SetClipboardData(format, Some(HANDLE(memory.0))).is_ok() || {
                let _ = GlobalFree(Some(memory));
                false
            }
        };
        let mut done = false;
        if OpenClipboard(Some(owner)).is_ok() {
            if EmptyClipboard().is_ok() {
                let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
                done = put(13 /* CF_UNICODETEXT */, bytes);
                if done {
                    let zero = 0u32.to_ne_bytes();
                    put(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")), &zero);
                    put(RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), &zero);
                    put(RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")), &zero);
                }
            }
            let _ = CloseClipboard();
        }
        done
    };
    for c in wide.iter_mut() {
        unsafe { std::ptr::write_volatile(c, 0) };
    }
    if done {
        with(|v| v.clip = Some(unsafe { GetClipboardSequenceNumber() }));
        unsafe {
            SetTimer(None, 0, CLEAR_AFTER_MS, Some(clear_clipboard));
        }
    }
    done
}

unsafe extern "system" fn clear_clipboard(_: HWND, _: u32, timer: usize, _: u32) {
    unsafe {
        let _ = KillTimer(None, timer);
        // Only ours: something copied since is left where it is.
        if with(|v| v.clip.take()) == Some(GetClipboardSequenceNumber())
            && let Some(owner) = crate::window()
            && OpenClipboard(Some(owner)).is_ok()
        {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

/// The vault as a script sees it: counts and outcomes, never a secret.
pub fn bench() -> Value {
    let (listed, verifying, last, clip) = with(|v| (v.listed, v.verifying.is_some(), v.last, v.clip.is_some()));
    json!({ "listed": listed, "verifying": verifying, "last": last, "clipboard_held": clip, "never": never() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_run_never_reaches_the_real_vault() {
        // Read from Debug: nothing here asks Credential Manager anything.
        let prefix = |s: CredentialStore| format!("{s:?}");
        let refused = prefix(CredentialStore::refusing());
        assert_eq!(prefix(store_for(None, true)), refused);
        for bad in ["SearchWin:", "searchwin:", "SEARCHWIN:x:", "no-colon", "a*:"] {
            assert_eq!(prefix(store_for(Some(bad), true)), refused, "{bad}");
            assert_eq!(prefix(store_for(Some(bad), false)), refused, "{bad}");
        }
        assert!(prefix(store_for(Some("SearchWin-test-20:"), true)).contains("SearchWin-test-20:"));
        assert!(store_for(None, false).is_real());
    }
}
