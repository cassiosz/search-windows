//! The app's hold on Shield: the paused sites, read once, and the two questions
//! pages ask of it. The rules themselves live in crates/shield.

use std::cell::RefCell;

use searchwin_shield::Shield;

thread_local! {
    static SHIELD: RefCell<Option<Shield>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Shield) -> R) -> R {
    SHIELD.with(|cell| {
        let mut cell = cell.borrow_mut();
        f(cell.get_or_insert_with(|| Shield::load(&crate::data_folder().join("shield.json"))))
    })
}

/// Nothing, with Shield switched off in Settings: pages already open stop blocking at once.
pub fn blocks(request_url: &str, page_url: Option<&str>) -> bool {
    crate::internal::prefs(|p| p.shield) && with(|shield| shield.should_block(request_url, page_url))
}

/// The page's ad slots, hidden from the first frame; nothing on a site Shield is paused for.
pub fn style_for(host: &str) -> String {
    if !crate::internal::prefs(|p| p.shield) {
        return String::new();
    }
    with(|shield| if shield.is_paused(host) { String::new() } else { searchwin_shield::cosmetic_css(host) })
}

thread_local! {
    /// How many requests Shield stopped since launch. Never shown in the window
    /// (upstream: "no counter"); only `bench state` reads it, to test Shield.
    static BLOCKED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub fn count_blocked() {
    BLOCKED.with(|b| b.set(b.get() + 1));
}

pub fn blocked() -> u64 {
    BLOCKED.with(|b| b.get())
}
