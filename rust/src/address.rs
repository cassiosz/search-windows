//! What you type has to be a place. This either hands back an address or hands
//! back nothing — never a guess. Port of upstream Sources/Search/Address.swift.

use url::Url;

/// Schemes the window can show itself. Anything else typed with a scheme
/// (mailto:, an app link) is somebody else's job and is refused here.
const OURS: [&str; 6] = ["http", "https", "file", "about", "data", crate::omnibox::INTERNAL_SCHEME];

/// A file on this PC, typed as C:\path, C:/path or ~\path. Only committed
/// navigation checks the disk; suggestions never do.
pub fn file(typed: &str) -> Option<Url> {
    let text = typed.trim();
    let bytes = text.as_bytes();
    let path = if let Some(rest) = text.strip_prefix("~\\").or_else(|| text.strip_prefix("~/")) {
        std::path::PathBuf::from(std::env::var_os("USERPROFILE")?).join(rest)
    } else if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/') {
        std::path::PathBuf::from(text)
    } else {
        return None;
    };
    if path.is_file() { Url::from_file_path(path).ok() } else { None }
}

pub fn url(typed: &str) -> Option<Url> {
    let text = typed.trim();
    if text.is_empty() || text.contains(' ') {
        return None;
    }

    // Written with a scheme, it is taken at its word.
    if let Some(split) = text.find("://") {
        let scheme = text[..split].to_ascii_lowercase();
        return if OURS.contains(&scheme.as_str()) { Url::parse(text).ok() } else { None };
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("about:") || lower.starts_with("data:") {
        return Url::parse(text).ok();
    }

    // Everything else has to look like a host before it gets a scheme.
    // "hello world" is not a website, and neither is "todo".
    let head = &text[..text.find(['/', '?', '#']).unwrap_or(text.len())];
    if head.contains('@') {
        return None; // an email address
    }
    if head.starts_with("[::1]") {
        return Url::parse(&format!("http://{text}")).ok();
    }
    let host = head.split(':').next().unwrap_or(head);
    if !looks_like_host(host) {
        return None;
    }

    // A local server almost never has a certificate.
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || private_range(host);
    Url::parse(&format!("{}://{text}", if local { "http" } else { "https" })).ok()
}

fn octets(host: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = host.split('.');
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Four numbers and nothing more: 10.io is a website, not 10.0.0.0/8.
fn private_range(host: &str) -> bool {
    matches!(octets(host), Some([10, ..]) | Some([192, 168, ..]) | Some([172, 16..=31, ..]))
}

/// Dev servers print 0.0.0.0 as the address to open; it is opened as localhost.
pub fn reachable(url: &Url) -> Option<Url> {
    if url.host_str() != Some("0.0.0.0") || !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut moved = url.clone();
    moved.set_host(Some("localhost")).ok()?;
    Some(moved)
}

fn looks_like_host(host: &str) -> bool {
    if host == "localhost" || octets(host).is_some() {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let fine = |label: &&str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    if !labels.iter().all(fine) {
        return false;
    }
    // A dotted thing ending in letters is a domain; ending in digits, a version number.
    let tld = labels[labels.len() - 1];
    tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic)
}

/// A page's address as the field shows it for editing: something that, sent
/// as it is, goes back to the same page.
pub fn editable(page: &Url) -> String {
    if page.scheme() == crate::omnibox::INTERNAL_SCHEME {
        return pretty(page);
    }
    let full = page.as_str();
    let prefix = format!("{}://", page.scheme());
    let Some(short) = full.strip_prefix(&prefix) else { return full.to_string() };
    let mut candidates = vec![short];
    if page.path() == "/" && page.query().is_none() && page.fragment().is_none() && short.ends_with('/') {
        candidates.insert(0, &short[..short.len() - 1]);
    }
    for candidate in candidates {
        if let Some(back) = url(candidate)
            && (back.as_str() == full || format!("{back}/") == full)
        {
            return candidate.to_string();
        }
    }
    full.to_string()
}

/// An address as a row shows it: without a leading www.
pub fn without_www(address: &str) -> &str {
    if address.len() >= 4 && address[..4].eq_ignore_ascii_case("www.") { &address[4..] } else { address }
}

/// What the tab says before the page has told us its title.
/// Our own pages keep their whole address, search://settings: it is what you type to get there.
pub fn pretty(url: &Url) -> String {
    if url.scheme() == crate::omnibox::INTERNAL_SCHEME {
        // WebView2 gives them back as search://settings/; the slash says nothing.
        let full = url.as_str();
        return full.strip_suffix('/').filter(|_| url.path() == "/").unwrap_or(full).to_string();
    }
    let Some(host) = url.host_str() else { return url.to_string() };
    let bare = without_www(host);
    match url.path() {
        "" | "/" => bare.to_string(),
        path => format!("{bare}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places() {
        for (typed, expected) in [
            ("example.com", "https://example.com/"),
            ("github.com/driceroland/Search", "https://github.com/driceroland/Search"),
            ("localhost:3000", "http://localhost:3000/"),
            ("192.168.0.10", "http://192.168.0.10/"),
            ("printer.local", "http://printer.local/"),
            ("10.io", "https://10.io/"),
            ("[::1]:8080", "http://[::1]:8080/"),
            ("http://example.com", "http://example.com/"),
        ] {
            assert_eq!(url(typed).map(String::from).as_deref(), Some(expected), "{typed}");
        }
    }

    #[test]
    fn not_places() {
        for typed in ["hello world", "todo", "1.2.3", "me@example.com", "mailto://x", "-bad.com"] {
            assert!(url(typed).is_none(), "{typed}");
        }
    }

    #[test]
    fn editable_forms() {
        for (page, shown) in [
            ("https://example.com/", "example.com"),
            ("http://example.com/", "http://example.com/"),
            ("http://localhost:3000/", "localhost:3000"),
            ("https://example.com/a?b=1", "example.com/a?b=1"),
        ] {
            assert_eq!(editable(&Url::parse(page).unwrap()), shown, "{page}");
        }
    }

    #[test]
    fn pretty_drops_www() {
        assert_eq!(pretty(&Url::parse("https://www.github.com/x").unwrap()), "github.com/x");
    }

    #[test]
    fn our_pages_as_typed() {
        let settings = url("search://settings").unwrap();
        assert_eq!(pretty(&settings), "search://settings");
        assert_eq!(editable(&settings), "search://settings");
        let slashed = Url::parse("search://settings/").unwrap();
        assert_eq!(pretty(&slashed), "search://settings");
        assert_eq!(editable(&slashed), "search://settings");
    }
}
