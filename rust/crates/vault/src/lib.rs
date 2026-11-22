//! Saved passwords: the model, where they are kept, and the rules for which
//! account is offered where. Port of upstream Vault.swift, the offer and
//! save rules of Browser.swift / Tab.swift (`hang`, `choose`, `onCredentials`,
//! `sentSignIn`, `settleSignIn`, `gaveName`) and the site notion of
//! Registrable.swift. The page script that finds sign-in boxes (Forms.swift)
//! is not here: this crate only judges what the app tells it.
//!
//! Nothing is filled on its own: [`offers`] only lists accounts for the box
//! the caret is in, and the app fills one after a click, re-checking with
//! [`Offered::may_fill`] that the page is still the one it was offered on.
//!
//! Where they live, on Windows ([`CredentialStore`]): the Windows Credential
//! Manager, as generic credentials, protected by DPAPI under the Windows
//! user's own logon secret. That keeps them from other Windows users and from
//! anyone holding the disk without the user's password. It does **not** keep
//! them from other programs running as the same user: any process of that
//! user can enumerate and read them. That is the same protection Chrome and
//! Edge have on Windows; there is no per-app access list like the macOS
//! keychain's ("readable only by Search" is not true here, and nothing in
//! this crate pretends it is). Showing, copying or exporting a password is
//! therefore gated by the app on [`verify_user`] (Windows Hello), as upstream
//! gates them on Touch ID — a check against a person at the keyboard, not
//! against other programs.
//!
//! Time is milliseconds since the Unix epoch, passed in, so every rule is
//! testable.

mod csv;
mod site;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod credman;
#[cfg(windows)]
mod hello;

pub use csv::{export_csv, parse_csv, take_csv, Parsed};
pub use site::{host_of, page_host, registrable, same_site};
#[cfg(windows)]
pub use credman::CredentialStore;
#[cfg(windows)]
pub use hello::verify_user;

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// At most this many accounts hang from a box (Browser.swift:1023 `prefix(5)`).
pub const OFFERED_AT_MOST: usize = 5;
/// A sign-in sent this long ago is no longer judged (Tab.swift:1324, 45 s).
pub const SENT_FOR_MS: u64 = 45_000;
/// Boxes a page took away by itself are judged this long after (Tab.swift:1331).
pub const SETTLE_AFTER_MS: u64 = 1_500;
/// A name sent on a first step counts for the password step this long
/// (Tab.swift:1281, 300 s).
pub const NAME_FOR_MS: u64 = 300_000;

// ---------------------------------------------------------------------------
// errors

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// What was asked to be kept can't be: no host, no password, too long.
    Invalid(&'static str),
    /// The system said no: an HRESULT from Credential Manager or WinRT.
    Os(i32),
    /// Windows Hello can't ask here (not set up, no device, turned off by
    /// policy). The UserConsentVerifierAvailability value.
    NoVerifier(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(why) => write!(f, "can't keep it: {why}"),
            Error::Os(code) => write!(f, "Windows refused it (0x{:08X})", *code as u32),
            Error::NoVerifier(why) => write!(f, "Windows Hello can't ask here ({why})"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// the secret

/// A password, held only as long as it is needed. Its `Debug` and `Display`
/// say nothing of it, it is not `Serialize`, and its bytes are overwritten
/// when it is dropped. (Copies the allocator made while it was being built —
/// a `String` that grew, a buffer it came from — are not reached; this keeps
/// the one it owns from lingering, it is not a guarantee about the heap.)
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Secret(text.into())
    }
    /// The password itself, for the one place that hands it on: the form
    /// filler, as an argument, or the clipboard after [`verify_user`].
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Overwrite bytes in a way the optimiser can't drop.
pub(crate) fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        // SAFETY: a valid, aligned, exclusive reference to one byte.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

impl Drop for Secret {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, so the string stays valid.
        wipe(unsafe { self.0.as_bytes_mut() });
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Secret(self.0.clone())
    }
}

impl PartialEq for Secret {
    /// Without stopping at the first difference.
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.0.as_bytes(), other.0.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}
impl Eq for Secret {}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(••••••••••)")
    }
}

// ---------------------------------------------------------------------------
// the model

/// One account kept, without its secret: what a list needs (Vault.swift
/// `Kept`). Nothing that writes a password or fills one takes this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Kept {
    /// The host it was kept for, lowercase, without `www.` — the key, as
    /// upstream keys the keychain item (`kSecAttrServer`). Its site is
    /// [`registrable`] of it.
    pub host: String,
    /// May be empty: a sign-in with a password box alone.
    pub user: String,
    pub created: Option<u64>,
    /// Last used to sign in. Newest first in lists.
    pub used: Option<u64>,
    /// Kept from a page sent over plain http. Only these are offered on such
    /// a page, and an http page is offered nothing else (Vault.swift:18).
    pub clear: bool,
    pub note: String,
}

impl Kept {
    pub fn site(&self) -> &str {
        registrable(&self.host)
    }
}

/// One account with its secret (Vault.swift `Login`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub host: String,
    pub user: String,
    pub password: Secret,
    pub created: Option<u64>,
    pub used: Option<u64>,
    pub clear: bool,
    pub note: String,
}

impl Account {
    pub fn new(host: &str, user: &str, password: Secret) -> Self {
        Account {
            host: host.to_owned(),
            user: user.to_owned(),
            password,
            created: None,
            used: None,
            clear: false,
            note: String::new(),
        }
    }

    pub fn kept(&self) -> Kept {
        Kept {
            host: self.host.clone(),
            user: self.user.clone(),
            created: self.created,
            used: self.used,
            clear: self.clear,
            note: self.note.clone(),
        }
    }

    pub fn site(&self) -> &str {
        registrable(&self.host)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// where they are kept

/// Where accounts are kept. Lists carry no secrets; a secret is read for one
/// account at a time, when it is picked, shown or copied (Vault.swift:58).
pub trait Store {
    /// Everything kept, without secrets, in no particular order. See [`list`].
    fn all(&self) -> Result<Vec<Kept>>;
    /// One account with its secret, by the two things that name it.
    fn account(&self, host: &str, user: &str) -> Result<Option<Account>>;
    /// Keeps it, or updates the one with the same host and user. A missing
    /// `used` keeps the one already kept; `created` is the first save's.
    fn save(&mut self, account: &Account) -> Result<()>;
    /// Whether there was one to let go.
    fn delete(&mut self, host: &str, user: &str) -> Result<bool>;
    /// What is kept for exactly this host (Vault.swift `kept(for:)`).
    fn kept_for(&self, host: &str) -> Result<Vec<Kept>> {
        Ok(self.all()?.into_iter().filter(|k| k.host == host).collect())
    }
}

/// Vault.swift:187: nothing without a host or a password.
pub(crate) fn check(account: &Account) -> Result<()> {
    if account.host.is_empty() {
        return Err(Error::Invalid("no site"));
    }
    if account.password.is_empty() {
        return Err(Error::Invalid("no password"));
    }
    Ok(())
}

/// What an update keeps of what was there: the first `created`, the last
/// known `used` when none is given (Vault.swift:209 writes the comment only
/// when given), the old note when none is given.
pub(crate) fn merged(old: Option<&Kept>, new: &Account, now: u64) -> Kept {
    let mut kept = new.kept();
    kept.created = old.and_then(|o| o.created).or(new.created).or(Some(now));
    kept.used = new.used.or(old.and_then(|o| o.used));
    if kept.note.is_empty() {
        kept.note = old.map(|o| o.note.clone()).unwrap_or_default();
    }
    kept
}

/// Kept in memory only: for tests, and nothing else.
#[derive(Debug, Default)]
pub struct MemoryStore {
    items: BTreeMap<(String, String), (Kept, Secret)>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemoryStore {
    fn all(&self) -> Result<Vec<Kept>> {
        Ok(self.items.values().map(|(k, _)| k.clone()).collect())
    }

    fn account(&self, host: &str, user: &str) -> Result<Option<Account>> {
        Ok(self.items.get(&(host.to_owned(), user.to_owned())).map(|(k, s)| Account {
            host: k.host.clone(),
            user: k.user.clone(),
            password: s.clone(),
            created: k.created,
            used: k.used,
            clear: k.clear,
            note: k.note.clone(),
        }))
    }

    fn save(&mut self, account: &Account) -> Result<()> {
        check(account)?;
        let key = (account.host.clone(), account.user.clone());
        let kept = merged(self.items.get(&key).map(|(k, _)| k), account, now_ms());
        self.items.insert(key, (kept, account.password.clone()));
        Ok(())
    }

    fn delete(&mut self, host: &str, user: &str) -> Result<bool> {
        Ok(self.items.remove(&(host.to_owned(), user.to_owned())).is_some())
    }
}

/// Everything kept, for the Passwords page: by host, then user
/// (Vault.swift:101).
pub fn list(store: &dyn Store) -> Result<Vec<Kept>> {
    let mut all = store.all()?;
    all.sort_by(|a, b| a.host.cmp(&b.host).then_with(|| a.user.cmp(&b.user)));
    Ok(all)
}

/// The accounts kept for exactly this host, newest first. Upstream's `wide`
/// (the site's other hosts too, Vault.swift:86) is left out: the page said
/// whether a box was "clearly a sign-in", and a page can't widen what it gets.
pub fn kept_matching(store: &dyn Store, host: &str) -> Result<Vec<Kept>> {
    let mut own = store.kept_for(host)?;
    own.sort_by_key(|k| std::cmp::Reverse(k.used.unwrap_or(0)));
    Ok(own)
}

/// It was just used to sign in: lists put it first from now on. An https
/// sign-in makes an http-kept account https-only; an http one never makes an
/// https account plain (should_offer_save doesn't even get here for it).
pub fn touch(store: &mut dyn Store, host: &str, user: &str, clear: bool, now: u64) -> Result<bool> {
    let Some(mut account) = store.account(host, user)? else { return Ok(false) };
    account.used = Some(now);
    account.clear &= clear;
    store.save(&account)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// which accounts are offered where

/// The box the caret is in, as the page script and the app describe it.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    /// The tab's page — the one on screen, not one still loading.
    pub page: &'a Url,
    /// The frame the box is in, when it isn't the page itself.
    pub frame: Option<&'a Url>,
    /// The name the step before was sent with ([`GivenName::for_page`]): that
    /// account comes first (Browser.swift:1021).
    pub given_name: Option<&'a str>,
}

/// What was offered under a box, and for which page: a pick fills only a page
/// that is still that one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub host: String,
    pub clear: bool,
    pub accounts: Vec<Kept>,
}

impl Offered {
    /// Browser.swift:1355 `choose`: the tab may have gone somewhere else while
    /// the list was up — a redirect, a script. What was offered for one site
    /// is never put into another's page, nor an https page's into an http
    /// one of the same name.
    pub fn may_fill(&self, page_now: &Url) -> bool {
        page_host(page_now).as_deref() == Some(self.host.as_str())
            && (page_now.scheme() == "http") == self.clear
    }
}

/// The accounts that hang from a sign-in box (Browser.swift:977 `hang`):
///
/// - only on an http or https page with a host;
/// - in a frame only of the page's own origin (scheme, host and port): a
///   frame's box is offered nothing the frame's own host and scheme wouldn't
///   get as the page (stricter than ApplePasswordsOffer.swift:78's same site);
/// - the page's exact host's accounts alone (see [`kept_matching`]);
/// - on a plain-http page, only accounts kept from plain http — never the
///   https site's of the same name (Browser.swift:1000, :1020);
/// - the name given on the step before first, then at most five.
///
/// `None`: nothing may be offered here at all.
pub fn offers(store: &dyn Store, field: &Field<'_>) -> Result<Option<Offered>> {
    let Some(host) = page_host(field.page) else { return Ok(None) };
    if let Some(frame) = field.frame
        && (frame.scheme() != field.page.scheme() || page_host(frame).as_deref() != Some(host.as_str()))
    {
        return Ok(None);
    }
    let clear = field.page.scheme() == "http";
    let mut kept: Vec<Kept> = kept_matching(store, &host)?
        .into_iter()
        .filter(|k| !clear || k.clear)
        .collect();
    if let Some(given) = field.given_name
        && let Some(at) = kept.iter().position(|k| k.user.to_lowercase() == given.to_lowercase())
    {
        let first = kept.remove(at);
        kept.insert(0, first);
    }
    kept.truncate(OFFERED_AT_MOST);
    Ok(Some(Offered { host, clear, accounts: kept }))
}

// ---------------------------------------------------------------------------
// a sign-in that worked, and the offer to keep it

/// A name sent on a first step (Google, Microsoft, impots.gouv.fr), for the
/// password page after it. In memory only, per tab, for the same host (with
/// its port, as accounts are kept; upstream: the same site) and a few minutes
/// (Tab.swift:1264).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenName {
    host: String,
    user: String,
    at: u64,
}

impl GivenName {
    /// Tab.swift:1271 `gaveName`: not empty, 256 characters at most.
    pub fn new(page: &Url, user: &str, now: u64) -> Option<Self> {
        if user.is_empty() || user.chars().count() > 256 {
            return None;
        }
        Some(GivenName { host: page_host(page)?, user: user.to_owned(), at: now })
    }

    /// Tab.swift:1280 `givenName`: this host's, and recent.
    pub fn for_page(&self, page: &Url, now: u64) -> Option<&str> {
        (now.saturating_sub(self.at) < NAME_FOR_MS && page_host(page)? == self.host).then_some(self.user.as_str())
    }
}

/// A name and password a page has just sent: held, not yet offered. Whether
/// the sign-in worked is only known afterwards (Tab.swift:1287).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub host: String,
    pub user: String,
    pub password: Secret,
    pub clear: bool,
    pub at: u64,
}

impl Sent {
    /// Tab.swift:1293 `sentSignIn`: the host now, while the page is still the
    /// sign-in page (lowercase, without `www.`); a password page with no name
    /// takes the name the step before was sent with. Only http(s) pages.
    pub fn new(page: &Url, user: &str, password: Secret, given: Option<&str>, now: u64) -> Option<Self> {
        let host = page_host(page)?;
        let user = if user.is_empty() { given.unwrap_or("") } else { user };
        Some(Sent {
            host,
            user: user.to_owned(),
            password,
            clear: page.scheme() == "http",
            at: now,
        })
    }
}

/// What to do with a held sign-in once the page has moved (Tab.swift:1322
/// `settleSignIn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// Too old: forget it.
    Drop,
    /// The boxes went without a new page: judge again [`SETTLE_AFTER_MS`]
    /// later (a sign-in built into the page closes its form at once and puts
    /// it back if the server says no), if nothing newer was sent meanwhile.
    Later,
    /// The password box is still there: refused, or a second step. Keep it.
    Keep,
    /// It took: ask [`should_offer_save`].
    Worked,
}

/// `navigated`: a new document loaded (as opposed to the boxes going away in
/// place). `password_box`: whether the page, now, still has a password box.
pub fn settle(sent: &Sent, now: u64, navigated: bool, password_box: bool) -> Settle {
    if now.saturating_sub(sent.at) >= SENT_FOR_MS {
        Settle::Drop
    } else if !navigated {
        Settle::Later
    } else if password_box {
        Settle::Keep
    } else {
        Settle::Worked
    }
}

/// What the app knows about the tab a sign-in was sent from.
#[derive(Debug, Clone, Copy)]
pub struct SaveContext<'a> {
    /// Settings › Passwords › Offer to save passwords.
    pub saves_passwords: bool,
    pub private_tab: bool,
    /// The page is an extension's own (Bitwarden's unlock PIN, #385).
    pub extension_page: bool,
    /// Sites asked never to be asked about (Vault.swift `never`).
    pub never: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOffer {
    /// Say nothing.
    Nothing,
    /// Already kept as it is: [`touch`] it (host, user, clear), say nothing.
    Touch,
    /// Ask "Save password?" — or "Update password?" when `changed`.
    Offer { changed: bool },
}

/// Browser.swift:6689 `onCredentials`: whether a sign-in that worked is
/// offered a place. Never with saving off, an empty password, in a private
/// tab, on a site asked never to be asked about, or on an extension's page.
/// One already kept with the same password is touched, not offered; an http
/// sign-in for an https-kept account is neither.
pub fn should_offer_save(store: &dyn Store, sent: &Sent, ctx: &SaveContext<'_>) -> Result<SaveOffer> {
    if !ctx.saves_passwords
        || sent.password.is_empty()
        || ctx.private_tab
        || ctx.extension_page
        || is_never(ctx.never, &sent.host)
    {
        return Ok(SaveOffer::Nothing);
    }
    let known = store.kept_for(&sent.host)?;
    // An http sign-in is never merged into an https account: that would make
    // it offered (and updatable) over plain http from then on.
    if sent.clear && known.iter().any(|k| k.user == sent.user && !k.clear) {
        return Ok(SaveOffer::Nothing);
    }
    let same_user = known.iter().any(|k| k.user == sent.user);
    if same_user
        && let Some(kept) = store.account(&sent.host, &sent.user)?
        && kept.password == sent.password
    {
        return Ok(SaveOffer::Touch);
    }
    Ok(SaveOffer::Offer { changed: same_user })
}

/// The answer Save: keep it, used now (Browser.swift:945 `keepOffer`).
pub fn keep_sent(store: &mut dyn Store, sent: &Sent, now: u64) -> Result<()> {
    let mut account = Account::new(&sent.host, &sent.user, sent.password.clone());
    account.used = Some(now);
    account.clear = sent.clear;
    store.save(&account)
}

/// Vault.swift:246 `isNever`: the host, or its site.
pub fn is_never(never: &[String], host: &str) -> bool {
    let site = registrable(host);
    never.iter().any(|n| n == host || n == site)
}
