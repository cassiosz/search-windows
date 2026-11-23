use super::*;

fn sample() -> (Bookmarks, String, String, String) {
    let mut b = Bookmarks::new();
    let a = b.add("https://example.com/", "Example").unwrap();
    let f = b.add_folder(None, "Work");
    let g = b.add_folder(Some(&f), "Deep");
    b.insert(Node::site("Rust", "https://www.rust-lang.org/learn"), Some(&g), None);
    b.insert(Node::site("Docs", "https://docs.rs/"), Some(&f), None);
    (b, a, f, g)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("swb-{name}-{}", new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("bookmarks.json")
}

#[test]
fn ids_look_like_uuids_and_differ() {
    let (a, b) = (new_id(), new_id());
    assert_ne!(a, b);
    assert_eq!(a.len(), 36);
    assert_eq!(&a[14..15], "4");
}

#[test]
fn add_is_once_and_toggle_reports() {
    let mut b = Bookmarks::new();
    let id = b.add("https://a.com", "A").unwrap();
    assert!(b.contains("https://a.com/"), "same page with or without the slash");
    assert_eq!(b.add("https://a.com/", "A again"), None);
    assert_eq!(b.toggle("https://a.com", "A"), Toggled::Already(id));
    assert!(matches!(b.toggle("https://b.com", ""), Toggled::Added(_)));
    assert_eq!(b.get(&b.find_url("https://b.com").unwrap().id.clone()).unwrap().title, "b.com");
}

#[test]
fn empty_title_is_the_address() {
    assert_eq!(Node::site("  ", "https://www.example.com/x/").title, "example.com/x");
}

#[test]
fn urls_in_opens_nested_folders_in_order() {
    let (b, a, f, _) = sample();
    assert_eq!(b.urls_in(&f), vec!["https://www.rust-lang.org/learn", "https://docs.rs/"]);
    assert_eq!(b.urls_in(&a), vec!["https://example.com/"]);
    assert!(b.urls_in("nope").is_empty());
    assert_eq!(b.count(), 3);
}

#[test]
fn bookmarklets_are_not_opened_or_suggested() {
    let mut b = Bookmarks::new();
    let f = b.add_folder(None, "F");
    b.insert(Node::site("Tool", "javascript:alert(1)"), Some(&f), None);
    assert!(b.urls_in(&f).is_empty());
    assert!(b.places("tool", 5).is_empty());
    assert_eq!(b.search("tool").len(), 1);
}

#[test]
fn remove_takes_a_folder_with_everything_in_it() {
    let (mut b, _, f, g) = sample();
    assert!(b.remove(&f));
    assert!(b.get(&g).is_none());
    assert!(!b.contains("https://docs.rs/"));
    assert!(!b.remove(&f));
}

#[test]
fn rename_and_update() {
    let (mut b, a, f, _) = sample();
    assert!(b.rename(&a, "Ex"));
    assert_eq!(b.get(&a).unwrap().title, "Ex");
    assert!(b.rename(&a, ""));
    assert!(b.get(&a).unwrap().unnamed());
    assert_eq!(b.get(&a).unwrap().label(), "example.com");
    assert!(!b.rename(&f, "  "), "a folder keeps a name");
    assert!(b.update(&a, None, Some("https://new.example/")));
    assert!(b.contains("https://new.example/") && !b.contains("https://example.com/"));
    assert!(b.update(&f, None, Some("https://x.y/")));
    assert!(b.get(&f).unwrap().is_folder(), "a folder gets no address");
    assert!(!b.rename("missing", "x"));
}

#[test]
fn move_into_folder_and_back_to_top() {
    let (mut b, a, f, g) = sample();
    assert!(b.move_to(&a, Some(&g), Some(0)));
    assert_eq!(b.parent_of(&a), Some(g.clone()));
    assert_eq!(b.path_to(&a).unwrap(), vec![f.clone(), g.clone()]);
    assert!(b.move_to(&a, None, Some(0)));
    assert_eq!(b.roots()[0].id, a);
    assert_eq!(b.parent_of(&a), None);
    assert_eq!(b.path_to("missing"), None);
}

#[test]
fn a_folder_cannot_go_into_itself_or_its_children() {
    let (mut b, _, f, g) = sample();
    assert!(!b.move_to(&f, Some(&f), None));
    assert!(!b.move_to(&f, Some(&g), None));
    assert!(b.get(&g).is_some());
}

#[test]
fn move_index_counts_the_list_before_the_move() {
    let mut b = Bookmarks::new();
    let ids: Vec<String> = (0..3).map(|i| b.add(&format!("https://s{i}.com"), "").unwrap()).collect();
    // dropping the first row just below itself leaves it where it was
    assert!(b.move_to(&ids[0], None, Some(1)));
    assert_eq!(b.roots().iter().map(|n| n.id.clone()).collect::<Vec<_>>(), ids);
    // below the second: becomes second
    assert!(b.move_to(&ids[0], None, Some(2)));
    assert_eq!(b.roots()[1].id, ids[0]);
    // unknown folder: refused, nothing lost
    assert!(!b.move_to(&ids[0], Some("nope"), None));
    assert_eq!(b.roots().len(), 3);
}

#[test]
fn search_needs_every_word_in_title_or_address() {
    let (b, _, f, g) = sample();
    let hits = b.search("rust learn");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, vec!["Work", "Deep"]);
    assert_eq!(b.search("WORK").first().map(|h| h.id.clone()), Some(f));
    assert!(b.search("deep").iter().any(|h| h.id == g));
    assert!(b.search("   ").is_empty());
    assert!(b.search("rust zzz").is_empty());
}

#[test]
fn places_rank_address_then_name_then_anywhere() {
    let mut b = Bookmarks::new();
    b.add("https://news.example.org/a/long/path", "Headlines");
    b.add("https://example.com/", "Zed");
    b.add("https://other.net/", "An example of things");
    b.add("https://x.net/page", "Other example");
    let urls: Vec<String> = b.places("example", 10).into_iter().map(|p| p.url).collect();
    assert_eq!(
        urls,
        ["https://example.com/", "https://other.net/", "https://x.net/page", "https://news.example.org/a/long/path"]
            .map(String::from)
    );
    assert_eq!(b.places("example", 2).len(), 2);
    assert!(b.places("e", 5).is_empty(), "one letter is in everything");
    assert_eq!(b.places("zed example", 5).len(), 1, "words in any order, across name and address");
}

#[test]
fn file_round_trip_in_upstream_shape() {
    let (b, ..) = sample();
    let path = tmp("rt");
    b.save(&path).unwrap();
    assert!(!path.with_extension("json.tmp").exists());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"children\"") && text.contains("\"title\":\"Example\""));
    assert!(!text.contains("null"), "absent url/children are omitted as upstream's encoder does");
    let again = Bookmarks::load(&path);
    assert_eq!(again.roots(), b.roots());
    assert!(again.contains("https://docs.rs/"));
}

#[test]
fn loads_a_file_the_mac_app_wrote() {
    let path = tmp("mac");
    std::fs::write(
        &path,
        r#"[{"id":"4B8F6A52-6C1E-4F0B-9E5C-0D7A1C2B3E4F","title":"Site","url":"https://a.com/"},
            {"id":"11111111-2222-3333-4444-555555555555","title":"Dir","children":[
              {"id":"4B8F6A52-6C1E-4F0B-9E5C-0D7A1C2B3E4F","title":"Dup id","url":"https://b.com/"}]}]"#,
    )
    .unwrap();
    let b = Bookmarks::load(&path);
    assert_eq!(b.count(), 2);
    let ids: HashSet<_> = [&b.roots()[0].id, &b.roots()[1].children.as_ref().unwrap()[0].id].into_iter().collect();
    assert_eq!(ids.len(), 2, "a repeated id is made unique");
}

#[test]
fn missing_or_corrupt_file_is_empty_and_corrupt_is_kept_aside() {
    let path = tmp("bad");
    assert!(Bookmarks::load(&path).is_empty());
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(Bookmarks::load(&path).is_empty());
    assert!(!path.exists());
    let bad = path.with_extension("json.bad");
    assert_eq!(std::fs::read(bad).unwrap(), b"{ not json");
}

#[test]
fn deep_nesting_in_a_file_is_cut() {
    let mut node = Node::site("leaf", "https://leaf.com/");
    for _ in 0..200 {
        node = Node::folder("d", vec![node]);
    }
    let b = Bookmarks::from_roots(vec![node]);
    assert!(b.folders().len() <= MAX_DEPTH);
}

#[test]
fn take_adds_only_what_is_new() {
    let incoming = || vec![Node::site("A", "https://a.com/"), Node::folder("F", vec![Node::site("B", "https://b.com/")])];
    let mut b = Bookmarks::new();
    assert_eq!(b.take(incoming(), "Chrome"), (2, 0));
    assert_eq!(b.roots().len(), 2, "into an empty list: at the top");
    assert_eq!(b.take(incoming(), "Chrome"), (0, 2));
    let mut b = Bookmarks::new();
    b.add("https://mine.com/", "Mine");
    assert_eq!(b.take(incoming(), "Chrome"), (2, 0));
    assert_eq!(b.roots().len(), 2);
    assert_eq!(b.roots()[1].title, "Chrome");
    let before = b.count();
    assert_eq!(b.take(incoming(), "Chrome"), (0, 2));
    assert_eq!(b.count(), before);
}

const CHROME: &str = r#"{
  "checksum": "x", "version": 1,
  "roots": {
    "bookmark_bar": {"type":"folder","name":"Bookmarks bar","children":[
      {"type":"url","name":"Rust","url":"https://www.rust-lang.org/"},
      {"type":"url","name":"","url":"https://nameless.example/p"},
      {"type":"url","name":"Chrome page","url":"chrome://settings"},
      {"type":"url","name":"Mail","url":"mailto:a@b.c"},
      {"type":"folder","name":"Dev","children":[
         {"type":"url","name":"Docs","url":"https://docs.rs/"},
         {"type":"folder","name":"Empty","children":[]}]}
    ]},
    "other": {"type":"folder","name":"Other bookmarks","children":[
      {"type":"url","name":"Wiki","url":"https://en.wikipedia.org/"}]},
    "synced": {"type":"folder","name":"Mobile bookmarks","children":[]}
  }
}"#;

#[test]
fn imports_the_chrome_format() {
    let nodes = from_chrome(CHROME.as_bytes()).unwrap();
    let titles: Vec<&str> = nodes.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Rust", "nameless.example", "Dev", "Other bookmarks"], "bar to the top; empty synced left out");
    assert_eq!(nodes[2].children.as_ref().unwrap().len(), 2, "empty folders are kept as upstream's import does");
    let b = Bookmarks::from_roots(nodes);
    assert_eq!(b.count(), 4, "only http and https came in");
    assert!(b.contains("https://www.rust-lang.org/") && !b.contains("chrome://settings"));
}

#[test]
fn not_a_chrome_file_is_none() {
    assert!(from_chrome(b"nope").is_none());
    assert!(from_chrome(b"{\"a\":1}").is_none());
    assert_eq!(from_chrome(b"{\"roots\":{}}").unwrap().len(), 0);
}

#[test]
fn the_cards_new_folder_files_the_site_in_it() {
    let (mut b, a, _, _) = sample();
    let made = b.file_in_new_folder(&a, "Later").unwrap();
    assert_eq!(b.parent_of(&a), Some(made.clone()));
    assert_eq!(b.get(&made).unwrap().title, "Later");
    assert_eq!(b.roots().last().unwrap().id, made, "at the top level, at the end");
    let unnamed = b.file_in_new_folder(&a, "  ").unwrap();
    assert_eq!(b.get(&unnamed).unwrap().title, "Folder");
    let folders = b.folders().len();
    assert_eq!(b.file_in_new_folder("missing", "X"), None);
    assert_eq!(b.folders().len(), folders, "no folder left behind for a site that isn't here");
}
