//! Passwords files in and out (Vault.swift:294 `take(csv:)`, `parse(csv:)`).
//!
//! In: one reader for every browser's export, by its header —
//! Chrome / Edge / Brave / Google (`name,url,username,password,note`),
//! Firefox (`url,username,password,httpRealm,…`), Bitwarden
//! (`…,login_uri,login_username,login_password,…`), Safari / Apple Passwords
//! (`Title,URL,Username,Password,Notes,OTPAuth`). Read once, kept, and the
//! file is the person's to delete: no copy is kept.
//!
//! Out: only on an explicit request, after [`crate::verify_user`] (the app
//! asks first; upstream has no export at all). Chrome's format, which every
//! one of the above reads back.

use crate::{host_of, wipe, Account, Result, Secret, Store};
use std::collections::BTreeMap;

const URL: &[&str] = &["url", "login_uri", "website", "site"];
const USER: &[&str] = &["username", "login_username", "user", "email"];
const PASS: &[&str] = &["password", "login_password"];
const NOTE: &[&str] = &["note", "notes"];

/// What a file held: the accounts to keep, and how many rows weren't one.
#[derive(Debug, Default)]
pub struct Parsed {
    /// One per host and user; a later row for the same one wins, as saving
    /// each in turn would leave it.
    pub accounts: Vec<Account>,
    /// Rows with no website (an Android app's, a card, a note), no password,
    /// or too few fields; or every row, when the header names no url,
    /// username and password columns.
    pub skipped: usize,
}

/// Reads an export. Nothing is kept: see [`take_csv`].
pub fn parse_csv(text: &str) -> Parsed {
    let mut rows = rows(text);
    if rows.is_empty() {
        return Parsed::default();
    }
    let header: Vec<String> = rows.remove(0).iter().map(|h| h.trim().to_lowercase()).collect();
    let column = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let (Some(url_at), Some(user_at), Some(pass_at)) = (column(URL), column(USER), column(PASS)) else {
        let skipped = rows.len();
        wipe_rows(rows);
        return Parsed { accounts: Vec::new(), skipped };
    };
    let note_at = column(NOTE);

    let mut skipped = 0;
    let mut kept: BTreeMap<(String, String), Account> = BTreeMap::new();
    for mut row in rows {
        if row.len() <= url_at.max(user_at).max(pass_at) {
            skipped += 1;
            wipe_row(&mut row);
            continue;
        }
        // Bitwarden puts every address of a login in one field.
        let url = row[url_at].split([',', '\n']).next().unwrap_or("").trim().to_owned();
        let host = host_of(&url);
        if host.is_empty() || row[pass_at].is_empty() {
            skipped += 1;
            wipe_row(&mut row);
            continue;
        }
        let password = Secret::new(std::mem::take(&mut row[pass_at]));
        let mut account = Account::new(&host, untext(&row[user_at]), password);
        account.clear = url.to_ascii_lowercase().starts_with("http://");
        if let Some(at) = note_at {
            account.note = row.get(at).map(|n| untext(n).to_owned()).unwrap_or_default();
        }
        wipe_row(&mut row);
        kept.insert((account.host.clone(), account.user.clone()), account);
    }
    Parsed { accounts: kept.into_values().collect(), skipped }
}

/// Reads an export and keeps every account in it: one already kept for the
/// same host and user is updated, so bringing the same file in twice adds
/// nothing twice (Browser.swift:1505 `keep`). Returns (kept, skipped).
pub fn take_csv(store: &mut dyn Store, text: &str) -> (usize, usize) {
    let parsed = parse_csv(text);
    let mut kept = 0;
    let mut skipped = parsed.skipped;
    for account in &parsed.accounts {
        match store.save(account) {
            Ok(()) => kept += 1,
            Err(_) => skipped += 1,
        }
    }
    (kept, skipped)
}

/// Every account kept, as a CSV in Chrome's format (`name,url,username,
/// password,note`), each secret read in turn. Call only on an explicit
/// request, after [`crate::verify_user`] said yes; the file it is written to
/// holds the passwords in the clear, and the app should say so.
pub fn export_csv(store: &dyn Store) -> Result<Secret> {
    let mut accounts = Vec::new();
    for kept in crate::list(store)? {
        if let Some(account) = store.account(&kept.host, &kept.user)? {
            accounts.push(account);
        }
    }
    // Sized once, so the buffer holding the passwords is never grown and
    // left behind half-copied.
    let size = 32 + accounts
        .iter()
        .map(|a| 2 * (2 * a.host.len() + a.user.len() + a.password.expose().len() + a.note.len()) + 40)
        .sum::<usize>();
    let mut out = String::with_capacity(size);
    out.push_str("name,url,username,password,note\n");
    for a in &accounts {
        let scheme = if a.clear { "http" } else { "https" };
        text(&mut out, &a.host);
        out.push(',');
        out.push_str(scheme);
        out.push_str("://");
        field(&mut out, &a.host);
        out.push_str("/,");
        text(&mut out, &a.user);
        out.push(',');
        field(&mut out, a.password.expose());
        out.push(',');
        text(&mut out, &a.note);
        out.push('\n');
    }
    Ok(Secret::new(out))
}

/// What a spreadsheet would run as a formula (CSV injection).
fn formula(value: &str) -> bool {
    value.starts_with(['=', '+', '-', '@', '\t', '\r'])
}

/// Whether [`text`] writes it with a `'` in front: it would run as a formula
/// (also in a cell after a `;`, pt-BR Excel's separator), or it already
/// starts with `'` before such a value, so that [`untext`] is its exact
/// inverse.
fn ticked(value: &str) -> bool {
    formula(value) || value.split(';').skip(1).any(formula) || value.strip_prefix('\'').is_some_and(ticked)
}

/// A field a person reads (name, user, note): see [`ticked`]; written `'…`,
/// quoted, and read back without the `'`. Never the password, which must come
/// back exactly as it is — so the export page says not to open the file in a
/// spreadsheet at all.
fn text(out: &mut String, value: &str) {
    if ticked(value) {
        out.push('"');
        out.push('\'');
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        field(out, value);
    }
}

/// Undoes [`text`]'s `'`.
fn untext(value: &str) -> &str {
    value.strip_prefix('\'').filter(|rest| ticked(rest)).unwrap_or(value)
}

/// One field, quoted when it has to be (`;` too, for pt-BR Excel).
fn field(out: &mut String, value: &str) {
    let quote = value.contains([',', ';', '"', '\n', '\r']) || value.starts_with(' ') || value.ends_with(' ');
    if !quote {
        out.push_str(value);
        return;
    }
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
}

fn wipe_row(row: &mut [String]) {
    for f in row {
        // SAFETY: zero bytes are valid UTF-8.
        wipe(unsafe { f.as_bytes_mut() });
    }
}

fn wipe_rows(rows: Vec<Vec<String>>) {
    for mut row in rows {
        wipe_row(&mut row);
    }
}

/// Quoted fields, doubled quotes inside them, and newlines inside those — all
/// three turn up in a real export. Blank lines are dropped.
fn rows(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\n' | '\r' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(c),
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    rows
}
