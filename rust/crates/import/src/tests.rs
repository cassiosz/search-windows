//! Every test reads made-up browsers in a folder of its own under %TEMP%;
//! no real profile is ever looked at. SQLite fixtures are written through the
//! same winsqlite3 binding the readers use.

use crate::sqlite::Db;
use crate::*;
use std::path::{Path, PathBuf};

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Fixture {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("searchwin-import-test-{}-{name}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("local")).unwrap();
        std::fs::create_dir_all(dir.join("roaming")).unwrap();
        Fixture(dir)
    }

    fn bases(&self) -> Bases {
        Bases { local: self.0.join("local"), roaming: self.0.join("roaming") }
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn db(&self, rel: &str, sql: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        Db::create(&path).unwrap().exec(sql).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const CHROME: &str = r"local\Google\Chrome\User Data";
/// 2024-01-01T00:00:00Z in Unix ms, and in Chromium's µs since 1601.
const JAN_2024_MS: u64 = 1_704_067_200_000;
const JAN_2024_WEBKIT: i64 = (1_704_067_200_000 + 11_644_473_600_000) * 1000;

fn chrome_history(sql_rows: &str) -> String {
    format!(
        "CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, \
         typed_count INTEGER DEFAULT 0, last_visit_time INTEGER, hidden INTEGER DEFAULT 0); {sql_rows}"
    )
}

const BOOKMARKS: &str = r#"{"roots":{
  "bookmark_bar":{"type":"folder","children":[
    {"type":"url","name":"Rust","url":"https://www.rust-lang.org/"},
    {"type":"url","name":"Bad","url":"javascript:alert(1)"},
    {"type":"folder","name":"Docs","children":[{"type":"url","name":"","url":"https://docs.rs/"}]}]},
  "other":{"type":"folder","children":[{"type":"url","name":"Ex","url":"https://example.com/"}]},
  "synced":{"type":"folder","children":[]}}}"#;

fn chrome_fixture() -> Fixture {
    let f = Fixture::new("chrome");
    f.write(
        &format!(r"{CHROME}\Local State"),
        br#"{"profile":{"info_cache":{"Default":{"name":"Personal"},"Profile 1":{"name":"Work"}},"last_used":"Profile 1"}}"#,
    );
    f.write(&format!(r"{CHROME}\Default\Bookmarks"), BOOKMARKS.as_bytes());
    f.db(
        &format!(r"{CHROME}\Default\History"),
        &chrome_history(&format!(
            "INSERT INTO urls(url,title,visit_count,last_visit_time) VALUES \
             ('https://a.com/','A',3,{JAN_2024_WEBKIT}), \
             ('https://b.com/','B',1,{}), \
             ('file:///C:/secret.txt','no',9,{JAN_2024_WEBKIT}), \
             ('https://user:pw@c.com/','C',2,0); \
             INSERT INTO urls(url,title,visit_count,last_visit_time,hidden) VALUES ('https://hidden.com/','H',5,{JAN_2024_WEBKIT},1);",
            JAN_2024_WEBKIT - 1_000_000
        )),
    );
    f.db(
        &format!(r"{CHROME}\Profile 10\History"),
        &chrome_history(&format!(
            "INSERT INTO urls(url,title,visit_count,last_visit_time) VALUES ('https://a.com/','',2,{});",
            JAN_2024_WEBKIT + 5_000_000
        )),
    );
    f.write(&format!(r"{CHROME}\Profile 2\Bookmarks"), b"not json");
    f.write(&format!(r"{CHROME}\Guest Profile\History"), b"");
    f.write(&format!(r"{CHROME}\Crashpad\settings.dat"), b"");
    f
}

#[test]
fn finds_chromium_profiles_with_their_names() {
    let f = chrome_fixture();
    let found = discover(&f.bases());
    assert_eq!(found.len(), 1);
    let chrome = &found[0];
    assert_eq!(chrome.name, "Chrome");
    assert_eq!(chrome.engine, Engine::Chromium);
    let ids: Vec<&str> = chrome.profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["Default", "Profile 2", "Profile 10"]);
    assert_eq!(chrome.profiles[0].name, "Personal");
    assert_eq!(chrome.profiles[1].name, "Profile 2");
    // Local State's last_used names a profile that has nothing to read, so
    // the newest-touched one is used.
    assert!(chrome.usual.is_some());
    assert!(chrome.arc_sidebar.is_none());
}

#[test]
fn nothing_installed_finds_nothing() {
    let f = Fixture::new("empty");
    assert!(discover(&f.bases()).is_empty());
}

#[test]
fn reads_chromium_bookmarks_and_says_when_one_failed() {
    let f = chrome_fixture();
    let chrome = &discover(&f.bases())[0];
    let (nodes, complete) = bookmarks(chrome, Some("Default"));
    assert!(complete);
    let titles: Vec<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Rust", "Docs", "Other bookmarks"]);
    assert_eq!(nodes[1].children.as_ref().unwrap()[0].title, "docs.rs");
    // Profile 2's file is damaged: what read comes, and it says so.
    let (all, complete) = bookmarks(chrome, None);
    assert!(!complete);
    assert_eq!(all.len(), 3);
    // A profile with no Bookmarks file has none and reads cleanly.
    assert_eq!(bookmarks(chrome, Some("Profile 10")), (Vec::new(), true));
    // No such profile: nothing read, not complete.
    assert!(!bookmarks(chrome, Some("Nope")).1);
}

#[test]
fn reads_chromium_history_across_profiles() {
    let f = chrome_fixture();
    let chrome = &discover(&f.bases())[0];
    let one = history(chrome, Some("Default"), HISTORY_LIMIT);
    let urls: Vec<&str> = one.iter().map(|e| e.url.as_str()).collect();
    // No file: or hidden rows; the password dropped; a missing date is now.
    assert_eq!(urls, ["https://c.com/", "https://a.com/", "https://b.com/"]);
    assert_eq!(one[1].last, JAN_2024_MS);
    assert_eq!(one[1].count, 3);
    assert_eq!(one[2].last, JAN_2024_MS - 1000);
    assert!(one[0].last > JAN_2024_MS);

    let all = history(chrome, None, HISTORY_LIMIT);
    let a = all.iter().find(|e| e.url == "https://a.com/").unwrap();
    assert_eq!((a.count, a.last, a.title.as_str()), (5, JAN_2024_MS + 5000, "A"));
    assert_eq!(history(chrome, None, 1).len(), 1);
    // Works with searchwin-history as is.
    assert_eq!(searchwin_history::History::from_entries(all).len(), 3);
}

#[test]
fn reads_what_the_running_browser_left_in_its_wal() {
    let f = Fixture::new("wal");
    let file = f.db(
        &format!(r"{CHROME}\Default\History"),
        &chrome_history("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;"),
    );
    // The "browser" keeps its connection open, so the row lives in -wal.
    let browser = Db::create(&file).unwrap();
    browser.exec("PRAGMA wal_autocheckpoint=0;").unwrap();
    browser
        .exec(&format!("INSERT INTO urls(url,title,visit_count,last_visit_time) VALUES ('https://new.com/','N',1,{JAN_2024_WEBKIT});"))
        .unwrap();
    assert!(Path::new(&format!("{}-wal", file.display())).exists());
    let chrome = &discover(&f.bases())[0];
    let read = history(chrome, None, 10);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].url, "https://new.com/");
    drop(browser);
}

#[test]
fn the_copy_is_deleted_and_the_original_untouched() {
    let f = chrome_fixture();
    let file = f.0.join(format!(r"{CHROME}\Default\History"));
    let before = std::fs::read(&file).unwrap();
    let db = Db::snapshot(&file).unwrap();
    let scratch = db.scratch().to_path_buf();
    assert!(scratch.join("History").exists());
    drop(db);
    assert!(!scratch.exists());
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let dir = file.parent().unwrap();
    let names: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn opera_keeps_its_profile_in_its_own_folder() {
    let f = Fixture::new("opera");
    f.write(r"roaming\Opera Software\Opera Stable\Bookmarks", BOOKMARKS.as_bytes());
    let found = discover(&f.bases());
    assert_eq!(found[0].name, "Opera");
    assert_eq!(found[0].profiles[0].id, "Opera Stable");
    assert_eq!(found[0].profiles[0].name, "Opera");
    assert_eq!(bookmarks(&found[0], None).0.len(), 3);
}

// ---- Firefox ----

fn firefox_fixture() -> Fixture {
    let f = Fixture::new("firefox");
    f.write(
        r"roaming\Mozilla\Firefox\profiles.ini",
        b"[Install308046B0AF4A39CB]\nDefault=Profiles/abc.default-release\n\n[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/xyz.default\nDefault=1\n\n[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n",
    );
    let stamp = JAN_2024_MS * 1000;
    f.db(
        r"roaming\Mozilla\Firefox\Profiles\abc.default-release\places.sqlite",
        &format!(
            "CREATE TABLE moz_places(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, hidden INTEGER DEFAULT 0, last_visit_date INTEGER);
             CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT, guid TEXT);
             INSERT INTO moz_places VALUES (1,'https://mozilla.org/','Mozilla',4,0,{stamp}),(2,'https://news.com/','News',1,0,{}),
               (3,'place:tag=x',NULL,0,0,NULL),(4,'https://unvisited.com/','U',0,0,NULL);
             INSERT INTO moz_bookmarks VALUES
               (1,2,NULL,0,0,'','root________'),
               (2,2,NULL,1,0,'menu','menu________'),
               (3,2,NULL,1,1,'toolbar','toolbar_____'),
               (4,2,NULL,1,2,'tags','tags________'),
               (5,2,NULL,1,3,'unfiled','unfiled_____'),
               (6,2,NULL,1,4,'mobile','mobile______'),
               (10,1,1,3,0,'Mozilla','a'),
               (11,2,NULL,3,1,'Work','b'),
               (12,1,2,11,0,'','c'),
               (13,1,1,2,0,'From menu','d'),
               (14,2,NULL,4,0,'sometag','e'),
               (15,1,3,5,0,'Tag query','f'),
               (16,1,4,5,1,'Later','g');",
            stamp - 1_000_000
        ),
    );
    f
}

#[test]
fn finds_firefox_profiles_from_profiles_ini() {
    let f = firefox_fixture();
    let found = discover(&f.bases());
    assert_eq!(found.len(), 1);
    let ff = &found[0];
    assert_eq!((ff.name.as_str(), ff.engine), ("Firefox", Engine::Firefox));
    // Only the profile with a places.sqlite; the Install section's default.
    assert_eq!(ff.profiles.len(), 1);
    assert_eq!(ff.profiles[0].id, "abc.default-release");
    assert_eq!(ff.profiles[0].name, "default-release");
    assert_eq!(ff.usual.as_deref(), Some("abc.default-release"));
}

#[test]
fn reads_firefox_bookmarks_as_a_tree() {
    let f = firefox_fixture();
    let ff = &discover(&f.bases())[0];
    let (nodes, complete) = bookmarks(ff, None);
    assert!(complete);
    let titles: Vec<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Mozilla", "Work", "From menu", "Other bookmarks"]);
    assert_eq!(nodes[1].children.as_ref().unwrap()[0].title, "news.com");
    let other = nodes[3].children.as_ref().unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].url.as_deref(), Some("https://unvisited.com/"));
}

#[test]
fn reads_firefox_history() {
    let f = firefox_fixture();
    let ff = &discover(&f.bases())[0];
    let read = history(ff, None, HISTORY_LIMIT);
    let urls: Vec<&str> = read.iter().map(|e| e.url.as_str()).collect();
    assert_eq!(urls, ["https://mozilla.org/", "https://news.com/"]);
    assert_eq!((read[0].count, read[0].last), (4, JAN_2024_MS));
    // Firefox's open tabs aren't read.
    assert_eq!(open_tabs(ff, None), Snapshot::default());
}

#[test]
fn profiles_ini_marked_default_when_no_install_section() {
    let ini = crate::browsers::profiles_ini("[Profile0]\nName=Me\nIsRelative=0\nPath=D:\\ff\\me.x\nDefault=1\n");
    assert_eq!(ini.usual.as_deref(), Some("me.x"));
    assert_eq!(ini.profiles, [("D:\\ff\\me.x".to_string(), false, Some("Me".to_string()))]);
}

// ---- open tabs (SNSS) ----

struct Snss(Vec<u8>);

impl Snss {
    fn new() -> Snss {
        let mut v = b"SNSS".to_vec();
        v.extend(3i32.to_le_bytes());
        Snss(v)
    }
    fn frame(&mut self, id: u8, body: &[u8]) -> &mut Snss {
        self.0.extend(u16::try_from(body.len() + 1).unwrap().to_le_bytes());
        self.0.push(id);
        self.0.extend(body);
        self
    }
    fn ints(&mut self, id: u8, ints: &[i32]) -> &mut Snss {
        let body: Vec<u8> = ints.iter().flat_map(|i| i.to_le_bytes()).collect();
        self.frame(id, &body)
    }
    fn nav(&mut self, tab: i32, index: i32, url: &str, title: &str) -> &mut Snss {
        let mut b = vec![0; 4];
        b.extend(tab.to_le_bytes());
        b.extend(index.to_le_bytes());
        utf8(&mut b, url);
        utf16(&mut b, title);
        self.frame(6, &b)
    }
    fn group(&mut self, tab: i32, token: (u64, u64)) -> &mut Snss {
        let mut b = tab.to_le_bytes().to_vec();
        b.extend([0; 4]);
        b.extend(token.0.to_le_bytes());
        b.extend(token.1.to_le_bytes());
        b.extend([1, 0, 0, 0]);
        self.frame(25, &b)
    }
    fn group_meta(&mut self, token: (u64, u64), title: &str, collapsed: bool) -> &mut Snss {
        let mut b = vec![0; 4];
        b.extend(token.0.to_le_bytes());
        b.extend(token.1.to_le_bytes());
        utf16(&mut b, title);
        b.extend(2i32.to_le_bytes());
        b.extend(i32::from(collapsed).to_le_bytes());
        self.frame(27, &b)
    }
}

fn utf8(b: &mut Vec<u8>, s: &str) {
    b.extend(i32::try_from(s.len()).unwrap().to_le_bytes());
    b.extend(s.as_bytes());
    b.extend(std::iter::repeat_n(0, (4 - s.len() % 4) % 4));
}

fn utf16(b: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    b.extend(i32::try_from(units.len()).unwrap().to_le_bytes());
    b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    b.extend(std::iter::repeat_n(0, (4 - units.len() * 2 % 4) % 4));
}

fn session_bytes() -> Vec<u8> {
    let mut s = Snss::new();
    s.ints(0, &[1, 10]).ints(0, &[1, 11]).ints(0, &[1, 12]).ints(0, &[1, 13]);
    s.ints(2, &[10, 2]).ints(2, &[11, 0]).ints(2, &[12, 1]).ints(2, &[13, 3]);
    s.nav(10, 0, "https://one.com/", "One");
    s.nav(11, 0, "https://old.com/", "Old").nav(11, 1, "https://pin.com/", "Pin").ints(7, &[11, 1]);
    s.frame(12, &[11, 0, 0, 0, 1]);
    s.nav(12, 0, "https://g.com/", "  ").group(12, (7, 8)).group_meta((7, 8), "Research", true);
    s.nav(13, 0, "chrome://settings/", "Settings");
    // A tab opened then closed, and a popup window.
    s.ints(0, &[1, 14]).nav(14, 0, "https://gone.com/", "Gone").ints(16, &[14]);
    s.ints(0, &[2, 20]).ints(9, &[2, 1]).nav(20, 0, "https://popup.com/", "Popup");
    // A torn frame at the end.
    s.0.extend([50, 0, 6]);
    s.0
}

#[test]
fn reads_a_session_file() {
    let snap = session::read(&session_bytes());
    assert_eq!(snap.windows.len(), 1);
    let tabs = &snap.windows[0];
    let urls: Vec<&str> = tabs.iter().map(|t| t.url.as_str()).collect();
    assert_eq!(urls, ["https://pin.com/", "https://g.com/", "https://one.com/"]);
    assert!(tabs[0].pinned);
    assert_eq!(tabs[1].title, "g.com");
    let token = tabs[1].group.unwrap();
    assert_eq!(snap.groups[&token].title, "Research");
    assert!(snap.groups[&token].collapsed);
    assert_eq!(snap.found, 3);
    assert_eq!(snap.capped(1).tabs(), 1);
    assert!(snap.capped(1).groups.is_empty());
    assert_eq!(session::read(b"SNSX\x03\0\0\0"), Snapshot::default());
}

#[test]
fn open_tabs_takes_the_newest_session_file_and_opens_into_tabs() {
    let f = Fixture::new("session");
    f.write(&format!(r"{CHROME}\Default\History"), b"");
    f.write(&format!(r"{CHROME}\Default\Sessions\Session_1"), b"SNSS\x03\0\0\0");
    std::thread::sleep(std::time::Duration::from_millis(20));
    f.write(&format!(r"{CHROME}\Default\Sessions\Session_2"), &session_bytes());
    let chrome = &discover(&f.bases())[0];
    let mut snap = open_tabs(chrome, None);
    assert_eq!(snap.tabs(), 3);

    let mut records = Records::default();
    assert!(records.first_time("Chrome", "tab", "https://one.com/"));
    snap.retain(|t| records.first_time("Chrome", "tab", &t.url));
    assert_eq!(snap.tabs(), 2);
    assert_eq!(snap.groups.len(), 1);

    let mut tabs = searchwin_tabs::Tabs::new();
    assert_eq!(snap.open_into(&mut tabs), 2);
    let opened: Vec<(Option<&str>, bool)> = tabs.iter().map(|t| (t.url.as_deref(), t.pinned)).collect();
    assert!(opened.contains(&(Some("https://pin.com/"), true)));
    assert!(opened.contains(&(Some("https://g.com/"), false)));
}

// ---- Arc ----

const SIDEBAR: &str = r#"{"sidebar":{"containers":[{"global":{}},{
  "spaces":[
    "s1",{"id":"s1","title":"Work","customInfo":{"iconType":{"emoji_v2":"🧠"}},"profile":{"default":{}},
          "newContainerIDs":[{"pinned":{}},"p1",{"unpinned":{}},"u1"]},
    "s2",{"id":"s2","title":"Dev stuff","profile":{"custom":{"_0":{"directoryBasename":"Profile 1"}}},
          "containerIDs":["pinned","p2","unpinned","u2"]}],
  "items":[
    "p1",{"id":"p1","childrenIds":["t1","f1","t1"],"data":{"itemContainer":{}}},
    "t1",{"id":"t1","title":null,"data":{"tab":{"savedURL":"https://user:pw@a.com/x","savedTitle":"A"}}},
    "f1",{"id":"f1","title":"Folder","childrenIds":["t2","f1","t3"],"data":{"list":{}}},
    "t2",{"id":"t2","title":"Two","data":{"tab":{"savedURL":"https://two.com/"}}},
    "t3",{"id":"t3","title":"Bad","data":{"tab":{"savedURL":"file:///c:/x"}}},
    "u1",{"id":"u1","childrenIds":["t4"],"data":{}},
    "t4",{"id":"t4","title":"Today","data":{"tab":{"savedURL":"https://today.com/"}}},
    "p2",{"id":"p2","childrenIds":["t5"],"data":{}},
    "t5",{"id":"t5","title":"Five","data":{"tab":{"savedURL":"https://five.com/"}}},
    "top",{"id":"top","childrenIds":["t6"],"data":{}},
    "t6",{"id":"t6","title":"Fav","data":{"tab":{"savedURL":"https://fav.com/"}}}],
  "topAppsContainerIDs":[{"default":{}},"top"]}]}}"#;

#[test]
fn finds_arc_in_its_store_package_and_reads_its_sidebar() {
    let f = Fixture::new("arc");
    let home = r"local\Packages\TheBrowserCompany.Arc_ttt1ap7aakyb4\LocalCache\Local\Arc";
    f.write(&format!(r"{home}\User Data\Default\Bookmarks"), BOOKMARKS.as_bytes());
    f.write(&format!(r"{home}\User Data\Profile 1\History"), b"");
    f.write(&format!(r"{home}\StorableSidebar.json"), SIDEBAR.as_bytes());
    let found = discover(&f.bases());
    assert_eq!(found.len(), 1);
    let arc = &found[0];
    assert_eq!(arc.name, "Arc");
    assert!(arc.arc_sidebar.is_some());

    let all = arc::sidebar(arc, None).unwrap();
    assert_eq!(all.spaces.len(), 2);
    let work = &all.spaces[0];
    assert_eq!((work.name.as_str(), work.symbol, work.profile.as_str()), ("Work", "brain.head.profile", "Default"));
    assert_eq!(
        work.pinned,
        [
            arc::Pin::Item(arc::Item { title: "A".into(), url: "https://a.com/x".into() }),
            arc::Pin::Folder { title: "Folder".into(), items: vec![arc::Pin::Item(arc::Item { title: "Two".into(), url: "https://two.com/".into() })] },
        ]
    );
    assert_eq!(all.spaces[1].profile, "Profile 1");
    assert_eq!(all.spaces[1].symbol, "chevron.left.forwardslash.chevron.right");
    assert_eq!(all.pinned_count(), 3);
    assert_eq!(all.favorites.len(), 1);
    assert_eq!(all.favorites_by_profile["Default"][0].url, "https://fav.com/");

    let only = arc::sidebar(arc, Some("Profile 1")).unwrap();
    assert_eq!(only.spaces.len(), 1);
    assert!(only.favorites.is_empty());

    // Not Arc, or a broken file: none.
    assert!(arc::parse(b"{}", None).is_none());
}

#[test]
fn arc_symbols() {
    assert_eq!(arc::symbol(Some("☕\u{FE0F}"), None, "x"), "cup.and.saucer");
    assert_eq!(arc::symbol(None, Some("Bulb"), "x"), "lightbulb");
    assert_eq!(arc::symbol(None, None, "Ma maison"), "house");
    assert_eq!(arc::symbol(None, None, "Maison"), "house");
    assert_eq!(arc::symbol(None, None, "Daily"), "briefcase");
    assert_eq!(arc::symbol(None, None, "Researching"), "book");
}

// ---- records ----

#[test]
fn records_bring_only_what_is_new_and_survive_a_save() {
    let f = Fixture::new("records");
    let path = f.0.join("import.json");
    let mut records = Records::load(&path);
    assert!(records.of("Chrome").is_none());
    let entry = |url: &str, count| searchwin_history::Entry { url: url.into(), title: String::new(), count, last: 1 };

    let first = records.fresh_history("Chrome", vec![entry("https://a.com/", 3), entry("https://b.com/", 1)]);
    assert_eq!(first.len(), 2);
    records.note("Chrome", Added { places: first.len(), bookmarks: 2, bookmark_ids: vec!["x".into()], spaces: 1, ..Added::default() }, 42);
    records.save(&path).unwrap();

    let mut again = Records::load(&path);
    assert_eq!(again, records);
    let second = again.fresh_history("Chrome", vec![entry("https://a.com/", 5), entry("https://b.com/", 1), entry("https://c.com/", 1)]);
    let got: Vec<(&str, u32)> = second.iter().map(|e| (e.url.as_str(), e.count)).collect();
    assert_eq!(got, [("https://a.com/", 2), ("https://c.com/", 1)]);
    // Another browser's record is its own.
    assert_eq!(again.fresh_history("Edge", vec![entry("https://a.com/", 5)]).len(), 1);

    let r = again.of("Chrome").unwrap();
    assert_eq!((r.date, r.places, r.bookmarks, r.spaces, r.pinned), (42, 2, 2, Some(1), None));
    again.forget_bookmarks("Chrome");
    assert!(again.of("Chrome").unwrap().bookmark_ids.is_empty());
    assert_eq!(again.of("Chrome").unwrap().places, 2);
}

#[test]
fn password_hints_point_at_an_export() {
    let f = chrome_fixture();
    let chrome = &discover(&f.bases())[0];
    assert!(password_hint(chrome).contains("chrome://password-manager/settings"));
    let ff = firefox_fixture();
    assert!(password_hint(&discover(&ff.bases())[0]).contains("about:logins"));
}

/// Made-up Chrome, Firefox and Arc folders, written where
/// SEARCHWIN_FIXTURE_OUT says and left there, for the app's own test run
/// (SEARCHWIN_IMPORT_BASES): the app is then shown these and nothing real.
/// Run with `cargo test -p searchwin-import -- --ignored fixture_world`.
#[test]
#[ignore = "writes a fixture world where SEARCHWIN_FIXTURE_OUT says"]
fn fixture_world() {
    let Some(out) = std::env::var_os("SEARCHWIN_FIXTURE_OUT") else { return };
    let out = PathBuf::from(out);
    for (built, keep) in [(chrome_fixture(), CHROME), (firefox_fixture(), r"roaming\Mozilla\Firefox"), (arc_world(), r"local\Packages")] {
        copy_tree(&built.0.join(keep), &out.join(keep));
    }
    let session = out.join(CHROME).join(r"Default\Sessions\Session_1");
    std::fs::create_dir_all(session.parent().unwrap()).unwrap();
    std::fs::write(session, session_bytes()).unwrap();
}

fn arc_world() -> Fixture {
    let f = Fixture::new("arc-world");
    let home = r"local\Packages\TheBrowserCompany.Arc_ttt1ap7aakyb4\LocalCache\Local\Arc";
    f.write(&format!(r"{home}\User Data\Default\Bookmarks"), BOOKMARKS.as_bytes());
    f.write(&format!(r"{home}\StorableSidebar.json"), SIDEBAR.as_bytes());
    f
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
