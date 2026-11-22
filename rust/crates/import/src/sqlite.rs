//! The little of SQLite an import needs, from the copy Windows itself ships
//! (System32\winsqlite3.dll), loaded the first time a database is read: the
//! app imports nothing of it at launch and carries no SQLite of its own.
//!
//! Another browser's database is never opened where it lies. The file (and
//! its -wal, which holds what the running browser wrote last) is copied to a
//! folder of our own, read from the copy, and the copy is deleted.

use std::ffi::{CString, c_char, c_int, c_void};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

type Handle = *mut c_void;

const OK: c_int = 0;
const ROW: c_int = 100;
const DONE: c_int = 101;
const OPEN_READONLY: c_int = 0x01;
#[cfg(test)]
const OPEN_READWRITE: c_int = 0x02;
#[cfg(test)]
const OPEN_CREATE: c_int = 0x04;
const OPEN_URI: c_int = 0x40;
/// LoadLibraryExW: look in System32 only, never beside the exe.
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The struct of function pointers and its loader, from one list.
macro_rules! api {
    ($($(#[$meta:meta])* $field:ident = $symbol:literal: fn($($arg:ty),*) -> $ret:ty;)*) => {
        struct Api {
            $($(#[$meta])* $field: unsafe extern "system" fn($($arg),*) -> $ret,)*
        }

        fn resolve(module: *mut c_void) -> Option<Api> {
            Some(Api {
                $($(#[$meta])* $field: {
                    // SAFETY: a nul-terminated name looked up in a loaded module.
                    let f = unsafe { GetProcAddress(module, concat!($symbol, "\0").as_ptr().cast()) };
                    if f.is_null() {
                        return None;
                    }
                    // SAFETY: the symbol is SQLite's own C API under that
                    // name, and this is that function's signature.
                    unsafe { std::mem::transmute::<*mut c_void, unsafe extern "system" fn($($arg),*) -> $ret>(f) }
                },)*
            })
        }
    };
}

api! {
    open_v2 = "sqlite3_open_v2": fn(*const c_char, *mut Handle, c_int, *const c_char) -> c_int;
    close = "sqlite3_close": fn(Handle) -> c_int;
    prepare_v2 = "sqlite3_prepare_v2": fn(Handle, *const c_char, c_int, *mut Handle, *mut *const c_char) -> c_int;
    step = "sqlite3_step": fn(Handle) -> c_int;
    finalize = "sqlite3_finalize": fn(Handle) -> c_int;
    column_text = "sqlite3_column_text": fn(Handle, c_int) -> *const u8;
    column_bytes = "sqlite3_column_bytes": fn(Handle, c_int) -> c_int;
    column_int64 = "sqlite3_column_int64": fn(Handle, c_int) -> i64;
    #[cfg(test)]
    exec = "sqlite3_exec": fn(Handle, *const c_char, *const c_void, *mut c_void, *mut *mut c_char) -> c_int;
}

// The pointers are to code in a DLL that stays loaded for the process.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load).as_ref()
}

fn load() -> Option<Api> {
    let name: Vec<u16> = "winsqlite3.dll\0".encode_utf16().collect();
    // SAFETY: a nul-terminated wide string; the module is never freed.
    let module = unsafe { LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return None;
    }
    resolve(module)
}

#[derive(Debug)]
pub enum Trouble {
    /// winsqlite3.dll isn't there (a Windows stripped of it).
    NoSqlite,
    /// The file couldn't be copied: missing, or held by another program.
    Copy,
    /// SQLite said no: not a database, or not one this reads.
    Sql,
}

/// An open database: a copy of the browser's, read-only unless made by a
/// test. Closing deletes the copy.
pub struct Db {
    api: &'static Api,
    handle: Handle,
    _copy: Option<Scratch>,
}

pub struct Row<'a> {
    api: &'static Api,
    stmt: Handle,
    _db: std::marker::PhantomData<&'a Db>,
}

impl Row<'_> {
    pub fn text(&self, col: c_int) -> Option<String> {
        // SAFETY: a live statement on a row; the pointer is valid for the
        // given byte count until the next step.
        unsafe {
            let ptr = (self.api.column_text)(self.stmt, col);
            if ptr.is_null() {
                return None;
            }
            let len = (self.api.column_bytes)(self.stmt, col).max(0) as usize;
            Some(String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned())
        }
    }

    pub fn int(&self, col: c_int) -> i64 {
        // SAFETY: a live statement on a row.
        unsafe { (self.api.column_int64)(self.stmt, col) }
    }
}

impl Db {
    /// A copy of `file` (and its -wal), opened read-only. With no -wal the
    /// copy is opened immutable, so SQLite takes no lock and writes nothing
    /// beside it; with one, read-only so the -wal's pages are seen.
    pub fn snapshot(file: &Path) -> Result<Db, Trouble> {
        let api = api().ok_or(Trouble::NoSqlite)?;
        let copy = Scratch::new().map_err(|_| Trouble::Copy)?;
        let name = file.file_name().unwrap_or_default();
        let target = copy.0.join(name);
        copy_shared(file, &target).map_err(|_| Trouble::Copy)?;
        let mut wal = file.as_os_str().to_owned();
        wal.push("-wal");
        let mut wal_target = target.as_os_str().to_owned();
        wal_target.push("-wal");
        let has_wal = copy_shared(Path::new(&wal), Path::new(&wal_target)).is_ok();
        let uri = format!("{}{}", uri(&target), if has_wal { "" } else { "?immutable=1" });
        let mut db = Db::open_raw(api, &uri, OPEN_READONLY | OPEN_URI)?;
        db._copy = Some(copy);
        Ok(db)
    }

    /// A database made or changed in place: only for building test fixtures.
    #[cfg(test)]
    pub fn create(file: &Path) -> Result<Db, Trouble> {
        let api = api().ok_or(Trouble::NoSqlite)?;
        Db::open_raw(api, &uri(file), OPEN_READWRITE | OPEN_CREATE | OPEN_URI)
    }

    fn open_raw(api: &'static Api, uri: &str, flags: c_int) -> Result<Db, Trouble> {
        let path = CString::new(uri).map_err(|_| Trouble::Sql)?;
        let mut handle: Handle = std::ptr::null_mut();
        // SAFETY: valid C string and out pointer; the handle is closed in
        // Drop even when opening failed, as SQLite asks.
        let rc = unsafe { (api.open_v2)(path.as_ptr(), &mut handle, flags, std::ptr::null()) };
        let db = Db { api, handle, _copy: None };
        if rc != OK {
            return Err(Trouble::Sql);
        }
        Ok(db)
    }

    /// Statements with no rows to give back: only for building fixtures.
    #[cfg(test)]
    pub fn exec(&self, sql: &str) -> Result<(), Trouble> {
        let text = CString::new(sql).map_err(|_| Trouble::Sql)?;
        // SAFETY: an open handle and a C string; no callback.
        let rc = unsafe { (self.api.exec)(self.handle, text.as_ptr(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if rc != OK { Err(Trouble::Sql) } else { Ok(()) }
    }

    /// Each row of one query, handed to `each`.
    pub fn rows(&self, sql: &str, mut each: impl FnMut(&Row)) -> Result<(), Trouble> {
        let text = CString::new(sql).map_err(|_| Trouble::Sql)?;
        let mut stmt: Handle = std::ptr::null_mut();
        // SAFETY: an open handle, a C string, out pointers.
        let rc = unsafe { (self.api.prepare_v2)(self.handle, text.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) };
        if rc != OK || stmt.is_null() {
            return Err(Trouble::Sql);
        }
        let row = Row { api: self.api, stmt, _db: std::marker::PhantomData };
        let result = loop {
            // SAFETY: a prepared statement, finalized below.
            match unsafe { (self.api.step)(stmt) } {
                ROW => each(&row),
                DONE => break Ok(()),
                _ => break Err(Trouble::Sql),
            }
        };
        // SAFETY: finalized once.
        unsafe { (self.api.finalize)(stmt) };
        result
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        // SAFETY: closed once; every statement was finalized in `rows`. The
        // copy (a field) is deleted after this, once SQLite let go of it.
        unsafe { (self.api.close)(self.handle) };
    }
}

/// `file:///C:/…` with anything not plainly safe escaped, as SQLite's URI
/// filenames want.
fn uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for b in text.trim_start_matches('/').bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Copied through a handle that lets the browser keep writing: std opens
/// with read, write and delete sharing, where CopyFile would be refused
/// while the browser holds the file.
fn copy_shared(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut source = File::open(from)?;
    if !source.metadata()?.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    let mut target = File::create(to)?;
    std::io::copy(&mut source, &mut target)?;
    Ok(())
}

/// A folder of our own under %TEMP% for one copy, deleted when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new() -> std::io::Result<Scratch> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("searchwin-import-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
impl Db {
    /// The folder holding this copy.
    pub fn scratch(&self) -> &Path {
        &self._copy.as_ref().expect("a snapshot").0
    }
}
