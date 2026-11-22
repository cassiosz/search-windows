//! searchwin-import: bringing things over from another browser on this PC
//! (port of the model half of upstream Import.swift, ImportArc.swift,
//! ImportSession.swift and ImportRecord.swift; the sheet is the app's).
//!
//! - `discover`: Chrome, Edge, Brave, Vivaldi, Opera, Helium, Comet, Arc and
//!   the rest, Firefox and its kin, each with its profiles by the names it
//!   gives them, and the one used last.
//! - `bookmarks`, `history`: Chromium's Bookmarks JSON (through
//!   searchwin-bookmarks) and History database; Firefox's places.sqlite.
//! - `open_tabs`: a Chromium browser's open tabs, pins and groups.
//! - `arc::sidebar`: Arc's spaces, pinned tabs and favourites.
//! - `Records`: what came from where, so bringing again adds only the new.
//!
//! Passwords aren't read: Chromium has kept them under app-bound encryption
//! since 2024, which only the browser itself can open. `password_hint` says
//! where each one exports a CSV, which the vault imports.
//!
//! Nothing here looks at a real browser unless handed this user's `Bases`;
//! databases are read from copies (see sqlite.rs).

pub mod arc;
mod browsers;
mod read;
mod record;
pub mod session;
mod sqlite;
#[cfg(test)]
mod tests;

pub use browsers::{Bases, Browser, Engine, Profile, discover};
pub use read::{HISTORY_LIMIT, bookmarks, history};
pub use record::{Added, Record, Records};
pub use session::{Snapshot, open_tabs};

/// Where `browser` exports its passwords as a CSV for the vault to take.
pub fn password_hint(browser: &Browser) -> String {
    let name = browser.name.as_str();
    let page = match name {
        n if n.starts_with("Chrome") || n == "Chromium" || n == "Helium" || n == "Comet" => {
            "chrome://password-manager/settings › Export passwords"
        }
        n if n.starts_with("Edge") => "edge://wallet/passwords › … › Export passwords",
        "Brave" => "brave://password-manager/settings › Export passwords",
        "Vivaldi" => "vivaldi://settings/passwords › ⋮ › Export passwords",
        "Opera" | "Opera GX" => "opera://password-manager/settings › Export passwords",
        "Arc" => "arc://password-manager/settings › Export passwords",
        _ if browser.engine == Engine::Firefox => "about:logins › ⋯ › Export Passwords",
        _ => "its password settings › Export passwords",
    };
    format!("{name} keeps its passwords locked to itself. In {name}, open {page}, save the CSV, then import that file here.")
}
