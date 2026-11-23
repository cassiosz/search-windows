//! Sync with your other PCs through a folder you choose (docs/sync.md):
//! settings, spaces and bookmarks, written as small files into a folder that
//! OneDrive, Dropbox or a shared disk carries between PCs. crates/sync is the
//! merge and the files; this is when a pass runs and applying what it brings.
//!
//! Off until turned on in Settings: with no sync.json nothing here runs,
//! writes, or sets a timer. Nothing leaves the PC by the app's doing: it
//! only reads and writes a folder on this PC.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use searchwin_bookmarks::Bookmarks;
use searchwin_spaces::Space;
use searchwin_history::now_ms;
use searchwin_sync::{INSIDE, Items, PinDef};
use serde_json::{Value, json};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows_core::HSTRING;

use crate::{data_folder, state};

/// Every half minute while on, for what other PCs wrote.
pub const EVERY_TIMER: usize = 0x5C;
/// A moment after something synced changes here; changes meanwhile join it.
pub const SOON_TIMER: usize = 0x5D;
const EVERY_MS: u32 = 30_000;
const SOON_MS: u32 = 2_000;

#[derive(Clone)]
struct On {
    folder: PathBuf,
    /// This PC's id in the folder, made when sync is turned on.
    device: String,
}

#[derive(Default)]
struct Status {
    problem: Option<&'static str>,
    /// Unix seconds of the last pass that ran.
    last: Option<u64>,
    others: Vec<String>,
}

thread_local! {
    static ON: RefCell<Option<On>> = const { RefCell::new(None) };
    static STATUS: RefCell<Status> = RefCell::new(Status::default());
    static RUNNING: Cell<bool> = const { Cell::new(false) };
}

fn config() -> PathBuf {
    data_folder().join("sync.json")
}

fn bench_on() -> bool {
    std::env::var("SEARCHWIN_BENCH").as_deref() == Ok("1")
}

/// At launch: carries on if it was on. Off, this is one file not found.
pub fn start() {
    let read = std::fs::read(config()).ok().and_then(|d| serde_json::from_slice::<Value>(&d).ok());
    let on = read.and_then(|v| {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(On { folder: text("folder")?.into(), device: text("device")? })
    });
    if let Some(on) = on.filter(|on| searchwin_sync::is_id(&on.device)) {
        ON.with(|o| *o.borrow_mut() = Some(on));
        begin();
    }
}

fn begin() {
    if let Some(window) = crate::window() {
        unsafe {
            SetTimer(Some(window), EVERY_TIMER, EVERY_MS, None);
        }
    }
    poke();
}

fn on() -> Option<On> {
    ON.with(|o| o.borrow().clone())
}

/// Something synced changed here: a pass soon. Nothing while off, nor for
/// the changes a pass itself makes.
pub fn poke() {
    if on().is_none() || RUNNING.with(Cell::get) {
        return;
    }
    if let Some(window) = crate::window() {
        unsafe {
            SetTimer(Some(window), SOON_TIMER, SOON_MS, None);
        }
    }
}

/// The app came forward: what other PCs wrote meanwhile, unless a pass ran a
/// moment ago (switching apps back and forth is no reason to read again).
pub fn came_back() {
    let recent = STATUS.with(|s| s.borrow().last.is_some_and(|last| now() as u64 <= last + 10));
    if !recent {
        poke();
    }
}

/// WM_TIMER for either timer.
pub fn tick(timer: usize) {
    if timer == SOON_TIMER
        && let Some(window) = crate::window()
    {
        unsafe {
            let _ = KillTimer(Some(window), SOON_TIMER);
        }
    }
    pass();
}

/// Turned on with a folder: always as a PC joining, whatever it agreed with
/// this folder or another before.
fn turn_on(folder: PathBuf) -> Result<(), String> {
    turn_off();
    let on = On { folder, device: searchwin_spaces::random_uuid() };
    let data = json!({ "folder": on.folder.to_string_lossy(), "device": on.device }).to_string();
    std::fs::create_dir_all(data_folder()).and_then(|()| std::fs::write(config(), data)).map_err(|e| e.to_string())?;
    ON.with(|o| *o.borrow_mut() = Some(on));
    begin();
    Ok(())
}

/// Off: this PC's files leave the folder and what it agreed is forgotten.
/// What is in the browser stays as it is.
fn turn_off() {
    if let Some(on) = ON.with(|o| o.borrow_mut().take()) {
        searchwin_sync::leave(&on.folder.join(INSIDE), &on.device);
    }
    searchwin_sync::forget(&data_folder());
    let _ = std::fs::remove_file(config());
    if let Some(window) = crate::window() {
        unsafe {
            let _ = KillTimer(Some(window), EVERY_TIMER);
            let _ = KillTimer(Some(window), SOON_TIMER);
        }
    }
    STATUS.with(|s| *s.borrow_mut() = Status::default());
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// One pass over every scope. Never twice at once: applying what it brings
/// runs code that would ask for another.
pub fn pass() {
    let Some(on) = on() else { return };
    if RUNNING.with(|r| r.replace(true)) {
        return;
    }
    let result = run_all(&on);
    RUNNING.with(|r| r.set(false));
    STATUS.with(|s| {
        let mut s = s.borrow_mut();
        match result {
            Ok(others) => {
                *s = Status { problem: None, last: Some(now() as u64), others: others.into_iter().collect() };
            }
            Err(problem) => s.problem = Some(problem),
        }
    });
}

/// The items of one kind, to tell whether a pass changed that kind.
fn part<'a>(items: &'a Items, kinds: &[&str]) -> Vec<(&'a String, &'a String)> {
    items.iter().filter(|(k, _)| kinds.iter().any(|p| k.starts_with(p))).collect()
}

fn run_all(on: &On) -> Result<BTreeSet<String>, &'static str> {
    // A folder not there (a disk unplugged, OneDrive not running) is no
    // reason to think everything was deleted: nothing happens.
    if !on.folder.is_dir() {
        return Err("missing");
    }
    // What is on screen, written first: the pass reads the files.
    crate::save();
    let root = on.folder.join(INSIDE);
    let data = data_folder();
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    let now = now();
    let state_file = |scope: &str| data.join(format!("sync-{scope}.json"));
    let mut others = BTreeSet::new();

    let here = crate::internal::prefs(searchwin_sync::settings);
    let out = searchwin_sync::run(&root, "app", &state_file("app"), &on.device, &name, now, |_| here).map_err(|_| "unwritable")?;
    let mut applied = true;
    for (key, value) in searchwin_sync::settings_to_apply(&out.live, &out.here) {
        applied &= crate::internal::set_pref(&key, value).is_ok();
    }
    if applied {
        out.agree().map_err(|_| "unwritable")?;
    }
    others.extend(out.others);

    let profiles: Vec<(String, String)> = state(|s| {
        s.spaces.profiles().iter().map(|p| (p.id.clone(), s.spaces.home_of(&p.id).to_string())).collect()
    })
    .unwrap_or_default();
    for (profile, home) in profiles {
        let scope = format!("profile-{profile}");
        let file = searchwin_spaces::profile_folder(&data, &profile).join("bookmarks.json");
        let marks_here = Bookmarks::load(&file);
        let local: Vec<Space> = state(|s| s.spaces.of_profile(&profile).cloned().collect()).unwrap_or_default();
        let local: Vec<&Space> = local.iter().collect();
        // A row that couldn't be read would look like pins deleted: no pass then.
        let pins_here = pins_of(&local.iter().map(|s| s.id.clone()).collect::<Vec<_>>()).ok_or("unreadable")?;
        let out = searchwin_sync::run(&root, &scope, &state_file(&scope), &on.device, &name, now, |base| {
            let mut items = searchwin_sync::spaces(&local, base);
            items.extend(searchwin_sync::bookmarks(marks_here.roots()));
            for (space, defs) in &pins_here {
                items.extend(searchwin_sync::pins(space, defs, base));
            }
            items
        })
        .map_err(|_| "unwritable")?;
        others.extend(out.others.iter().cloned());

        // A whole kind gone at once, by another PC's file, is not applied: it
        // is far likelier a damaged or hostile file than a choice, and the
        // next pass stamps what is here again, so it comes back everywhere.
        let wiped = |kind: &str| {
            let count = |items: &Items| items.keys().filter(|k| k.starts_with(kind)).count();
            count(&out.here) >= searchwin_sync::WHOLE_KIND_FLOOR && count(&out.live) == 0
        };
        let spaces = ["space:", "order:spaces"];
        if part(&out.live, &spaces) != part(&out.here, &spaces) && !wiped("space:") {
            let list = searchwin_sync::apply_spaces(&out.live, &local, &profile, &home);
            state(|s| {
                let removals = s.spaces.adopt(&profile, list);
                s.removals.extend(removals);
            });
            crate::spaces::changed();
        }
        let marks = ["bm:", "kids:"];
        if part(&out.live, &marks) != part(&out.here, &marks) && !wiped("bm:") {
            let tree = Bookmarks::from_roots(searchwin_sync::apply_bookmarks(&out.live, marks_here.roots()));
            // The one before, beside it: what another PC changed can be put back by hand.
            let _ = std::fs::copy(&file, file.with_extension("json.before-sync"));
            tree.save(&file).map_err(|_| "unwritable")?;
            if Some(profile.as_str()) == state(|s| s.spaces.current_profile().to_string()).as_deref() {
                crate::bookmarks::reload();
            }
        }
        // Pins, space by space, after the spaces: one that just arrived has its row.
        {
            let spaces: Vec<String> = state(|s| s.spaces.of_profile(&profile).map(|s| s.id.clone()).collect()).unwrap_or_default();
            let mut moved = false;
            for space in spaces {
                let kinds = [format!("pin:{space}:"), format!("order:pins:{space}")];
                let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
                // Titles alone never move a pin: a new one brings its own with it.
                // A space's pins all gone at once is not applied, as above.
                if part(&out.live, &kinds) == part(&out.here, &kinds) || wiped(kinds[0]) || !readable(&space) {
                    continue;
                }
                let here = pins_here.iter().find(|(s, _)| *s == space).map(|(_, d)| d.as_slice()).unwrap_or_default();
                moved |= adopt_pins(&space, &searchwin_sync::apply_pins(&out.live, &space, here));
            }
            if moved {
                crate::repaint_column();
                crate::save_soon();
            }
        }
        // Only now: a failed apply above returned before it.
        out.agree().map_err(|_| "unwritable")?;
    }
    Ok(others)
}

/// Each space's pins, from its row: the one on screen, or one kept aside
/// (read from its file the first time, as going to it would). A pin without
/// a lasting id gets one here, and its row is written.
/// None when a kept row's file is there but can't be read (another program
/// holding it): read as empty, it would be written over as empty.
fn pins_of(spaces: &[String]) -> Option<Vec<(String, Vec<PinDef>)>> {
    if !spaces.iter().all(|s| readable(s)) {
        return None;
    }
    let base = data_folder();
    let now = now_ms();
    let mut fresh = searchwin_spaces::random_uuid;
    state(|s| {
        let mut out = Vec::new();
        for space in spaces {
            let defs = if *space == s.row_of {
                let private = &s.private;
                let (defs, given) = s.tabs.pin_defs(&|id| private.contains_key(&id), &mut fresh);
                s.session_owed |= given;
                defs
            } else {
                let (defs, given) = s.rows.row(&base, space, now).pin_defs(&|_| false, &mut fresh);
                if given {
                    let _ = s.rows.save(&base, space);
                }
                defs
            };
            out.push((space.clone(), defs));
        }
        out
    })
}

/// Whether a space's row is in hand: on screen, kept in memory, with no file
/// yet, or with a file that reads now.
fn readable(space: &str) -> bool {
    let held = state(|s| s.row_of == space || s.rows.get(space).is_some()).unwrap_or(false);
    let file = searchwin_spaces::session_path(&data_folder(), space);
    held || !file.exists() || std::fs::read(&file).is_ok()
}

/// A space's pins made what the merge says, on screen or in its kept row,
/// which is written. Private tabs are never touched.
fn adopt_pins(space: &str, defs: &[PinDef]) -> bool {
    let base = data_folder();
    let now = now_ms();
    state(|s| {
        if s.row_of == space {
            let private = &s.private;
            let moved = s.tabs.adopt_pins(defs, &|id| private.contains_key(&id));
            s.session_owed |= moved;
            moved
        } else {
            let moved = s.rows.row(&base, space, now).adopt_pins(defs, &|_| false);
            if moved {
                let _ = s.rows.save(&base, space);
            }
            moved
        }
    })
    .unwrap_or(false)
}

/// The folder asked for, starting in OneDrive when there is one. A test run
/// names it instead (SEARCHWIN_SYNC, only with SEARCHWIN_BENCH=1).
fn pick_folder() -> Option<PathBuf> {
    if bench_on() {
        return std::env::var_os("SEARCHWIN_SYNC").filter(|f| !f.is_empty()).map(Into::into);
    }
    let owner = crate::window();
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        if let Some(drive) = std::env::var_os("OneDrive").filter(|d| Path::new(d).is_dir())
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(drive.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // Cancelled is an error here, and no folder.
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        path.map(Into::into)
    }
}

fn snapshot() -> Value {
    let on = on();
    STATUS.with(|s| {
        let s = s.borrow();
        json!({
            "on": on.is_some(),
            "folder": on.map(|o| o.folder.to_string_lossy().into_owned()),
            "problem": s.problem,
            "last": s.last,
            "others": s.others,
        })
    })
}

/// What search://settings asks: "sync.get", "sync.on" (asks for a folder),
/// "sync.off", "sync.now". Every answer is the whole picture.
pub fn message(op: &str) -> Value {
    let error = match op {
        "sync.on" => match pick_folder() {
            Some(folder) => turn_on(folder).err(),
            None => None,
        },
        "sync.off" => {
            turn_off();
            None
        }
        "sync.now" => {
            pass();
            None
        }
        _ => None,
    };
    let mut answer = snapshot();
    if let Some(error) = error {
        answer["error"] = json!(error);
    }
    answer
}

pub fn bench() -> Value {
    let mut value = snapshot();
    value["device"] = json!(on().map(|o| o.device));
    value
}
