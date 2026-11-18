//! Shield, the ad blocker (upstream Shield.swift, YouTubeAds.swift,
//! Registrable.swift). No settings, no counter: a short list, decided cheaply.
//!
//! On macOS the list is compiled into WebKit. Here the app registers one
//! WebView2 `WebResourceRequested` filter per entry of [`filters`], so only
//! requests that might be blocked ever reach the app, and then asks
//! [`Shield::should_block`] about each one. Nothing here touches WebView2.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use url::Url;

/// Third parties whose only job is to watch or to sell (Shield.swift
/// `unwanted`). Sorted, so a lookup is a binary search; a test keeps it so.
/// Blocked only as third party: a site's own scripts are the site.
const UNWANTED: &[&str] = &[
    "33across.com", "adjust.com", "adnxs.com", "adroll.com", "ads-twitter.com",
    "adservice.google.com", "adsrvr.org", "amazon-adsystem.com", "amplitude.com",
    "analytics.tiktok.com", "analytics.twitter.com", "appsflyer.com", "bidswitch.net",
    "branch.io", "casalemedia.com", "chartbeat.com", "clarity.ms", "connect.facebook.net",
    "criteo.com", "criteo.net", "doubleclick.net", "fullstory.com",
    "google-analytics.com", "googleadservices.com", "googlesyndication.com",
    "googletagmanager.com", "googletagservices.com", "hotjar.com", "indexww.com",
    "mixpanel.com", "moatads.com", "mouseflow.com", "openx.net", "outbrain.com",
    "pubmatic.com", "quantserve.com", "rubiconproject.com", "scorecardresearch.com",
    "segment.com", "segment.io", "sharethrough.com", "smartadserver.com", "taboola.com",
    "teads.tv",
];

/// YouTube's ad reports and ad paths, on YouTube's own hosts, blocked even
/// though they are first party (YouTubeAds.swift `paths`). Prefix matches.
const YOUTUBE_PATHS: &[&str] = &[
    "/api/stats/ads", "/pagead/", "/ptracking", "/get_midroll_info",
    "/youtubei/v1/player/ad_break", "/pcs/activeview",
];

/// The player's quality reports for an ad, never the video's own
/// (YouTubeAds.swift `rules`: `/api/stats/qoe?...adformat=`).
const QOE_PATH: &str = "/api/stats/qoe";

/// Google's own ad paths and the video ads SDK, only when the page is
/// YouTube's: other sites' players may need the SDK to play at all
/// (YouTubeAds.swift `rules`, the `if-domain` pair).
const YOUTUBE_PAGES: &[&str] = &["youtube.com", "youtube-nocookie.com"];

/// The generic slots that are reliably an advertisement and nothing else
/// (Shield.swift `slots`). Kept short on purpose: a generous cosmetic list is
/// how a blocker starts eating the page it was meant to clean.
const SLOTS: &[&str] = &[
    ".adsbygoogle", "ins.adsbygoogle", "[id^=\"google_ads_\"]",
    "[id^=\"div-gpt-ad\"]", "[id^=\"taboola-\"]", "#taboola-below-article",
    "iframe[src*=\"doubleclick.net\"]", "iframe[src*=\"googlesyndication\"]",
    "iframe[src*=\"amazon-adsystem\"]",
    ".ad-slot", ".ad-slot-container", ".top-banner-ad-container",
    ".ad-leaderboard", ".ad-billboard", ".ad-giga", ".ad-mpu", ".ad-mrec",
    ".ad-unit", ".adunit", ".adslot", ".dfp-ad", ".gpt-ad", ".w_ad",
];

/// Slots too plain, or too much a site's own, to hide everywhere, hidden only
/// on these sites (Shield.swift `slotsBySite`). A site entry matches that
/// domain and its subdomains, on a label boundary.
const SLOTS_BY_SITE: &[(&[&str], &str)] = &[
    (&["as.com", "elpais.com"], ".ad"),
    (&["theguardian.com"], ".top-fronts-banner-ad-container"),
    (&["independent.co.uk", "the-independent.com"], "#billboard-wrapper"),
    (&["cnn.com"], ".ad-slot-header__wrapper"),
];

/// Multi-label public suffixes worth knowing. Upstream asks the system
/// "is this a suffix" (Registrable.swift's `isSuffix`); there is no Public
/// Suffix List to download here, so this short list stands in. It only
/// decides where "site" ends, and every blocked domain is a plain
/// two-label one, so a miss can only make two unrelated sites look like one
/// when both sit under an unlisted suffix, which no listed tracker does.
const SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "com.au", "net.au", "org.au",
    "edu.au", "gov.au", "co.nz", "co.jp", "ne.jp", "or.jp", "co.kr", "co.in",
    "co.za", "com.br", "net.br", "org.br", "gov.br", "com.mx", "com.ar", "com.tr",
    "com.cn", "com.hk", "com.sg", "com.tw", "com.pt",
    "github.io", "blogspot.com", "herokuapp.com", "pages.dev", "vercel.app",
    "netlify.app",
];

/// The WebView2 URI patterns to register with `AddWebResourceRequestedFilter`
/// (resource context: all).
///
/// Wildcard semantics relied on: `*` matches any run of characters
/// (including none, and including `/`), `?` matches exactly one character,
/// and the pattern must match the whole URI. So `*://*.d/*` is a subdomain of
/// `d` and `*://d/*` is `d` itself. Two limits, both fine because the caller
/// re-checks with [`Shield::should_block`]: `*` can also match inside a path
/// (`http://x.test/a.d/`), which only costs a callback; and a URL with an
/// explicit port (`https://d:8443/`) is not matched by `*://d/*`, so it
/// slips through unblocked (ad servers on odd ports are not worth a filter
/// each). Patterns never contain a literal `?`, since it is a wildcard.
pub fn filters() -> Vec<String> {
    let mut out = Vec::with_capacity(UNWANTED.len() * 2 + 20);
    for d in UNWANTED {
        out.push(format!("*://*.{d}/*"));
        out.push(format!("*://{d}/*"));
    }
    for p in YOUTUBE_PATHS.iter().chain([&QOE_PATH]) {
        out.push(format!("*://*.youtube.com{p}*"));
        out.push(format!("*://youtube.com{p}*"));
    }
    out.push("*://*.google.com/pagead/*".into());
    out.push("*://google.com/pagead/*".into());
    out.push("*://imasdk.googleapis.com/*".into());
    out
}

/// The registrable domain of a host, as Registrable.swift: an address is its
/// own domain, and so is a name that is itself a suffix. Takes a lowercase,
/// dot-trimmed host and returns a slice of it.
fn registrable(host: &str) -> &str {
    let numeric = host.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    if host.contains(':') || numeric || !host.contains('.') || SUFFIXES.contains(&host) {
        return host;
    }
    // Longest tail first: the first tail that is a suffix means the label
    // before it completes the domain.
    let mut tail = host;
    while let Some(i) = tail.find('.') {
        let rest = &tail[i + 1..];
        if SUFFIXES.contains(&rest) {
            return tail;
        }
        tail = rest;
    }
    // No known multi-label suffix: the last two labels.
    match host[..host.rfind('.').unwrap_or(0)].rfind('.') {
        Some(i) => &host[i + 1..],
        None => host,
    }
}

/// Whether two hosts are one site (Registrable.swift `sameSite`): the same
/// host, or the same registrable domain. Inputs are lowercase.
fn same_site(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let left = registrable(a);
    left.contains('.') && left == registrable(b)
}

/// `host` is `domain` or a subdomain of it, on a label boundary, so
/// `notdoubleclick.net` is not `doubleclick.net`.
fn within(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain)
        .is_some_and(|head| head.is_empty() || head.ends_with('.'))
}

/// Lowercase, trailing dot removed, leading `www.` dropped: the form a site
/// is paused under (upstream pauses by the tab's host without www).
fn site(host: &str) -> String {
    let h = host.trim().trim_matches('.').to_ascii_lowercase();
    h.strip_prefix("www.").map(str::to_owned).unwrap_or(h)
}

/// Whether a host falls under a listed domain: walk the labels, longest
/// first, and look each tail up in the sorted list.
fn unwanted(host: &str) -> bool {
    let mut tail = host;
    loop {
        if UNWANTED.binary_search(&tail).is_ok() {
            return true;
        }
        match tail.find('.') {
            Some(i) => tail = &tail[i + 1..],
            None => return false,
        }
    }
}

fn host_of(u: &Url) -> Option<String> {
    u.host_str().map(|h| h.trim_matches('.').to_ascii_lowercase())
}

/// Sites Shield is off for: the ones it broke. A checkout that never
/// finishes beats switching off everywhere and forgetting to switch back.
#[derive(Debug, Default, Clone)]
pub struct Shield {
    paused: HashSet<String>,
}

impl Shield {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paused sites from a JSON array of hosts. Missing or corrupt means
    /// nothing is paused: failing open for a lost file is fine, refusing to
    /// start is not.
    pub fn load(path: &Path) -> Self {
        let paused = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .map(|v| v.iter().map(|h| site(h)).collect())
            .unwrap_or_default();
        Self { paused }
    }

    /// Written to a sibling file and renamed over, so a crash leaves the old
    /// list or the new one, never half of one. Sorted, as upstream.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut hosts: Vec<&String> = self.paused.iter().collect();
        hosts.sort();
        let json = serde_json::to_vec(&hosts).map_err(io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    pub fn is_paused(&self, host: &str) -> bool {
        !self.paused.is_empty() && self.paused.contains(&site(host))
    }

    pub fn pause(&mut self, host: &str) {
        self.paused.insert(site(host));
    }

    pub fn resume(&mut self, host: &str) {
        self.paused.remove(&site(host));
    }

    /// Whether to cancel this request. `page_url` is the page (main frame)
    /// the request belongs to; `None` means unknown, and an unknown page is
    /// treated as a different site, so the third-party rules apply.
    ///
    /// Rules: a listed tracker domain (or subdomain) that is not the page's
    /// own site; YouTube's ad paths on YouTube's hosts, first party or not;
    /// Google's `/pagead/` and the IMA SDK only when the page is YouTube's.
    /// Never when Shield is paused for the page's host.
    pub fn should_block(&self, request_url: &str, page_url: Option<&str>) -> bool {
        let Ok(req) = Url::parse(request_url) else { return false };
        if !matches!(req.scheme(), "http" | "https") {
            return false;
        }
        let Some(host) = host_of(&req) else { return false };
        let page_host = page_url.and_then(|p| Url::parse(p).ok()).and_then(|u| host_of(&u));
        if page_host.as_deref().is_some_and(|p| self.is_paused(p)) {
            return false;
        }
        let third = page_host.as_deref().is_none_or(|p| !same_site(&host, p));
        if third && unwanted(&host) {
            return true;
        }
        let path = req.path();
        if within(&host, "youtube.com") {
            if YOUTUBE_PATHS.iter().any(|p| path.starts_with(p)) {
                return true;
            }
            // Only the ad flavour of the quality report.
            return path.starts_with(QOE_PATH)
                && req.query().is_some_and(|q| q.contains("adformat="));
        }
        let on_youtube = page_host
            .as_deref()
            .is_some_and(|p| YOUTUBE_PAGES.iter().any(|y| within(p, y)));
        on_youtube
            && ((within(&host, "google.com") && path.starts_with("/pagead/"))
                || host == "imasdk.googleapis.com")
    }
}

/// The stylesheet for a page on `page_host`: the generic slots, then the
/// per-site ones that apply, each hidden with `display:none !important`.
/// Meant to be injected at document start so a slot is never painted.
pub fn cosmetic_css(page_host: &str) -> String {
    const HIDE: &str = "{display:none !important}";
    let host = page_host.trim().trim_matches('.').to_ascii_lowercase();
    let mut css = SLOTS.join(",");
    css.push_str(HIDE);
    for (sites, selector) in SLOTS_BY_SITE {
        if sites.iter().any(|s| within(&host, s)) {
            css.push_str(selector);
            css.push_str(HIDE);
        }
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: Option<&str> = Some("https://example.com/article");

    fn block(url: &str, page: Option<&str>) -> bool {
        Shield::new().should_block(url, page)
    }

    #[test]
    fn list_is_sorted_for_binary_search() {
        assert!(UNWANTED.windows(2).all(|w| w[0] < w[1]), "UNWANTED must be sorted and unique");
        assert_eq!(UNWANTED.len(), 44); // Shield.swift `unwanted`, none added
    }

    #[test]
    fn third_party_tracker_is_blocked() {
        assert!(block("https://doubleclick.net/ad.js", PAGE));
        assert!(block("https://www.google-analytics.com/analytics.js", PAGE));
        assert!(block("http://connect.facebook.net/en_US/sdk.js", PAGE));
    }

    #[test]
    fn subdomains_are_blocked() {
        assert!(block("https://stats.g.doubleclick.net/x", PAGE));
        assert!(block("https://pagead2.googlesyndication.com/a", PAGE));
    }

    #[test]
    fn look_alikes_are_not_blocked() {
        assert!(!block("https://notdoubleclick.net/x", PAGE));
        assert!(!block("https://doubleclick.net.evil.com/x", PAGE));
        assert!(!block("https://segment.community/x", PAGE));
        assert!(!block("https://evil.com/doubleclick.net/x", PAGE));
    }

    #[test]
    fn first_party_is_untouched() {
        // The page is the tracker's own site (a site's own scripts are the site).
        assert!(!block("https://www.criteo.com/s.js", Some("https://criteo.com/")));
        assert!(!block("https://script.hotjar.com/x.js", Some("https://www.hotjar.com/")));
        // Registrable.swift: same registrable domain counts as one site.
        assert!(!block("https://adservice.google.com/x", Some("https://www.google.com/")));
    }

    #[test]
    fn unrelated_requests_pass() {
        assert!(!block("https://example.com/app.js", PAGE));
        assert!(!block("https://cdn.example.net/a.css", PAGE));
        assert!(!block("not a url", PAGE));
        assert!(!block("ftp://doubleclick.net/x", PAGE));
        assert!(!block("data:text/plain,hi", PAGE));
    }

    #[test]
    fn unknown_page_counts_as_third_party() {
        assert!(block("https://doubleclick.net/x", None));
        assert!(block("https://doubleclick.net/x", Some("garbage")));
    }

    #[test]
    fn address_hosts_are_never_one_site() {
        assert!(!same_site("10.0.0.1", "10.0.0.2"));
        assert!(same_site("10.0.0.1", "10.0.0.1"));
        assert!(!same_site("[::1]", "[::2]"));
    }

    #[test]
    fn registrable_domains() {
        assert_eq!(registrable("idmsa.apple.com"), "apple.com");
        assert_eq!(registrable("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(registrable("co.uk"), "co.uk");
        assert_eq!(registrable("localhost"), "localhost");
        assert!(same_site("idmsa.apple.com", "account.apple.com"));
        assert!(!same_site("a.co.uk", "b.co.uk"));
        assert!(!same_site("apple.com", "apple.net"));
    }

    #[test]
    fn paused_site_is_not_blocked() {
        let mut s = Shield::new();
        assert!(s.should_block("https://doubleclick.net/x", PAGE));
        s.pause("www.Example.com");
        assert!(s.is_paused("example.com"));
        assert!(s.is_paused("WWW.EXAMPLE.COM."));
        assert!(!s.should_block("https://doubleclick.net/x", PAGE));
        // Another site is still protected.
        assert!(s.should_block("https://doubleclick.net/x", Some("https://other.org/")));
        s.resume("example.com");
        assert!(!s.is_paused("example.com"));
        assert!(s.should_block("https://doubleclick.net/x", PAGE));
    }

    #[test]
    fn youtube_paths_block_even_first_party() {
        let yt = Some("https://www.youtube.com/watch?v=1");
        // YouTubeAds.swift `paths`
        for u in [
            "https://www.youtube.com/api/stats/ads?x=1",
            "https://www.youtube.com/pagead/viewthroughconversion",
            "https://www.youtube.com/ptracking?a=b",
            "https://youtube.com/get_midroll_info",
            "https://www.youtube.com/youtubei/v1/player/ad_break",
            "https://m.youtube.com/pcs/activeview",
        ] {
            assert!(block(u, yt), "{u}");
        }
        // The video itself is never touched.
        assert!(!block("https://www.youtube.com/youtubei/v1/player?key=1", yt));
        assert!(!block("https://www.youtube.com/watch?v=1", yt));
        assert!(!block("https://rr1.googlevideo.com/videoplayback", yt));
        // Look-alike host.
        assert!(!block("https://notyoutube.com/ptracking", yt));
    }

    #[test]
    fn youtube_qoe_only_for_ads() {
        let yt = Some("https://www.youtube.com/");
        assert!(block("https://www.youtube.com/api/stats/qoe?cpn=1&adformat=15_2", yt));
        assert!(!block("https://www.youtube.com/api/stats/qoe?cpn=1&fmt=18", yt));
    }

    #[test]
    fn google_ad_paths_only_from_youtube_pages() {
        let yt = Some("https://www.youtube.com/");
        let nocookie = Some("https://www.youtube-nocookie.com/embed/x");
        let other = Some("https://example.com/");
        assert!(block("https://www.google.com/pagead/x", yt));
        assert!(block("https://www.google.com/pagead/x", nocookie));
        assert!(block("https://imasdk.googleapis.com/js/sdkloader/ima3.js", yt));
        assert!(!block("https://www.google.com/pagead/x", other));
        assert!(!block("https://imasdk.googleapis.com/js/sdkloader/ima3.js", other));
        assert!(!block("https://www.google.com/search?q=1", yt));
        // A page that only ends in the same letters is not YouTube.
        assert!(!block("https://imasdk.googleapis.com/x", Some("https://notyoutube.com/")));
    }

    #[test]
    fn youtube_rules_respect_pause() {
        let mut s = Shield::new();
        s.pause("youtube.com");
        assert!(!s.should_block("https://www.youtube.com/ptracking", Some("https://www.youtube.com/")));
    }

    #[test]
    fn filters_are_wildcard_patterns() {
        let f = filters();
        assert!(f.contains(&"*://*.doubleclick.net/*".to_string()));
        assert!(f.contains(&"*://doubleclick.net/*".to_string()));
        assert!(f.contains(&"*://*.youtube.com/ptracking*".to_string()));
        assert!(f.contains(&"*://imasdk.googleapis.com/*".to_string()));
        assert!(f.iter().all(|p| !p.contains('?') && p.starts_with("*://")));
        assert_eq!(f.len(), UNWANTED.len() * 2 + (YOUTUBE_PATHS.len() + 1) * 2 + 3);
    }

    #[test]
    fn css_generic_slots() {
        let css = cosmetic_css("example.com");
        assert!(css.starts_with(".adsbygoogle,ins.adsbygoogle,"));
        assert!(css.contains("[id^=\"div-gpt-ad\"]"));
        assert!(css.ends_with(".w_ad{display:none !important}"));
        assert_eq!(css.matches("!important").count(), 1);
        // A bare `.ad` is too plain to hide everywhere.
        assert!(!css.contains(",.ad,") && !css.contains("}.ad{"));
    }

    #[test]
    fn css_per_site() {
        // Shield.swift `slotsBySite`
        assert!(cosmetic_css("www.as.com").ends_with("}.ad{display:none !important}"));
        assert!(cosmetic_css("elpais.com").ends_with("}.ad{display:none !important}"));
        assert!(cosmetic_css("www.theguardian.com").contains(".top-fronts-banner-ad-container{"));
        assert!(cosmetic_css("independent.co.uk").contains("#billboard-wrapper{"));
        assert!(cosmetic_css("the-independent.com").contains("#billboard-wrapper{"));
        assert!(cosmetic_css("edition.cnn.com").contains(".ad-slot-header__wrapper{"));
        // Boundary: "alias.com" ends in "as.com" but is not as.com.
        assert!(!cosmetic_css("alias.com").contains("}.ad{"));
        assert!(!cosmetic_css("example.com").contains("billboard-wrapper"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("shield-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("paused.json");
        let mut s = Shield::new();
        s.pause("b.com");
        s.pause("www.a.com");
        s.save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"["a.com","b.com"]"#);
        assert!(!dir.join("paused.json.tmp").exists());
        let t = Shield::load(&path);
        assert!(t.is_paused("a.com") && t.is_paused("b.com") && !t.is_paused("c.com"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_corrupt_file_pauses_nothing() {
        let dir = std::env::temp_dir().join(format!("shield-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!Shield::load(&dir.join("nope.json")).is_paused("a.com"));
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert!(!Shield::load(&bad).is_paused("a.com"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
