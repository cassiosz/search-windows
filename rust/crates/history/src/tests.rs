use super::*;
use std::time::Instant;

const NOW: u64 = 1_700_000_000_000;
const DAY: u64 = 86_400_000;

fn visits(history: &mut History, url: &str, title: &str, times: u64, last: u64) {
    for n in 0..times {
        assert!(history.record_visit(url, title, last - (times - 1 - n)));
    }
}

fn keys(history: &History, typed: &str) -> Vec<String> {
    history.suggest_at(typed, 5, NOW).into_iter().map(|s| s.key).collect()
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("searchwin-history-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// MARK: recording

#[test]
fn only_web_pages_are_kept() {
    let mut h = History::new();
    assert!(!h.record_visit("file:///C:/a.txt", "a", NOW));
    assert!(!h.record_visit("about:blank", "", NOW));
    assert!(!h.record_visit("mailto:a@b.c", "", NOW));
    assert!(!h.record_visit("not a url", "", NOW));
    assert!(h.is_empty());
    assert!(h.record_visit("HTTPS://Example.com", "Example", NOW));
    assert_eq!(h.len(), 1);
}

#[test]
fn name_password_and_fragment_are_not_kept() {
    // History.swift `kept` / `identity`: a password has no place in the file.
    let mut h = History::new();
    h.record_visit("https://me:secret@example.com/page#section", "P", NOW);
    let e = &h.entries("")[0];
    assert_eq!(e.url, "https://example.com/page");
    h.record_visit("https://example.com/page#other", "", NOW + 1);
    assert_eq!(h.entries("")[0].count, 2); // the same page
}

#[test]
fn a_visit_counts_and_a_title_sticks() {
    let mut h = History::new();
    h.record_visit("https://example.com/a", "First", NOW);
    h.record_visit("https://example.com/a", "", NOW + 5);
    let e = &h.entries("")[0];
    assert_eq!((e.count, e.last, e.title.as_str()), (2, NOW + 5, "First"));
    h.record_visit("https://example.com/a", "Renamed", NOW + 6);
    assert_eq!(h.entries("")[0].title, "Renamed");
}

#[test]
fn a_deep_page_also_credits_the_front_door() {
    // History.swift `record`: so three letters offer the front page.
    let mut h = History::new();
    h.record_visit("https://example.com/deep/article?id=3", "Article", NOW);
    assert_eq!(h.len(), 2);
    assert_eq!(h.find("https://example.com/").map(|i| h.pages[i].entry.count), Some(1));
    // But a list of where you have been shows only the article.
    let shown = h.entries("");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title, "Article");
    // The front door has a title once you really go there.
    h.record_visit("https://example.com/", "Example", NOW + 1);
    assert_eq!(h.entries("").len(), 2);
}

#[test]
fn set_title_lands_after_the_page() {
    let mut h = History::new();
    h.record_visit("https://example.com/a", "", NOW);
    assert!(h.set_title("https://example.com/a#x", "Late title"));
    assert!(!h.set_title("https://example.com/a", "Late title")); // unchanged
    assert!(!h.set_title("https://example.com/a", "")); // empty never erases
    assert!(!h.set_title("https://nowhere.com/", "x"));
    assert_eq!(keys(&h, "late"), ["example.com/a"]); // and it is searchable
}

#[test]
fn remove_and_clear() {
    let mut h = History::new();
    for u in ["https://a.com/", "https://b.com/", "https://c.com/"] {
        h.record_visit(u, "t", NOW);
    }
    assert!(h.remove("https://a.com/"));
    assert!(!h.remove("https://a.com/"));
    assert_eq!(h.len(), 2);
    // The swapped-in page is still found.
    assert!(h.remove("https://c.com"));
    assert_eq!(h.entries("")[0].url, "https://b.com/");
    // a.com is no longer visited, so the app's own known names come back.
    h.clear();
    assert!(h.is_empty());
    assert!(h.entries("").is_empty());
}

// MARK: how a row reads

#[test]
fn addresses_read_as_the_field_would_have_them() {
    // Address.swift `editable`.
    let mut h = History::new();
    h.record_visit("https://www.example.com/", "", NOW);
    h.record_visit("http://insecure.example/", "", NOW);
    h.record_visit("http://localhost:3000/app", "", NOW);
    h.record_visit("https://example.org/a?b=1", "", NOW);
    let mut got: Vec<String> = h.suggest_at("e", 9, NOW).into_iter().map(|s| s.key).collect();
    got.sort();
    assert_eq!(got, ["example.org", "example.org/a?b=1", "http://insecure.example/", "www.example.com"]);
    assert_eq!(keys(&h, "localhost"), ["localhost:3000", "localhost:3000/app"]);
}

// MARK: panel

#[test]
fn the_panel_is_newest_first_and_searchable() {
    let mut h = History::new();
    h.record_visit("https://old.com/x", "Old page", NOW - 2 * DAY);
    h.record_visit("https://new.com/y", "New page", NOW);
    h.record_visit("https://mid.com/z", "Middle", NOW - DAY);
    let all: Vec<String> = h.entries("").into_iter().map(|t| t.title).collect();
    assert_eq!(all, ["New page", "Middle", "Old page"]);
    // by title or by address, any case, trimmed
    assert_eq!(h.entries("  MIDDLE ").len(), 1);
    assert_eq!(h.entries("old.com").len(), 1);
    assert!(h.entries("nothing like it").is_empty());
}

// MARK: suggestions

#[test]
fn an_empty_field_proposes_nothing() {
    let h = History::new();
    assert!(h.suggest_at("", 5, NOW).is_empty());
    assert!(h.suggest_at("   ", 5, NOW).is_empty());
    assert!(h.suggest_at("https://", 5, NOW).is_empty());
    assert!(h.suggest_at("git", 0, NOW).is_empty());
}

#[test]
fn the_first_day_offers_known_names() {
    // History.swift `known`: ranked below visits, from the start.
    let h = History::new();
    let s = h.suggest_at("git", 5, NOW);
    assert_eq!(s[0].key, "github.com");
    assert_eq!(s[0].kind, Kind::Known);
    assert_eq!(s[0].url, "https://github.com/");
    assert_eq!(s[0].title, "GitHub");
}

#[test]
fn a_known_name_goes_once_you_have_been_there() {
    let mut h = History::new();
    h.record_visit("https://github.com/rust-lang/rust", "rust", NOW);
    let s = h.suggest_at("github", 5, NOW);
    assert!(s.iter().all(|s| s.kind == Kind::Visited));
}

#[test]
fn visits_rank_above_known_names() {
    let mut h = History::new();
    h.record_visit("https://gitea.example.com/", "Gitea", NOW);
    let s = h.suggest_at("git", 5, NOW);
    assert_eq!(s[0].key, "gitea.example.com");
    assert_eq!(s[1].key, "github.com");
}

#[test]
fn fit_prefix_beats_host_word_beats_title() {
    // History.swift `fit`: 4 address starts with it, 3 a host word, 2 title.
    let mut h = History::new();
    h.record_visit("https://docs.rustacean.dev/", "", NOW); // host word
    h.record_visit("https://elsewhere.com/", "Rustacean news", NOW); // title word
    h.record_visit("https://rust-lang.org/", "", NOW); // prefix
    assert_eq!(keys(&h, "rust"), ["rust-lang.org", "docs.rustacean.dev", "elsewhere.com"]);
}

#[test]
fn title_and_path_need_three_letters() {
    // "ui" is in nearly every page.
    let mut h = History::new();
    h.record_visit("https://example.com/guide", "Guide", NOW);
    assert!(keys(&h, "ui").is_empty());
    assert_eq!(keys(&h, "gui"), ["example.com/guide"]);
}

#[test]
fn every_word_must_be_there() {
    let mut h = History::new();
    h.record_visit("https://a.com/x", "Rust async book", NOW);
    h.record_visit("https://b.com/y", "Rust ownership", NOW);
    assert_eq!(keys(&h, "rust async"), ["a.com/x"]);
    assert_eq!(keys(&h, "async rust"), ["a.com/x"]); // any order
    assert!(keys(&h, "rust python").is_empty());
}

#[test]
fn www_is_not_part_of_what_you_type() {
    // Address.withoutWWW use in History.stripped.
    let mut h = History::new();
    h.record_visit("https://www.apple.com/", "Apple", NOW);
    assert_eq!(keys(&h, "app")[0], "www.apple.com");
    assert_eq!(keys(&h, "www.app")[0], "www.apple.com");
    assert_eq!(keys(&h, "https://www.apple")[0], "www.apple.com");
}

#[test]
fn frequency_ranks_equal_matches() {
    let mut h = History::new();
    visits(&mut h, "https://gitlab.com/", "", 2, NOW);
    visits(&mut h, "https://github.com/", "", 20, NOW);
    assert_eq!(keys(&h, "git")[..2], ["github.com", "gitlab.com"]);
}

#[test]
fn recency_fades_a_habit_over_a_month() {
    // History.swift `score`: a month-old visit is about a third of a fresh one.
    let mut h = History::new();
    visits(&mut h, "https://gitlab.com/", "", 10, NOW - 90 * DAY);
    visits(&mut h, "https://github.com/", "", 4, NOW);
    assert_eq!(keys(&h, "git")[..2], ["github.com", "gitlab.com"]);
}

#[test]
fn a_tie_goes_to_the_shorter_then_the_earlier() {
    let mut h = History::new();
    h.record_visit("https://abc.example.com/", "", NOW);
    h.record_visit("https://abc.com/", "", NOW);
    assert_eq!(keys(&h, "abc"), ["abc.com", "abc.example.com"]);
}

#[test]
fn the_limit_is_kept() {
    let mut h = History::new();
    for i in 0..20 {
        h.record_visit(&format!("https://site{i}.com/"), "", NOW);
    }
    assert_eq!(h.suggest_at("site", 5, NOW).len(), 5);
    assert_eq!(h.suggest_at("site", 3, NOW).len(), 3);
}

#[test]
fn a_typo_is_forgiven_but_only_as_a_fallback() {
    // Fuzzy.swift (#762).
    let mut h = History::new();
    h.record_visit("https://stackoverflow.com/", "", NOW);
    assert_eq!(keys(&h, "stakoverflow")[0], "stackoverflow.com");
    assert_eq!(keys(&h, "gihtub")[0], "github.com"); // swapped, from the known names
    assert!(keys(&h, "ab").is_empty()); // under three letters: noise
    // Exact matches come first, the typo only fills the room.
    h.record_visit("https://githubs.example/", "", NOW);
    let s = keys(&h, "githb");
    assert_eq!(s[0], "githubs.example");
    assert!(s.contains(&"github.com".to_string()));
}

#[test]
fn no_typo_search_when_the_rows_are_full() {
    let mut h = History::new();
    for i in 0..5 {
        h.record_visit(&format!("https://github{i}.com/"), "", NOW);
    }
    let s = keys(&h, "github");
    assert_eq!(s.len(), 5);
    assert!(s.iter().all(|k| k.starts_with("github")));
}

// MARK: inline completion

#[test]
fn it_finishes_an_address_from_history() {
    let mut h = History::new();
    h.record_visit("https://github.com/", "", NOW);
    assert_eq!(h.complete_at("git", NOW).as_deref(), Some("hub.com"));
    assert_eq!(h.complete_at("GIT", NOW).as_deref(), Some("hub.com")); // typed case kept by the caller
    assert_eq!(h.complete_at("github.com", NOW), None); // nothing left
    assert_eq!(h.complete_at("zzz", NOW), None);
    assert_eq!(h.complete_at("", NOW), None);
}

#[test]
fn one_letter_finishes_only_a_front_door_you_have_been_to() {
    // History.swift `completion` (#696).
    let mut h = History::new();
    h.record_visit("https://x.com/", "", NOW);
    assert_eq!(h.complete_at("x", NOW).as_deref(), Some(".com")); // "x" becomes x.com
    let mut h = History::new();
    h.record_visit("https://example.com/", "", NOW);
    assert_eq!(h.complete_at("e", NOW).as_deref(), Some("xample.com"));
    // A room inside a site waits for a second letter.
    let mut h = History::new();
    h.record_visit("https://example.com/room", "", NOW);
    h.record_visit("https://example.com/", "", NOW); // credited anyway
    assert_eq!(h.complete_at("e", NOW).as_deref(), Some("xample.com"));
    assert_eq!(h.complete_at("example.com/r", NOW).as_deref(), Some("oom"));
    // A name the app merely knows waits for a second letter.
    let h = History::new();
    assert_eq!(h.complete_at("g", NOW), None);
    assert_eq!(h.complete_at("gi", NOW).as_deref(), Some("thub.com"));
}

#[test]
fn it_finishes_a_www_site_without_the_www() {
    // History.swift `endings`: "app" still becomes apple.com.
    let mut h = History::new();
    h.record_visit("https://www.apple.com/", "", NOW);
    assert_eq!(h.complete_at("app", NOW).as_deref(), Some("le.com"));
    assert_eq!(h.complete_at("www.ap", NOW).as_deref(), Some("ple.com"));
}

#[test]
fn a_path_keeps_its_case() {
    let mut h = History::new();
    h.record_visit("https://example.com/Docs/Intro", "", NOW);
    assert_eq!(h.complete_at("example.com/Do", NOW).as_deref(), Some("cs/Intro"));
    assert_eq!(h.complete_at("example.com/do", NOW), None);
}

#[test]
fn it_never_finishes_from_a_title() {
    let mut h = History::new();
    h.record_visit("https://example.com/", "Rustacean weekly", NOW);
    assert_eq!(h.complete_at("rustac", NOW), None);
}

// MARK: the file

#[test]
fn a_missing_or_corrupt_file_is_an_empty_history() {
    let dir = temp("corrupt");
    assert!(History::load(&dir.join("nope.json")).is_empty());
    let bad = dir.join("history.json");
    std::fs::write(&bad, b"{ this is not json").unwrap();
    assert!(History::load(&bad).is_empty());
    std::fs::write(&bad, b"").unwrap();
    assert!(History::load(&bad).is_empty());
    std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();
    assert!(History::load(&bad).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn save_and_load_round_trip() {
    let dir = temp("round");
    let file = dir.join("sub").join("history.json");
    let mut h = History::new();
    h.record_visit("https://example.com/a", "A", NOW);
    h.record_visit("https://example.com/a", "A", NOW + 1);
    h.save(&file).unwrap();
    h.save(&file).unwrap(); // over an existing file
    assert!(!file.with_extension("json.tmp").exists());
    let back = History::load(&file);
    assert_eq!(back.len(), 2);
    assert_eq!(back.entries(""), h.entries(""));
    assert_eq!(back.suggest_at("exa", 3, NOW), h.suggest_at("exa", 3, NOW));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_file_is_a_plain_json_list() {
    let dir = temp("shape");
    let file = dir.join("history.json");
    let mut h = History::new();
    h.record_visit("https://example.com/", "Example", NOW);
    h.save(&file).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text, format!(r#"[{{"url":"https://example.com/","title":"Example","count":1,"last":{NOW}}}]"#));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loading_merges_and_cleans_what_it_is_given() {
    // History.swift `read`: keys can meet; counts held to a range; a password
    // written by an older version is dropped.
    let entry = |url: &str, title: &str, count: u32, last: u64| Entry { url: url.into(), title: title.into(), count, last };
    let h = History::from_entries(vec![
        entry("https://example.com", "", 2, 10),
        entry("https://EXAMPLE.com/#frag", "Example", 3, 20),
        entry("https://u:p@secret.com/", "S", u32::MAX, 5),
        entry("ftp://old.example/", "F", 1, 1),
        entry("garbage", "", 1, 1),
    ]);
    assert_eq!(h.len(), 2);
    let shown = h.entries("");
    let example = shown.iter().find(|t| t.url == "https://example.com/").unwrap();
    assert_eq!((example.count, example.last, example.title.as_str()), (5, 20, "Example"));
    let secret = shown.iter().find(|t| t.url == "https://secret.com/").unwrap();
    assert_eq!(secret.count, MOST_VISITS);
}

#[test]
fn the_file_keeps_the_best_places_only() {
    let dir = temp("cap");
    let file = dir.join("history.json");
    let entries = (0..CAP + 100)
        .map(|i| Entry { url: format!("https://site{i}.com/"), title: String::new(), count: if i < CAP { 50 } else { 1 }, last: now_ms() })
        .collect();
    let h = History::from_entries(entries);
    assert_eq!(h.len(), CAP + 100); // loading never cuts
    h.save(&file).unwrap();
    let back = History::load(&file);
    assert_eq!(back.len(), CAP);
    assert!(back.index.contains_key("https://site0.com/"));
    assert!(!back.index.contains_key(&format!("https://site{}.com/", CAP + 99)));
    let _ = std::fs::remove_dir_all(dir);
}

// MARK: growth

#[test]
fn memory_is_cut_back_to_the_cap_keeping_the_newest() {
    // History.swift `prune`: cut once SLACK past the cap, newest SLACK kept.
    let mut h = History::new();
    // One well-visited, old place scores above any fresh single visit.
    visits(&mut h, "https://favourite.com/", "", 40, NOW - 20 * DAY);
    for i in 0..CAP + SLACK {
        h.record_visit(&format!("https://site{i}.com/"), "", NOW + i as u64);
    }
    assert_eq!(h.len(), CAP);
    assert!(h.index.contains_key("https://favourite.com/"));
    assert!(h.index.contains_key(&format!("https://site{}.com/", CAP + SLACK - 1))); // newest
    assert_eq!(h.index.len(), h.pages.len());
    assert!(h.hosts.contains_key("favourite.com"));
    // Index stays honest after the cut.
    for (url, &at) in &h.index {
        assert_eq!(&h.pages[at].entry.url, url);
    }
}

// MARK: speed

#[test]
fn suggest_stays_fast_over_20000_places() {
    let entries = (0..20_000u64)
        .map(|i| Entry {
            url: format!("https://site{i}.example{}.com/section/page-{i}", i % 97),
            title: format!("Notes about topic {i} and things"),
            count: (i % 13 + 1) as u32,
            last: NOW - (i % 60) * DAY,
        })
        .collect();
    let h = History::from_entries(entries);
    assert_eq!(h.len(), 20_000);
    // Prefix hit, word hit, a miss that falls through to the typo search
    // (the slowest path), and a short one.
    for typed in ["site1", "topic 77", "gthubx", "zzzzzzzz", "s"] {
        let start = Instant::now();
        let mut found = 0;
        for _ in 0..5 {
            found += h.suggest_at(typed, 5, NOW).len();
        }
        let each = start.elapsed() / 5;
        eprintln!("suggest({typed:?}) over 20000: {each:?} ({} rows)", found / 5);
        assert!(each.as_millis() < 50, "{typed}: {each:?}");
    }
    let start = Instant::now();
    let _ = h.complete_at("site12", NOW);
    eprintln!("complete over 20000: {:?}", start.elapsed());
    assert!(start.elapsed().as_millis() < 50);
}
