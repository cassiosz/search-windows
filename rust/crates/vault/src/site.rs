//! The site behind a host (Registrable.swift, Vault.swift:271..290). The
//! logic is crates/shield's, copied rather than depended on; the suffix list
//! is longer here, because for passwords a missing suffix is not a missed ad
//! but two owners' sites taken for one.

use url::Url;

/// Multi-label public suffixes. Upstream asks the system's Public Suffix List
/// (Passkeys.publicSuffix); there is none to ask on Windows, so this list
/// stands in. A suffix missing here makes two hosts under it one "site" — which
/// only groups the never-ask list and a first step's name: accounts are offered
/// on their exact host alone (lib.rs `offers`).
const SUFFIXES: &[&str] = &[
    // country second levels
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "net.uk", "sch.uk", "nhs.uk",
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "asn.au", "id.au",
    "co.nz", "org.nz", "net.nz", "ac.nz", "govt.nz",
    "co.jp", "ne.jp", "or.jp", "ac.jp", "go.jp", "gr.jp",
    "co.kr", "or.kr", "go.kr", "ac.kr",
    "co.in", "net.in", "org.in", "gov.in", "ac.in", "firm.in",
    "co.za", "org.za", "gov.za", "ac.za",
    "com.br", "net.br", "org.br", "gov.br", "edu.br", "art.br", "blog.br",
    "com.mx", "org.mx", "gob.mx", "edu.mx",
    "com.ar", "gob.ar", "org.ar", "com.co", "gov.co", "com.pe", "gob.pe", "com.uy", "com.ve", "cl.cl",
    "com.tr", "gov.tr", "edu.tr", "org.tr",
    "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn",
    "com.hk", "org.hk", "gov.hk", "edu.hk",
    "com.sg", "gov.sg", "edu.sg", "com.tw", "org.tw", "gov.tw", "edu.tw",
    "com.my", "gov.my", "com.ph", "gov.ph", "com.vn", "gov.vn", "co.th", "go.th", "ac.th",
    "co.id", "or.id", "go.id", "ac.id", "co.il", "org.il", "gov.il", "ac.il",
    "com.pt", "gov.pt", "com.es", "gob.es", "org.es", "com.pl", "net.pl", "gov.pl", "com.ua", "gov.ua",
    "com.ru", "com.eg", "gov.eg", "com.sa", "gov.sa", "com.pk", "gov.pk", "com.ng", "gov.ng", "co.ke",
    // hosting where each name is someone else's
    "github.io", "gitlab.io", "bitbucket.io", "codeberg.page", "blogspot.com", "wordpress.com",
    "herokuapp.com", "pages.dev", "workers.dev", "vercel.app", "netlify.app", "web.app",
    "firebaseapp.com", "appspot.com", "azurewebsites.net", "azurestaticapps.net", "cloudfront.net",
    "s3.amazonaws.com", "onrender.com", "fly.dev", "railway.app", "deno.dev", "glitch.me",
    "repl.co", "surge.sh", "myshopify.com", "wixsite.com", "neocities.org", "ngrok.io",
    "ngrok-free.app", "duckdns.org", "no-ip.org", "dyndns.org", "translate.goog",
];

/// example.com for www.example.com and accounts.example.com; bbc.co.uk stays
/// bbc.co.uk (Registrable.swift `domain`). An address is its own domain, and
/// so is a name that is itself a suffix. Takes a lowercase host.
pub fn registrable(host: &str) -> &str {
    let host = host.trim_matches('.');
    let numeric = host.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    if host.contains(':') || host.starts_with('[') || numeric || !host.contains('.') || SUFFIXES.contains(&host) {
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
/// host, or the same registrable domain — never for an address, nor when the
/// domain couldn't be worked out. Lowercase inputs.
pub fn same_site(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let left = registrable(a);
    // An address is its own domain, so two differing ones never match here.
    left.contains('.') && left == registrable(b)
}

/// The host an account is kept under, from whatever an export or a person
/// typed (Vault.swift:279 `host(of:)`): `https://` assumed, http(s) only —
/// an Android app's login (android://…@com.vendor.app/) names a package that
/// can read as a domain someone else owns — lowercase, without `www.`.
/// Empty when it isn't a website.
pub fn host_of(text: &str) -> String {
    let text = text.trim();
    let owned;
    let text = if text.contains("://") {
        text
    } else {
        owned = format!("https://{text}");
        &owned
    };
    Url::parse(text).ok().and_then(|u| page_host(&u)).unwrap_or_default()
}

/// The host a page's accounts are kept under (Curtain.swift `host(of:)`),
/// for http and https pages only: lowercase, without `www.`, and with the port
/// when it isn't the scheme's own (`example.com:8080`): another port is
/// another server. Kept hosts without one still match default-port pages.
pub fn page_host(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let host = host.strip_prefix("www.").map(str::to_owned).unwrap_or(host);
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}
