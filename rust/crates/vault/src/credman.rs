//! Accounts in the Windows Credential Manager, as generic credentials.
//!
//! Mapping, one credential per account:
//! - TargetName: `<prefix><host>:<user>`, prefix `SearchWin:`; `%`, `:` and
//!   `*` in host and user written as `%25`, `%3A`, `%2A`, so the separator
//!   stays unambiguous and an enumeration filter (`SearchWin:*`) can't be
//!   widened by a user name.
//! - CredentialBlob: the password as UTF-16LE (what Windows' own UI writes),
//!   at most 2560 bytes (1280 UTF-16 units).
//! - UserName: the user, when it fits Credential Manager's 513 characters
//!   (the target holds it either way and is what is read back).
//! - Comment: `c=<created ms>;u=<used ms>;h` — `h` when kept from plain http.
//! - An attribute `note`: the note, cut to 256 bytes on a character boundary.
//! - Type CRED_TYPE_GENERIC, Persist CRED_PERSIST_LOCAL_MACHINE: kept across
//!   logons on this PC, not roamed with a domain profile.
//!
//! What it protects, plainly: Windows encrypts the blob with DPAPI under the
//! user's logon secret. Other Windows users, and someone with the disk but
//! not the password, can't read it. **Any program running as the same user
//! can** — CredEnumerate/CredRead need no permission — exactly as with
//! Chrome's and Edge's passwords on Windows. There is no per-application
//! access list like the macOS keychain's.
//!
//! Only credentials under this store's own prefix are ever listed, read,
//! written or deleted.

use crate::{check, merged, now_ms, wipe, Account, Error, Kept, Result, Secret, Store};
use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CREDENTIAL_ATTRIBUTEW,
    CRED_ENUMERATE_FLAGS, CRED_FLAGS, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_MAX_USERNAME_LENGTH,
    CRED_MAX_VALUE_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

const NOTE: &str = "note";

/// Credential Manager, under one prefix of target names.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    prefix: String,
}

impl Default for CredentialStore {
    fn default() -> Self {
        CredentialStore { prefix: "SearchWin:".into() }
    }
}

impl CredentialStore {
    /// The app's own: `SearchWin:`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Another namespace — a test run's (`SearchWin-test:`) or a profile's.
    /// It must end with `:` and hold no `*`, so that an enumeration under it
    /// is exactly its own.
    pub fn with_prefix(prefix: &str) -> Result<Self> {
        if prefix.len() < 2 || !prefix.ends_with(':') || prefix.contains('*') {
            return Err(Error::Invalid("prefix must end with ':' and hold no '*'"));
        }
        Ok(CredentialStore { prefix: prefix.into() })
    }

    /// A store that refuses everything without asking Windows: where a test
    /// run has no prefix of its own, so it can't reach what you keep.
    pub fn refusing() -> Self {
        CredentialStore { prefix: String::new() }
    }

    fn open(&self) -> Result<()> {
        if self.prefix.is_empty() {
            return Err(Error::Invalid("no vault for this run"));
        }
        Ok(())
    }

    /// Whether the prefix is the app's own, or inside it (target names are
    /// case-insensitive): never a test run's.
    pub fn is_real(&self) -> bool {
        self.prefix.to_ascii_lowercase().starts_with("searchwin:")
    }

    fn target(&self, host: &str, user: &str) -> Vec<u16> {
        wide(&format!("{}{}:{}", self.prefix, escape(host), escape(user)))
    }

    /// (host, user) of one of ours.
    fn parse_target(&self, target: &str) -> Option<(String, String)> {
        let rest = target.strip_prefix(&self.prefix)?;
        let (host, user) = rest.split_once(':')?;
        Some((unescape(host)?, unescape(user)?))
    }

    /// One credential, read and freed. `f` sees it before it is wiped.
    fn read<T>(&self, host: &str, user: &str, f: impl FnOnce(&CREDENTIALW) -> T) -> Result<Option<T>> {
        let target = self.target(host, user);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: a NUL-terminated target; `cred` is freed below.
        match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) } {
            Ok(()) => {}
            Err(e) if not_found(e.code()) => return Ok(None),
            Err(e) => return Err(Error::Os(e.code().0)),
        }
        // SAFETY: CredReadW succeeded, so `cred` points at one CREDENTIALW.
        let out = unsafe {
            let c = &*cred;
            let out = f(c);
            wipe_blob(c);
            CredFree(cred as *const _);
            out
        };
        Ok(Some(out))
    }
}

impl Store for CredentialStore {
    fn all(&self) -> Result<Vec<Kept>> {
        self.open()?;
        let filter = wide(&format!("{}*", self.prefix));
        let mut count = 0u32;
        let mut list: *mut *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: NUL-terminated filter; `list` is freed below.
        match unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), Some(CRED_ENUMERATE_FLAGS(0)), &mut count, &mut list) } {
            Ok(()) => {}
            Err(e) if not_found(e.code()) => return Ok(Vec::new()),
            Err(e) => return Err(Error::Os(e.code().0)),
        }
        let mut out = Vec::with_capacity(count as usize);
        // SAFETY: CredEnumerateW gave `count` valid pointers in one block.
        unsafe {
            for &cred in std::slice::from_raw_parts(list, count as usize) {
                let c = &*cred;
                if c.Type == CRED_TYPE_GENERIC
                    && let Some(kept) = self.kept(c)
                {
                    out.push(kept);
                }
                // The enumeration hands every secret over too: not kept.
                wipe_blob(c);
            }
            CredFree(list as *const _);
        }
        Ok(out)
    }

    fn account(&self, host: &str, user: &str) -> Result<Option<Account>> {
        self.open()?;
        let found = self.read(host, user, |c| {
            // SAFETY: a credential Windows just returned.
            let password = unsafe { blob(c) }?;
            let kept = unsafe { self.kept(c) }?;
            Some(Account {
                host: kept.host,
                user: kept.user,
                password,
                created: kept.created,
                used: kept.used,
                clear: kept.clear,
                note: kept.note,
            })
        })?;
        Ok(found.flatten())
    }

    fn save(&mut self, account: &Account) -> Result<()> {
        self.open()?;
        check(account)?;
        // SAFETY: a credential Windows just returned.
        let old = self.read(&account.host, &account.user, |c| unsafe { self.kept(c) })?.flatten();
        let kept = merged(old.as_ref(), account, now_ms());

        let mut password: Vec<u8> = account.password.expose().encode_utf16().flat_map(u16::to_le_bytes).collect();
        if password.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            wipe(&mut password);
            return Err(Error::Invalid("password longer than Credential Manager keeps"));
        }
        let mut target = self.target(&account.host, &account.user);
        let mut comment = wide(&meta(&kept));
        let mut user = wide(&account.user);
        let fits_user = account.user.encode_utf16().count() <= CRED_MAX_USERNAME_LENGTH as usize;
        let mut keyword = wide(NOTE);
        let mut note = cut(&kept.note, CRED_MAX_VALUE_SIZE as usize).as_bytes().to_vec();
        let mut attribute = CREDENTIAL_ATTRIBUTEW {
            Keyword: PWSTR(keyword.as_mut_ptr()),
            Flags: 0,
            ValueSize: note.len() as u32,
            Value: note.as_mut_ptr(),
        };
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            Comment: PWSTR(comment.as_mut_ptr()),
            CredentialBlobSize: password.len() as u32,
            CredentialBlob: password.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: u32::from(!note.is_empty()),
            Attributes: if note.is_empty() { std::ptr::null_mut() } else { &mut attribute },
            UserName: if fits_user { PWSTR(user.as_mut_ptr()) } else { PWSTR::null() },
            ..Default::default()
        };
        // SAFETY: every pointer in `cred` lives until after the call.
        let written = unsafe { CredWriteW(&cred, 0) };
        wipe(&mut password);
        written.map_err(|e| Error::Os(e.code().0))
    }

    fn delete(&mut self, host: &str, user: &str) -> Result<bool> {
        self.open()?;
        let target = self.target(host, user);
        // SAFETY: NUL-terminated target.
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(true),
            Err(e) if not_found(e.code()) => Ok(false),
            Err(e) => Err(Error::Os(e.code().0)),
        }
    }
}

impl CredentialStore {
    /// One of ours, without its secret; `None` for anything else.
    ///
    /// # Safety
    /// `c` must be a credential Windows returned, still allocated.
    unsafe fn kept(&self, c: &CREDENTIALW) -> Option<Kept> {
        let target = unsafe { text(c.TargetName) }?;
        let (host, user) = self.parse_target(&target)?;
        let comment = unsafe { text(c.Comment) }.unwrap_or_default();
        let mut kept = Kept { host, user, created: None, used: None, clear: false, note: String::new() };
        for part in comment.split(';') {
            match part.split_once('=') {
                Some(("c", v)) => kept.created = v.parse().ok(),
                Some(("u", v)) => kept.used = v.parse().ok(),
                None if part == "h" => kept.clear = true,
                _ => {}
            }
        }
        if !c.Attributes.is_null() {
            let attributes = unsafe { std::slice::from_raw_parts(c.Attributes, c.AttributeCount as usize) };
            for a in attributes {
                if unsafe { text(a.Keyword) }.as_deref() == Some(NOTE) && !a.Value.is_null() {
                    let bytes = unsafe { std::slice::from_raw_parts(a.Value, a.ValueSize as usize) };
                    kept.note = String::from_utf8_lossy(bytes).into_owned();
                }
            }
        }
        Some(kept)
    }
}

/// The password in a credential's blob (UTF-16LE).
///
/// # Safety
/// `c` must be a credential Windows returned, still allocated.
unsafe fn blob(c: &CREDENTIALW) -> Option<Secret> {
    if c.CredentialBlob.is_null() || !c.CredentialBlobSize.is_multiple_of(2) {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize) };
    let mut units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
    let text = String::from_utf16(&units).ok();
    for u in units.iter_mut() {
        // SAFETY: a valid exclusive reference.
        unsafe { std::ptr::write_volatile(u, 0) };
    }
    text.map(Secret::new)
}

/// Overwrites a returned credential's secret before it is freed.
///
/// # Safety
/// `c` must be a credential Windows returned, still allocated.
unsafe fn wipe_blob(c: &CREDENTIALW) {
    if !c.CredentialBlob.is_null() {
        wipe(unsafe { std::slice::from_raw_parts_mut(c.CredentialBlob, c.CredentialBlobSize as usize) });
    }
}

/// # Safety
/// `p` must be null or a NUL-terminated UTF-16 string.
unsafe fn text(p: PWSTR) -> Option<String> {
    if p.is_null() { None } else { unsafe { p.to_string() }.ok() }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn not_found(code: HRESULT) -> bool {
    code == ERROR_NOT_FOUND.to_hresult()
}

fn meta(k: &Kept) -> String {
    let mut out = String::new();
    if let Some(c) = k.created {
        out.push_str(&format!("c={c};"));
    }
    if let Some(u) = k.used {
        out.push_str(&format!("u={u};"));
    }
    if k.clear {
        out.push('h');
    }
    out
}

/// At most `max` bytes, on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn escape(s: &str) -> String {
    s.replace('%', "%25").replace(':', "%3A").replace('*', "%2A")
}

fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let code = rest.get(i + 1..i + 3)?;
        out.push(match code {
            "25" => '%',
            "3A" => ':',
            "2A" => '*',
            _ => return None,
        });
        rest = &rest[i + 3..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_escape_and_round_trip() {
        let store = CredentialStore::with_prefix("SearchWin-test:").unwrap();
        let t = String::from_utf16(&store.target("example.com", "a:b%c*d")).unwrap();
        assert_eq!(t.trim_end_matches('\0'), "SearchWin-test:example.com:a%3Ab%25c%2Ad");
        assert_eq!(
            store.parse_target(t.trim_end_matches('\0')),
            Some(("example.com".into(), "a:b%c*d".into()))
        );
        assert_eq!(store.parse_target("SearchWin:example.com:x"), None);
        assert!(CredentialStore::with_prefix("SearchWin-test").is_err());
        assert!(CredentialStore::with_prefix("Search*:").is_err());
        assert_eq!(cut("aé", 2), "a");
    }

    /// Writes, reads and deletes under `SearchWin-test:` only, and removes
    /// whatever it finds there at the start and the end. Never touches any
    /// other credential. Run with `cargo test -p searchwin-vault -- --ignored`.
    #[test]
    #[ignore = "writes to this user's Credential Manager (own SearchWin-test: prefix, cleaned up)"]
    fn credential_manager_round_trip() {
        /// Empties the test prefix now and again on the way out, panic or not.
        struct Clean(CredentialStore);
        impl Clean {
            fn sweep(&mut self) {
                for k in self.0.all().unwrap_or_default() {
                    let _ = self.0.delete(&k.host, &k.user);
                }
            }
        }
        impl Drop for Clean {
            fn drop(&mut self) {
                self.sweep();
            }
        }
        let mut clean = Clean(CredentialStore::with_prefix("SearchWin-test:").unwrap());
        clean.sweep();
        let store = &mut clean.0;
        assert!(store.all().unwrap().is_empty());

        let mut a = Account::new("example.com", "ana:x", Secret::new("s3cret-ü"));
        a.note = "é".repeat(200);
        a.clear = true;
        store.save(&a).unwrap();
        let got = store.account("example.com", "ana:x").unwrap().unwrap();
        assert!(got.password == a.password, "password read back differs");
        assert!(got.clear && got.created.is_some());
        assert_eq!(got.note, "é".repeat(128));
        assert_eq!(store.all().unwrap().len(), 1);

        store.save(&Account::new("example.com", "ana:x", Secret::new("new"))).unwrap();
        let updated = store.account("example.com", "ana:x").unwrap().unwrap();
        assert!(updated.password == Secret::new("new"), "update not written");
        assert!(updated.created == got.created && !updated.clear && updated.note == got.note);

        assert!(store.delete("example.com", "ana:x").unwrap());
        assert!(!store.delete("example.com", "ana:x").unwrap());
        assert!(store.all().unwrap().is_empty());
    }
}
