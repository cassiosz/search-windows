//! The one field: an address goes there, anything else is searched.
//! Nothing typed leaves the PC until Return.

use crate::{address, engine::Engine};
use url::Url;

/// The browser's own pages, search://settings and the like.
pub const INTERNAL_SCHEME: &str = "search";

pub fn resolve(typed: &str, engine: Engine, custom: &str) -> Option<Url> {
    let text = typed.trim();
    if text.is_empty() {
        return None;
    }
    match address::file(text).or_else(|| address::url(text)) {
        Some(url) => Some(address::reachable(&url).unwrap_or(url)),
        None => crate::engine::search(engine, text, custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_searched() {
        let url = resolve("gatos fofos", Engine::Google, "").unwrap();
        assert_eq!(url.as_str(), "https://www.google.com/search?q=gatos%20fofos");
    }

    #[test]
    fn zeroes_open_as_localhost() {
        assert_eq!(resolve("0.0.0.0:5173", Engine::Google, "").unwrap().as_str(), "http://localhost:5173/");
    }
}
