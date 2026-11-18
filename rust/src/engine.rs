//! Search engines, as upstream Sources/Search/Engine.swift.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

/// Which engine is a setting, so the engines are named in crates/prefs.
pub use searchwin_prefs::Engine;

/// Everything but RFC 3986's unreserved characters is escaped: a space is %20.
const QUERY: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

pub fn template(engine: Engine, custom: &str) -> String {
    match engine {
        Engine::Google => "https://www.google.com/search?q=%s".into(),
        Engine::DuckDuckGo => "https://duckduckgo.com/?q=%s".into(),
        Engine::Bing => "https://www.bing.com/search?q=%s".into(),
        Engine::Ecosia => "https://www.ecosia.org/search?q=%s".into(),
        Engine::Startpage => "https://www.startpage.com/sp/search?query=%s".into(),
        Engine::Kagi => "https://kagi.com/search?q=%s".into(),
        Engine::Brave => "https://search.brave.com/search?q=%s".into(),
        Engine::Qwant => "https://www.qwant.com/?q=%s".into(),
        Engine::Custom => {
            let written = written(custom);
            if accepts(&written) { written } else { template(Engine::default(), "") }
        }
    }
}

pub fn search(engine: Engine, words: &str, custom: &str) -> Option<Url> {
    let words = utf8_percent_encode(words.trim(), QUERY).to_string();
    Url::parse(&template(engine, custom).replace("%s", &words)).ok()
}

/// A custom template as people bring it: with {searchTerms}, or without https://.
pub fn written(template: &str) -> String {
    let text = template.trim().replace("{searchTerms}", "%s");
    if !text.is_empty() && !text.contains("://") { format!("https://{text}") } else { text }
}

pub fn accepts(template: &str) -> bool {
    template.contains("%s")
        && Url::parse(&template.replace("%s", "x"))
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_engine_from_another_browser() {
        assert_eq!(written("s.example/?q={searchTerms}"), "https://s.example/?q=%s");
    }

    #[test]
    fn custom_engine_that_cannot_search_falls_back() {
        assert_eq!(template(Engine::Custom, "not a template"), template(Engine::Google, ""));
        assert_eq!(template(Engine::Custom, "s.example/?q=%s"), "https://s.example/?q=%s");
    }
}
