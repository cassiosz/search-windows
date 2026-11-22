//! Send links to a space (upstream SpaceRoutes.swift): links to chosen sites
//! open in their space, with its sign-ins, as Arc's Air Traffic Control
//! does. A rule is a site, its subdomains with it, and a space; the most
//! specific site wins (SpaceRoutes.swift:87-94).
//!
//! Off unless turned on, and only with spaces on (SpaceRoutes.swift:7). What
//! is sent is the app's call (a link from another app, or one a page opens
//! in a new tab from a page in another space); a link followed in its own
//! tab, an address typed, a private tab's links and a window the page sizes
//! stay where they are (SpaceRoutes.swift:11-20).

use serde::{Deserialize, Serialize};

/// One site to one space (SpaceRoute). `site` is a host, lowercased,
/// without `www.`: "notion.so".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub site: String,
    pub space: String,
}

/// What was typed, as the site a rule keeps (SpaceRoutes.swift:54-61):
/// "https://www.Notion.so/x", "notion.so" and "*.notion.so" are all
/// "notion.so". None for anything that isn't a host.
pub fn site(typed: &str) -> Option<String> {
    let mut text = typed.trim().to_lowercase();
    if let Some(rest) = text.strip_prefix("*.") {
        text = rest.to_string();
    }
    if text.is_empty() {
        return None;
    }
    if !text.contains("://") {
        text = format!("https://{text}");
    }
    let parsed = url::Url::parse(&text).ok()?;
    clean(parsed.host_str()?)
}

/// The site of a web address; None for anything but http and https
/// (SpaceRoutes.swift:64-67).
pub fn site_of(address: &str) -> Option<String> {
    let parsed = url::Url::parse(address).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    clean(&parsed.host_str()?.to_lowercase())
}

/// SpaceRoutes.swift:69-83: brackets off an IPv6 address, the root dot and a
/// leading `www.` off a name (but never `www.` alone off "www.com"); a name
/// has no empty label, and one with no dot is only ever localhost.
fn clean(host: &str) -> Option<String> {
    let mut host = host.trim_matches(|c| c == '[' || c == ']');
    host = host.strip_suffix('.').unwrap_or(host);
    if let Some(rest) = host.strip_prefix("www.")
        && rest.contains('.')
    {
        host = rest;
    }
    let allowed = |c: char| c.is_alphanumeric() || c == '-' || c == '.' || c == ':';
    if host.is_empty() || host.chars().count() > 253 || !host.chars().all(allowed) {
        return None;
    }
    if host.contains(':') {
        return Some(host.to_string());
    }
    if host.split('.').any(str::is_empty) || !(host.contains('.') || host == "localhost") {
        return None;
    }
    Some(host.to_string())
}

/// The rule for an address: the one whose site is its host or a domain of
/// it, the longest such site when more than one is (SpaceRoutes.swift:89-94).
pub fn find<'a>(address: &str, routes: &'a [Route]) -> Option<&'a Route> {
    let host = site_of(address)?;
    routes
        .iter()
        .filter(|r| host == r.site || (!host.contains(':') && host.ends_with(&format!(".{}", r.site))))
        .max_by_key(|r| r.site.len())
}
