use super::*;

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

fn kept(store: &mut MemoryStore, host: &str, user: &str, pass: &str, used: Option<u64>, clear: bool) {
    let mut a = Account::new(host, user, Secret::new(pass));
    a.used = used;
    a.clear = clear;
    store.save(&a).unwrap();
}

fn field<'a>(page: &'a Url, frame: Option<&'a Url>) -> Field<'a> {
    Field { page, frame, given_name: None }
}

fn users(o: &Offered) -> Vec<&str> {
    o.accounts.iter().map(|k| k.user.as_str()).collect()
}

#[test]
fn secret_never_printed() {
    let a = Account::new("example.com", "ana", Secret::new("hunter2"));
    let shown = format!("{a:?} {:?}", a.password);
    assert!(!shown.contains("hunter2"));
    assert_eq!(a.password.expose(), "hunter2");
    assert_eq!(Secret::new("a"), Secret::new("a"));
    assert_ne!(Secret::new("a"), Secret::new("ab"));
}

#[test]
fn sites() {
    assert_eq!(registrable("accounts.example.com"), "example.com");
    assert_eq!(registrable("www.bbc.co.uk"), "bbc.co.uk");
    assert_eq!(registrable("ana.github.io"), "ana.github.io");
    assert_eq!(registrable("192.168.0.1"), "192.168.0.1");
    assert!(same_site("idmsa.apple.com", "account.apple.com"));
    assert!(!same_site("ana.github.io", "bob.github.io"));
    assert!(!same_site("10.0.0.1", "10.0.0.2"));
    assert!(!same_site("localhost", "other"));
    assert_eq!(host_of(" www.Example.com/login "), "example.com");
    assert_eq!(host_of("http://Shop.example.com:8080/x"), "shop.example.com:8080");
    assert_eq!(host_of("android://hash@com.vendor.app/"), "");
    assert_eq!(host_of("ftp://example.com"), "");
    assert_eq!(page_host(&url("file:///C:/x.html")), None);
}

#[test]
fn store_saves_updates_and_forgets() {
    let mut s = MemoryStore::new();
    assert!(s.save(&Account::new("", "a", Secret::new("p"))).is_err());
    assert!(s.save(&Account::new("example.com", "a", Secret::new(""))).is_err());
    kept(&mut s, "example.com", "ana", "one", Some(5), false);
    let first = s.account("example.com", "ana").unwrap().unwrap();
    // An update with no `used` keeps the one kept; `created` is the first's.
    kept(&mut s, "example.com", "ana", "two", None, false);
    let second = s.account("example.com", "ana").unwrap().unwrap();
    assert_eq!(second.password, Secret::new("two"));
    assert_eq!(second.used, Some(5));
    assert_eq!(second.created, first.created);
    assert_eq!(s.all().unwrap().len(), 1);
    // No user at all is still an account (CHANGELOG: password-only sign-ins).
    kept(&mut s, "example.com", "", "pin", None, false);
    assert_eq!(s.kept_for("example.com").unwrap().len(), 2);
    assert!(s.delete("example.com", "ana").unwrap());
    assert!(!s.delete("example.com", "ana").unwrap());
    // An http sign-in never makes an https account plain.
    assert!(touch(&mut s, "example.com", "", true, 99).unwrap());
    let t = s.account("example.com", "").unwrap().unwrap();
    assert!(!t.clear && t.used == Some(99));
    // An https one makes an http-kept account https-only.
    kept(&mut s, "plain.test", "", "pin", None, true);
    assert!(touch(&mut s, "plain.test", "", false, 100).unwrap());
    assert!(!s.account("plain.test", "").unwrap().unwrap().clear);
}

#[test]
fn offers_exact_host_alone_newest_first() {
    let mut s = MemoryStore::new();
    kept(&mut s, "example.com", "old", "p", Some(1), false);
    kept(&mut s, "example.com", "new", "p", Some(9), false);
    kept(&mut s, "accounts.example.com", "acc", "p", Some(5), false);
    kept(&mut s, "other.com", "x", "p", Some(50), false);
    kept(&mut s, "notexample.com", "y", "p", Some(50), false);
    let page = url("https://accounts.example.com/signin");
    let o = offers(&s, &field(&page, None)).unwrap().unwrap();
    assert_eq!(users(&o), ["acc"]);
    assert_eq!(o.host, "accounts.example.com");
    // www. is the bare host.
    let o = offers(&s, &field(&url("https://www.example.com/"), None)).unwrap().unwrap();
    assert_eq!(users(&o), ["new", "old"]);
    // A suffix missing from the list (two owners under one name) gives nothing.
    kept(&mut s, "ana.unknown-host.example", "ana", "p", None, false);
    let o = offers(&s, &field(&url("https://bob.unknown-host.example/"), None)).unwrap().unwrap();
    assert!(o.accounts.is_empty());
}

#[test]
fn another_port_is_another_server() {
    let mut s = MemoryStore::new();
    kept(&mut s, "example.com", "plainport", "p", None, false);
    kept(&mut s, "example.com:8443", "dev", "p", None, false);
    // An account kept without a port is the scheme's own port's.
    for page in ["https://example.com/", "https://example.com:443/"] {
        assert_eq!(users(&offers(&s, &field(&url(page), None)).unwrap().unwrap()), ["plainport"]);
    }
    let o = offers(&s, &field(&url("https://www.example.com:8443/"), None)).unwrap().unwrap();
    assert_eq!((users(&o), o.host.as_str()), (vec!["dev"], "example.com:8443"));
    assert!(offers(&s, &field(&url("https://example.com:9999/"), None)).unwrap().unwrap().accounts.is_empty());
    assert!(o.may_fill(&url("https://example.com:8443/next")));
    assert!(!o.may_fill(&url("https://example.com/")));
    assert_eq!(page_host(&url("http://[::1]:8080/")).as_deref(), Some("[::1]:8080"));
}

#[test]
fn http_page_gets_only_http_accounts() {
    let mut s = MemoryStore::new();
    kept(&mut s, "example.com", "secure", "p", None, false);
    kept(&mut s, "example.com", "plain", "p", None, true);
    let o = offers(&s, &field(&url("http://example.com/"), None)).unwrap().unwrap();
    assert_eq!(users(&o), ["plain"]);
    assert!(o.clear);
    // An https page is offered both: the http one was kept by the same site.
    let o = offers(&s, &field(&url("https://example.com/"), None)).unwrap().unwrap();
    assert_eq!(o.accounts.len(), 2);
}

#[test]
fn never_in_a_frame_from_another_site_nor_off_the_web() {
    let mut s = MemoryStore::new();
    kept(&mut s, "example.com", "ana", "p", None, false);
    let page = url("https://example.com/");
    let evil = url("https://evil.test/frame");
    assert_eq!(offers(&s, &field(&page, Some(&evil))).unwrap(), None);
    let data = url("data:text/html,x");
    assert_eq!(offers(&s, &field(&page, Some(&data))).unwrap(), None);
    // A frame gets only what its own origin would: the page's, if it is the
    // page's origin; nothing on another host, port or scheme of the site.
    let own = url("https://www.example.com/frame");
    assert_eq!(offers(&s, &field(&page, Some(&own))).unwrap().unwrap().accounts.len(), 1);
    for other in ["https://login.example.com/frame", "https://example.com:8443/frame", "http://example.com/frame"] {
        assert_eq!(offers(&s, &field(&page, Some(&url(other)))).unwrap(), None, "{other}");
    }
    for other in ["file:///C:/x.html", "searchwin://settings", "chrome-extension://abc/p.html"] {
        assert_eq!(offers(&s, &field(&url(other), None)).unwrap(), None);
    }
}

#[test]
fn five_at_most_given_name_first() {
    let mut s = MemoryStore::new();
    for i in 0..8u64 {
        kept(&mut s, "example.com", &format!("u{i}"), "p", Some(i), false);
    }
    let page = url("https://example.com/");
    let o = offers(&s, &Field { page: &page, frame: None, given_name: Some("U0") }).unwrap().unwrap();
    assert_eq!(users(&o), ["u0", "u7", "u6", "u5", "u4"]);
}

#[test]
fn a_pick_fills_only_the_page_it_was_offered_on() {
    let o = Offered { host: "example.com".into(), clear: false, accounts: vec![] };
    assert!(o.may_fill(&url("https://www.example.com/next")));
    assert!(!o.may_fill(&url("http://example.com/")));
    assert!(!o.may_fill(&url("https://evil.test/")));
    assert!(!o.may_fill(&url("https://login.example.com/")));
}

#[test]
fn given_name_same_host_and_recent() {
    let first = url("https://accounts.google.com/step1");
    let g = GivenName::new(&first, "ana@x.com", 1_000).unwrap();
    assert_eq!(g.for_page(&url("https://accounts.google.com/pw"), 2_000), Some("ana@x.com"));
    // Another host of the site, another port, another site: not theirs.
    for other in ["https://myaccount.google.com/pw", "https://accounts.google.com:8443/pw", "https://evil.test/pw"] {
        assert_eq!(g.for_page(&url(other), 2_000), None, "{other}");
    }
    assert_eq!(g.for_page(&first, 1_000 + NAME_FOR_MS), None);
    assert!(GivenName::new(&first, "", 0).is_none());
    assert!(GivenName::new(&first, &"x".repeat(257), 0).is_none());
}

#[test]
fn sent_and_settled() {
    let page = url("http://www.example.com/login");
    let sent = Sent::new(&page, "", Secret::new("p"), Some("ana"), 0).unwrap();
    assert_eq!((sent.host.as_str(), sent.user.as_str(), sent.clear), ("example.com", "ana", true));
    assert!(Sent::new(&url("chrome-extension://x/unlock.html"), "", Secret::new("p"), None, 0).is_none());
    assert_eq!(settle(&sent, 1_000, true, false), Settle::Worked);
    assert_eq!(settle(&sent, 1_000, true, true), Settle::Keep);
    assert_eq!(settle(&sent, 1_000, false, false), Settle::Later);
    assert_eq!(settle(&sent, SENT_FOR_MS, true, false), Settle::Drop);
}

#[test]
fn save_offer_rules() {
    let mut s = MemoryStore::new();
    let page = url("https://example.com/login");
    let sent = Sent::new(&page, "ana", Secret::new("pw"), None, 0).unwrap();
    let never: Vec<String> = vec![];
    let ctx = SaveContext { saves_passwords: true, private_tab: false, extension_page: false, never: &never };
    assert_eq!(should_offer_save(&s, &sent, &ctx).unwrap(), SaveOffer::Offer { changed: false });
    for off in [
        SaveContext { saves_passwords: false, ..ctx },
        SaveContext { private_tab: true, ..ctx },
        SaveContext { extension_page: true, ..ctx },
    ] {
        assert_eq!(should_offer_save(&s, &sent, &off).unwrap(), SaveOffer::Nothing);
    }
    let never = vec!["example.com".to_owned()];
    let sub = Sent::new(&url("https://login.example.com/"), "ana", Secret::new("pw"), None, 0).unwrap();
    assert_eq!(should_offer_save(&s, &sub, &SaveContext { never: &never, ..ctx }).unwrap(), SaveOffer::Nothing);

    keep_sent(&mut s, &sent, 7).unwrap();
    assert_eq!(s.account("example.com", "ana").unwrap().unwrap().used, Some(7));
    assert_eq!(should_offer_save(&s, &sent, &ctx).unwrap(), SaveOffer::Touch);
    let changed = Sent::new(&page, "ana", Secret::new("new"), None, 0).unwrap();
    assert_eq!(should_offer_save(&s, &changed, &ctx).unwrap(), SaveOffer::Offer { changed: true });
    // The same account sent from plain http: neither touched nor updated.
    let plain = url("http://example.com/login");
    for pw in ["pw", "new"] {
        let sent = Sent::new(&plain, "ana", Secret::new(pw), None, 0).unwrap();
        assert_eq!(should_offer_save(&s, &sent, &ctx).unwrap(), SaveOffer::Nothing);
    }
    assert!(!s.account("example.com", "ana").unwrap().unwrap().clear);
}

const CHROME: &str = "name,url,username,password,note\r\n\
example.com,https://www.example.com/login,ana,\"p,w\"\"1\",hi\r\n\
plain,http://plain.test/,bob,pw2,\r\n\
app,android://h@com.vendor.app/,x,pw3,\r\n\
nopass,https://a.test/,x,,\r\n\
\r\n\
dup,https://example.com/other,ana,last,\r\n";

#[test]
fn csv_chrome_with_quotes_dupes_and_skips() {
    let p = parse_csv(CHROME);
    assert_eq!(p.skipped, 2);
    assert_eq!(p.accounts.len(), 2);
    let ana = p.accounts.iter().find(|a| a.user == "ana").unwrap();
    assert_eq!((ana.host.as_str(), ana.password.expose(), ana.clear), ("example.com", "last", false));
    let bob = p.accounts.iter().find(|a| a.user == "bob").unwrap();
    assert!(bob.clear && bob.host == "plain.test");
}

#[test]
fn csv_other_browsers() {
    let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\n\
\"https://example.com\",\"ana\",\"pw\",,\"https://example.com\",\"{x}\",\"1\",\"2\",\"3\"\n";
    let bitwarden = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
,,login,Ex,\"line1\nline2\",,0,\"https://example.com/a,https://other.test\",ana,pw,\n\
,,note,Secret note,text,,0,,,,\n";
    let safari = "Title,URL,Username,Password,Notes,OTPAuth\nEx,https://example.com/,ana,pw,n,\n";
    for (text, skipped) in [(firefox, 0), (bitwarden, 1), (safari, 0)] {
        let p = parse_csv(text);
        assert_eq!(p.skipped, skipped);
        assert_eq!(p.accounts.len(), 1);
        assert_eq!((p.accounts[0].host.as_str(), p.accounts[0].user.as_str()), ("example.com", "ana"));
        assert_eq!(p.accounts[0].password.expose(), "pw");
    }
    assert_eq!(parse_csv(bitwarden).accounts[0].note, "line1\nline2");
    // A header with no password column: every row skipped, nothing kept.
    let p = parse_csv("a,b\n1,2\n3,4\n");
    assert_eq!((p.accounts.len(), p.skipped), (0, 2));
    assert_eq!(parse_csv("").skipped, 0);
}

#[test]
fn csv_import_twice_adds_nothing_twice_and_export_round_trips() {
    let mut s = MemoryStore::new();
    assert_eq!(take_csv(&mut s, CHROME), (2, 2));
    assert_eq!(take_csv(&mut s, CHROME), (2, 2));
    assert_eq!(s.all().unwrap().len(), 2);
    kept(&mut s, "example.com", "ana", "p,w\"1\n", None, false);

    let out = export_csv(&s).unwrap();
    assert!(out.expose().starts_with("name,url,username,password,note\n"));
    assert!(out.expose().contains("http://plain.test/"));
    let back = parse_csv(out.expose());
    assert_eq!(back.skipped, 0);
    assert_eq!(back.accounts.len(), 2);
    let ana = back.accounts.iter().find(|a| a.user == "ana").unwrap();
    assert_eq!(ana.password.expose(), "p,w\"1\n");
    assert!(back.accounts.iter().any(|a| a.host == "plain.test" && a.clear));
}

#[test]
fn csv_export_defuses_formulas_and_round_trips() {
    let mut s = MemoryStore::new();
    let mut a = Account::new("example.com:8080", "=HYPERLINK(\"x\")", Secret::new("=1+1"));
    a.note = "@SUM(A1)".into();
    s.save(&a).unwrap();
    kept(&mut s, "example.com", "-ana", "p", None, false);
    kept(&mut s, "example.com", "'quoted", "p", None, false);
    let out = export_csv(&s).unwrap();
    let text = out.expose();
    assert!(text.contains("\"'=HYPERLINK(\"\"x\"\")\""), "{text}");
    assert!(text.contains("\"'@SUM(A1)\"") && text.contains("\"'-ana\""), "{text}");
    // The password is never touched, and no line starts a formula.
    assert!(text.contains(",=1+1,"));
    assert!(text.lines().all(|l| !l.starts_with(['=', '+', '-', '@'])));
    let back = parse_csv(text);
    let users: Vec<&str> = back.accounts.iter().map(|a| a.user.as_str()).collect();
    assert_eq!(users, ["'quoted", "-ana", "=HYPERLINK(\"x\")"]);
    let f = back.accounts.iter().find(|a| a.host == "example.com:8080").unwrap();
    assert_eq!((f.password.expose(), f.note.as_str()), ("=1+1", "@SUM(A1)"));
}

#[test]
fn csv_ticks_round_trip_exactly_and_semicolons_are_quoted() {
    let users = ["'=abc", "''+x", "a;=cmd", "a;b", "'plain", "ok", "=x"];
    let mut s = MemoryStore::new();
    for u in users {
        kept(&mut s, "example.com", u, "p;=1", None, false);
    }
    let out = export_csv(&s).unwrap();
    let text = out.expose();
    assert!(text.contains("\"''=abc\"") && text.contains("\"'a;=cmd\"") && text.contains("\"a;b\""), "{text}");
    // The password is quoted for its `;`, never ticked.
    assert!(text.contains(",\"p;=1\","), "{text}");
    let back = parse_csv(text);
    let mut got: Vec<&str> = back.accounts.iter().map(|a| a.user.as_str()).collect();
    let mut want = users.to_vec();
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert!(back.accounts.iter().all(|a| a.password.expose() == "p;=1"));
}
